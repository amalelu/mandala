//! Application-bundled mutations, shipped inside the binary via
//! `include_str!`. This is the lowest-precedence source: the user's
//! file overrides app mutations by id, the map overrides user, and
//! per-node inline mutations override everything.
//!
//! The bundle lives at `assets/mutations/application.json` in the
//! repo root (sibling to `maps/`). A malformed bundle is a build-time
//! invariant failure, so parsing uses `expect()` per CODE_CONVENTIONS
//! §7's startup-path rule — a broken bundle needs a fix in the
//! source file, not a degraded runtime.

use baumhard::mindmap::custom_mutation::CustomMutation;

/// Raw JSON of the application mutation bundle. `include_str!` so the
/// bytes ship in the binary (works identically on native and WASM —
/// no `fs::read` on the browser side).
const APP_MUTATIONS_JSON: &str =
    include_str!("../../../../assets/mutations/application.json");

/// Parse the bundled mutations once per process. A failure here means
/// the repo's JSON is broken and merits a build-time fix.
pub fn load_app() -> Vec<CustomMutation> {
    super::parse_mutations_json(APP_MUTATIONS_JSON)
        .expect("application.json must parse at startup")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled JSON must parse successfully — if it doesn't, the
    /// binary would panic at startup, and that's exactly the contract
    /// this test defends.
    #[test]
    fn bundled_application_json_parses() {
        let _ = load_app();
    }

    /// Every bundled mutation should declare at least one context so
    /// `mutation list` doesn't silently hide it. Internal-by-default
    /// semantics apply only to programmatically-registered mutations.
    #[test]
    fn bundled_mutations_declare_contexts() {
        for cm in load_app() {
            assert!(
                !cm.contexts.is_empty(),
                "bundled mutation '{}' has empty contexts — \
                 add 'map.node' / 'map.tree' / 'internal' as appropriate",
                cm.id
            );
        }
    }

    /// Bundled mutations should have a non-empty description — it's
    /// the first thing the console shows in `mutation list`.
    #[test]
    fn bundled_mutations_have_descriptions() {
        for cm in load_app() {
            assert!(
                !cm.description.is_empty(),
                "bundled mutation '{}' has no description",
                cm.id
            );
        }
    }
}
