// SPDX-License-Identifier: MPL-2.0

//! Macro loaders for the four-tier registry.
//!
//! The format reference is in `format/macros.md`. Each tier has a
//! pinned `SourceTier`; the registry picks the highest-tier macro
//! by id when collisions happen. All four tiers ship on both native
//! and WASM (Track B in `WASM_CONVERGENCE.md`):
//! - **App** — `assets/macros/application.json`, embedded with
//!   `include_str!`. Cross-platform.
//! - **User** — `~/.config/mandala/macros.json` on native;
//!   `?macros=<urlencoded-json>` query param > `localStorage`
//!   under `mandala_macros` on WASM. Routed via the
//!   `platform_desktop` / `platform_web` sibling modules. Both
//!   are cfg-gated to their target so intra-doc links would
//!   trip "unresolved link" warnings on the inactive one.
//! - **Map** — declared in the currently-loaded `.mindmap.json`'s
//!   `mindmap.macros` array; refreshed on every document load.
//!   Cross-platform.
//! - **Inline** — declared on a specific node's `inline_macros`
//!   array; refreshed alongside Map. Cross-platform.
//!
//! Resilience: app-bundle parses with `expect()` (a malformed bundle
//! is a startup-time bug, not a user input error). Other tiers'
//! parse failures log `warn!` and fall through to an empty slice
//! / skipped entry so the application boots even if user input is
//! broken.

#[cfg(not(target_arch = "wasm32"))]
pub mod platform_desktop;
#[cfg(target_arch = "wasm32")]
pub mod platform_web;

#[cfg(not(target_arch = "wasm32"))]
pub use platform_desktop::load_user_macros;
#[cfg(target_arch = "wasm32")]
pub use platform_web::load_user_macros;

use super::Macro;
use crate::application::source_tier::SourceTier;

/// Application-bundle JSON, embedded at compile time. Parsed by
/// [`load_app_macros`]. Empty array today; future shipped macros
/// land here.
const APP_MACROS_JSON: &str = include_str!("../../../../assets/macros/application.json");

/// Load the application-bundle macros. Tier: `SourceTier::App`,
/// assigned at the call site in `run_native_init::build` and the
/// WASM init block.
///
/// Parses with `expect()` — a malformed bundle is a build-time bug.
/// `format/macros.md` documents the format; the file MUST be a
/// top-level JSON array of macro objects.
pub fn load_app_macros() -> Vec<Macro> {
    let trimmed = APP_MACROS_JSON.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    baumhard::format::json::parse::<Vec<Macro>>(trimmed)
        .expect("malformed assets/macros/application.json — bundle is invalid")
}

/// Parse a free-form JSON `Vec<Macro>` payload. Used by both
/// targets' user-tier loaders (the native `platform_desktop` after
/// `read_to_string`; the WASM `platform_web` after the query /
/// localStorage fetch). Returns `Err(String)` so the platform
/// layer can `warn!` consistently.
pub fn parse_user_macros_json(source: &str) -> Result<Vec<Macro>, String> {
    if source.trim().is_empty() {
        return Ok(Vec::new());
    }
    baumhard::format::json::parse::<Vec<Macro>>(source)
        .map_err(|e| format!("malformed user macros JSON: {}", e))
}

/// Parse Map-tier macros out of a loaded document's
/// `mindmap.macros: Vec<serde_json::Value>`. Per-entry parse
/// failures log a `warn!` and are skipped — a malformed Map-tier
/// macro doesn't break document loading. Tier assignment
/// (`SourceTier::Map`) happens at the call site in the document-
/// load path.
///
/// Map-tier macros are stored as untyped JSON in baumhard because
/// the typed `Macro` lives in the application crate (its `Action`
/// enum would otherwise force a circular dependency). The JSON
/// shape matches the User / App tiers — see `format/macros.md`.
pub fn parse_map_macros(values: &[baumhard::format::json::Value]) -> Vec<Macro> {
    let mut out = Vec::with_capacity(values.len());
    for (idx, v) in values.iter().enumerate() {
        match baumhard::format::json::parse_value::<Macro>(v.clone()) {
            Ok(m) => out.push(m),
            Err(e) => {
                // Surface the entry's `id` field in the warning when
                // available — the user / map author addresses macros
                // by id, so "entry [3] (id=save-and-quit)" is a much
                // better diagnostic than "entry [3]" alone. The id
                // may be missing entirely on a malformed entry; fall
                // back to a placeholder.
                let id_hint = v.get("id").and_then(|s| s.as_str()).unwrap_or("<no id>");
                log::warn!(
                    "macros: Map-tier entry [{}] (id={}) failed to parse: {} (skipping)",
                    idx,
                    id_hint,
                    e
                );
            }
        }
    }
    out
}

