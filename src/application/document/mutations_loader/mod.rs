//! Four-source mutation loader: application bundle, user file, map,
//! inline-on-node — in ascending precedence.
//!
//! This module owns the outer loader (app + user slices produced in
//! `load_app_and_user`), the app-bundle parser ([`builtin`]), and the
//! platform-split user-file readers ([`platform_desktop`] /
//! [`platform_web`]). The merged registry — where map and inline
//! mutations join and later keys override earlier ones — is built by
//! [`crate::application::document::MindMapDocument::build_mutation_registry_with_app_and_user`].
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

/// Source layer a registered mutation came from. The
/// `build_mutation_registry_with_app_and_user` method stamps a
/// `MutationSource` into `MindMapDocument::mutation_sources`
/// alongside every registry write, so `mutation help <id>` can
/// report which layer won the id — critical for authors debugging
/// override precedence.
///
/// Variants are in ascending precedence: `App` is the lowest layer
/// (most easily overridable), `Inline` is the highest (wins last).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationSource {
    /// Shipped with the binary via `assets/mutations/application.json`.
    App,
    /// Loaded from the user's `mutations.json` (XDG path on native,
    /// `?mutations=` query param or `localStorage` on WASM).
    User,
    /// Declared in the currently-loaded map's `custom_mutations` array.
    Map,
    /// Declared on a specific node's `inline_mutations` array.
    Inline,
}

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
    serde_json::from_str::<Envelope>(source)
        .map(|e| e.mutations)
        .map_err(|e| e.to_string())
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
