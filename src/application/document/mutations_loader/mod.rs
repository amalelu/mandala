// SPDX-License-Identifier: MPL-2.0

//! Four-source mutation loader: application bundle, user file, map,
//! inline-on-node — in ascending precedence.
//!
//! **Source-of-truth note:** the precedence order (App < User < Map
//! < Inline) is [`crate::application::source_tier::SourceTier`],
//! shared with the macro registry and pinned by that module's tests.
//! `format/mutations.md` documents the same ladder under "Where
//! mutations come from" for authors;
//! [`crate::application::document::MindMapDocument::build_mutation_registry_with_app_and_user`]
//! walks it.
//!
//! This module owns the outer loader (app + user slices produced in
//! `load_app_and_user`), the app-bundle parser ([`builtin`]), and the
//! platform-split user-file readers (`platform_desktop` /
//! `platform_web`). The two readers are cfg-gated to their target
//! and cargo doc would warn under "unresolved link" for the inactive
//! one if they were intra-doc links.
//!
//! Resilience posture (§7): every layer is best-effort. Failures log
//! `warn!` and fall through to the next source; the app never crashes
//! on a bad user file. The application bundle, by contrast, is a
//! build-time invariant and is parsed with `expect()` — a malformed
//! bundle is a startup-time bug, not a user input error.

pub mod builtin;

#[cfg(not(target_arch = "wasm32"))]
pub mod platform_desktop;
#[cfg(target_arch = "wasm32")]
pub mod platform_web;

use baumhard::mindmap::custom_mutation::CustomMutation;

/// Load the two slices the registry builder expects — application
/// mutations (bundled with the binary) and user mutations (from the
/// local config file on native; from query/localStorage on WASM).
///
/// Map and inline mutations are read from the `MindMapDocument` itself
/// at merge time, not here.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_app_and_user(
    explicit_user_path: Option<&std::path::Path>,
) -> (Vec<CustomMutation>, Vec<CustomMutation>) {
    let app = builtin::load_app();
    let user = platform_desktop::load_user(explicit_user_path);
    (app, user)
}

/// WASM companion — reads the user slice from the `?mutations=` query
/// param or `localStorage` instead of the filesystem.
#[cfg(target_arch = "wasm32")]
pub fn load_app_and_user() -> (Vec<CustomMutation>, Vec<CustomMutation>) {
    let app = builtin::load_app();
    let user = platform_web::load_user();
    (app, user)
}

/// Parse a JSON string carrying a top-level `{"mutations": [...]}`
/// shape. Extracted so the app bundle, user file, and test fixtures
/// share one parser.
pub fn parse_mutations_json(source: &str) -> Result<Vec<CustomMutation>, String> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        #[serde(default)]
        mutations: Vec<CustomMutation>,
    }
    baumhard::format::json::parse::<Envelope>(source).map(|e| e.mutations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_envelope_yields_empty_vec() {
        let v = parse_mutations_json(r#"{"mutations": []}"#).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn parse_missing_mutations_key_treated_as_empty() {
        let v = parse_mutations_json(r#"{}"#).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn parse_malformed_json_reports_error() {
        let err = parse_mutations_json("{ not json").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn parse_single_mutation_with_new_shape() {
        let src = r#"{
            "mutations": [{
                "id": "hi",
                "name": "Hi",
                "mutator": {"Macro": {"channel": 0, "mutations": {"Literal": []}}},
                "target_scope": "SelfOnly"
            }]
        }"#;
        let v = parse_mutations_json(src).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "hi");
    }
}