/// Refresh the registry's `Map` tier from a loaded document. App
/// and User tiers are untouched — those load once at startup and
/// don't depend on which document is open. Called from:
///
/// 1. `run_native_init::build` after the initial document load.
/// 2. `execute_console_line` when `replace_document` swaps the
///    document via `open` / `new`.
pub fn rebuild_map_macros(
    registry: &mut super::MacroRegistry,
    doc: &crate::application::document::MindMapDocument,
) {
    registry.clear_tier(SourceTier::Map);
    let map_macros = parse_map_macros(&doc.mindmap.macros);
    if !map_macros.is_empty() {
        log::info!("macros: loaded {} Map-tier macro(s)", map_macros.len());
    }
    registry.extend_with_tier(map_macros, SourceTier::Map);
}

/// Walk every node's `inline_macros` and parse them into typed
/// `Macro`s. Per-entry parse failures log `warn!` and skip — same
/// resilience posture as `parse_map_macros`. Tier assignment
/// (`SourceTier::Inline`) happens at the call site.
///
/// Inline macros are scoped to the node they live on, but the
/// registry is flat (id-keyed). Authors should namespace inline
/// macro ids to avoid collisions across nodes — `format/macros.md`
/// covers this and recommends `node-id.action` patterns.
pub fn parse_inline_macros(doc: &crate::application::document::MindMapDocument) -> Vec<super::Macro> {
    let mut out = Vec::new();
    // Iterate nodes in lexicographic id order so cross-node id
    // collisions resolve deterministically: the lowest node id
    // wins. Without the sort, `MindMap.nodes` is a HashMap and
    // the walk order changes per process start, making the
    // "winner" of a duplicated id vary between runs. The warn
    // below still fires — authors should namespace inline ids
    // (e.g. `<node-id>.action`) — but the runtime behaviour is
    // now reproducible.
    let mut node_ids: Vec<&String> = doc.mindmap.nodes.keys().collect();
    node_ids.sort();
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for node_id in node_ids {
        let node = match doc.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => continue,
        };
        for (idx, v) in node.inline_macros.iter().enumerate() {
            match baumhard::format::json::parse_value::<super::Macro>(v.clone()) {
                Ok(m) => {
                    if let Some(prev_node) = seen.insert(m.id.clone(), node_id.clone()) {
                        if prev_node != *node_id {
                            log::warn!(
                                "macros: Inline-tier id '{}' duplicated across nodes \
                                 '{}' and '{}'; the lower-id node ('{}') wins. \
                                 Namespace your ids (e.g. '<node-id>.{}').",
                                m.id,
                                prev_node,
                                node_id,
                                prev_node,
                                m.id
                            );
                        }
                    }
                    out.push(m);
                }
                Err(e) => {
                    let id_hint = v.get("id").and_then(|s| s.as_str()).unwrap_or("<no id>");
                    log::warn!(
                        "macros: Inline-tier entry on node '{}' [{}] (id={}) failed to parse: {} (skipping)",
                        node_id,
                        idx,
                        id_hint,
                        e
                    );
                }
            }
        }
    }
    out
}

/// Refresh the registry's `Inline` tier from every node's
/// `inline_macros` field. Called from the same two sites as
/// `rebuild_map_macros` so the two tiers stay coherent across
/// document loads. Inline tier is the highest precedence — it
/// overrides Map, User, and App on id collisions.
pub fn rebuild_inline_macros(
    registry: &mut super::MacroRegistry,
    doc: &crate::application::document::MindMapDocument,
) {
    registry.clear_tier(SourceTier::Inline);
    let inline_macros = parse_inline_macros(doc);
    if !inline_macros.is_empty() {
        log::info!("macros: loaded {} Inline-tier macro(s)", inline_macros.len());
    }
    registry.extend_with_tier(inline_macros, SourceTier::Inline);
}

/// Refresh both document-derived tiers (Map and Inline) in the
/// correct order. Map is rebuilt first so Inline's higher
/// precedence wins on id collision via the registry's
/// last-writer-wins insert semantics.
///
/// Single entry point for callers that load / replace a
/// document so the two-call ordering can't drift between sites.
/// Used at startup in `run_native_init::build` and at every
/// `open` / `new` console verb in
/// `console_input::exec::execute_console_line`.
pub fn rebuild_document_macros(
    registry: &mut super::MacroRegistry,
    doc: &crate::application::document::MindMapDocument,
) {
    rebuild_map_macros(registry, doc);
    rebuild_inline_macros(registry, doc);
}

/// Build the load-time macro registry: App and User tiers from the
/// platform loaders, then the document-derived Map and Inline tiers.
///
/// This is the whole of what both runtimes do at startup, and both
/// used to do it in their own copy — same loop, same counters, same
/// `log::info!` format string. The logging shape is load-bearing:
/// cross-target log triage compares browser-console output against
/// desktop stderr, so a divergence in the message is a divergence in
/// the tooling.
///
/// Tier precedence is App < User < Map < Inline, enforced by insert
/// order plus the registry's last-writer-wins semantics. Higher-tier
/// ids shadow lower-tier ones; clearing a higher tier reveals what
/// is underneath. `format/macros.md` carries the threat model and the
/// SOURCE-OF-TRUTH list of places that must move together when the
/// order changes — this function is now one place rather than two.
///
/// `document` is `None` when the caller has no document at all; the
/// two document-derived tiers are then simply absent.
///
/// **No production caller passes `None` any more.** Startup always
/// produces a document — a real map, or the load-failure placard from
/// `app::startup_load` (a private module, so this is a code span
/// rather than a link) — so both init sites pass `Some`. The
/// parameter is kept `Option`
/// because the console's document-replace path may yet need the
/// document-less shape, and because collapsing it belongs with the
/// wider `InitState.document: Option<_>` cleanup rather than here.
/// Until then the `None` arm is exercised only by
/// `test_build_macro_registry_includes_map_tier_when_a_document_is_given`,
/// which needs it as the negative control proving the Map tier is
/// what supplied the id.
pub fn build_macro_registry(
    document: Option<&crate::application::document::MindMapDocument>,
) -> super::MacroRegistry {
    let mut macros = super::MacroRegistry::new();
    let mut app_count = 0usize;
    for m in load_app_macros() {
        macros.insert(m, SourceTier::App);
        app_count += 1;
    }
    let mut user_count = 0usize;
    for m in load_user_macros() {
        macros.insert(m, SourceTier::User);
        user_count += 1;
    }
    if app_count > 0 || user_count > 0 {
        log::info!(
            "loaded {} macro(s): {} app-tier, {} user-tier",
            macros.len(),
            app_count,
            user_count
        );
    }
    if let Some(d) = document {
        rebuild_document_macros(&mut macros, d);
    }
    macros
}

// `load_user_macros` lives in the cfg-routed sibling modules
// `platform_desktop` and `platform_web`; the platform-routed
// `pub use` at the top of this file picks the right one for the
// current target. Both expose the same `pub fn load_user_macros()
// -> Vec<Macro>` signature. Written as a plain comment because a
// `///` here would attach itself to the test module below.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The bundled `assets/macros/application.json` parses cleanly.
    /// Catches malformed-asset regressions at test time rather than
    /// startup `expect()` panics.
    #[test]
    fn app_bundle_parses() {
        let _ = load_app_macros();
    }

    /// `build_macro_registry` folds the document-derived tiers in
    /// when a document is present. Both runtimes' init calls this;
    /// before the extraction each had its own copy of the loop, so a
    /// tier added on one target could silently miss the other.
    ///
    /// Not hermetic in the App / User tiers — those read the bundled
    /// asset and the user's config dir — so the assertions are about
    /// the *document* tiers and about `None` versus `Some` being
    /// distinguishable, which is the part the extraction could have
    /// broken.
    #[test]
    fn test_build_macro_registry_includes_map_tier_when_a_document_is_given() {
        use crate::application::document::tests_common::load_test_doc;
        use crate::application::source_tier::SourceTier;

        let mut doc = load_test_doc();
        doc.mindmap.macros = vec![json!({
            "id": "from-the-map",
            "steps": [{"kind": "Action", "action": "Undo"}]
        })];

        let with_doc = build_macro_registry(Some(&doc));
        assert_eq!(
            with_doc.get_with_source("from-the-map").map(|(_, t)| t),
            Some(SourceTier::Map),
            "a map-tier macro must be registered at the Map tier",
        );

        // The same call with no document must not carry it. This is
        // the positive control for the assertion above: without it,
        // a registry that ignored its argument entirely would still
        // pass the first check on a machine whose config dir happens
        // to define the id.
        let without_doc = build_macro_registry(None);
        assert!(
            without_doc.get("from-the-map").is_none(),
            "document tiers must be absent when no document is passed",
        );
    }

    /// Inline tier outranks Map on an id collision — the precedence
    /// `format/macros.md` names as SOURCE-OF-TRUTH-critical, now
    /// enforced in the single body both targets run.
    #[test]
    fn test_build_macro_registry_lets_inline_tier_shadow_map_tier() {
        use crate::application::document::tests_common::load_test_doc;
        use crate::application::source_tier::SourceTier;

        let mut doc = load_test_doc();
        doc.mindmap.macros = vec![json!({
            "id": "collides",
            "steps": [{"kind": "Action", "action": "Undo"}]
        })];
        let mut node_ids: Vec<String> = doc.mindmap.nodes.keys().cloned().collect();
        node_ids.sort();
        doc.mindmap
            .nodes
            .get_mut(&node_ids[0])
            .expect("test doc has at least one node")
            .inline_macros = vec![json!({
            "id": "collides",
            "steps": [{"kind": "Action", "action": "SelectAll"}]
        })];

        let reg = build_macro_registry(Some(&doc));
        assert_eq!(
            reg.get_with_source("collides").map(|(_, t)| t),
            Some(SourceTier::Inline),
            "Inline must shadow Map on an id collision",
        );
    }

    /// `parse_map_macros` is best-effort: malformed entries log
    /// `warn!` and skip without breaking the rest of the parse.
    /// Locks the resilience contract documented in the rustdoc.
    #[test]
    fn parse_map_macros_skips_malformed_entries() {
        let values = vec![
            json!({
                "id": "valid",
                "steps": [{"kind": "Action", "action": "Undo"}]
            }),
            json!({
                "id": "missing-steps"
                // missing required `steps` field
            }),
            json!({
                "id": "valid-2",
                "steps": [{"kind": "ConsoleLine", "line": "save"}]
            }),
        ];
        let parsed = parse_map_macros(&values);
        assert_eq!(parsed.len(), 2, "malformed middle entry should be skipped");
        assert_eq!(parsed[0].id, "valid");
        assert_eq!(parsed[1].id, "valid-2");
    }

    #[test]
    fn parse_map_macros_empty_input_returns_empty() {
        let parsed = parse_map_macros(&[]);
        assert!(parsed.is_empty());
    }

    /// `parse_inline_macros` walks every node's `inline_macros`
    /// and returns a flat list. Per-entry parse failures log
    /// `warn!` and skip without breaking the rest of the parse —
    /// same resilience contract as `parse_map_macros`.
    #[test]
    fn parse_inline_macros_walks_all_nodes_and_skips_malformed() {
        use crate::application::document::tests_common::load_test_doc;
        let mut doc = load_test_doc();

        // Pick the first two nodes and stuff inline_macros onto
        // them — one valid, one malformed, one valid.
        let mut node_ids: Vec<String> = doc.mindmap.nodes.keys().cloned().collect();
        node_ids.sort(); // deterministic ordering for the test
        let n0 = node_ids[0].clone();
        let n1 = node_ids[1].clone();

        if let Some(node) = doc.mindmap.nodes.get_mut(&n0) {
            node.inline_macros = vec![
                json!({
                    "id": "node0-action",
                    "steps": [{"kind": "Action", "action": "Undo"}]
                }),
                json!({
                    "id": "node0-malformed"
                    // missing required `steps`
                }),
            ];
        }
        if let Some(node) = doc.mindmap.nodes.get_mut(&n1) {
            node.inline_macros = vec![json!({
                "id": "node1-action",
                "steps": [{"kind": "Action", "action": "ZoomReset"}]
            })];
        }

        let parsed = parse_inline_macros(&doc);
        // Two valid entries (one from each node), one malformed
        // skipped.
        assert_eq!(parsed.len(), 2);
        let ids: std::collections::HashSet<&str> = parsed.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains("node0-action"));
        assert!(ids.contains("node1-action"));
        assert!(!ids.contains("node0-malformed"));
    }

    #[test]
    fn parse_inline_macros_empty_when_no_node_has_macros() {
        use crate::application::document::tests_common::load_test_doc;
        let doc = load_test_doc();
        let parsed = parse_inline_macros(&doc);
        assert!(parsed.is_empty());
    }

    /// `parse_user_macros_json` is the cross-platform parsing seam
    /// the WASM `platform_web::load_user_macros` and the native
    /// `platform_desktop::load_user_macros` both call. Pin its
    /// contract here so the WASM path (which has no headless test
    /// harness — see `TEST_CONVENTIONS.md §T9`) is at least
    /// indirectly covered.
    #[test]
    fn parse_user_macros_json_empty_input_returns_empty() {
        assert!(parse_user_macros_json("").unwrap().is_empty());
        assert!(parse_user_macros_json("   ").unwrap().is_empty());
    }

    #[test]
    fn parse_user_macros_json_array_round_trips() {
        let source = r#"[
            {"id": "u1", "steps": [{"kind": "Action", "action": "Undo"}]},
            {"id": "u2", "steps": [{"kind": "ConsoleLine", "line": "save"}]}
        ]"#;
        let parsed = parse_user_macros_json(source).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "u1");
        assert_eq!(parsed[1].id, "u2");
    }

    #[test]
    fn parse_user_macros_json_malformed_returns_err_not_panic() {
        // User-tier loader on both targets logs the err and falls
        // back to empty rather than panicking; pin the err-not-panic
        // contract here.
        let result = parse_user_macros_json("definitely { not } json");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("malformed user macros JSON"),
            "expected canonical err prefix, got: {}",
            msg
        );
    }
}
