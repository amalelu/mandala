//! Configurable keybindings.
//!
//! This module holds the configuration for every keyboard-driven action the
//! app supports, so users can rebind them without recompiling. The design
//! favors flexibility and forgiving loading:
//!
//! - Start with hardcoded defaults (`KeybindConfig::default`).
//! - Overlay a JSON config file if one is found or explicitly specified.
//! - On desktop, the file path is either supplied via CLI (`--keybinds
//!   <path>`) or looked up at a conventional location
//!   (`$XDG_CONFIG_HOME/mandala/keybinds.json`, or
//!   `$HOME/.config/mandala/keybinds.json`).
//! - On WASM, the config is read from a URL query param (`?keybinds=<json>`)
//!   or from `localStorage` under the key `mandala_keybinds`.
//! - Any failure to load a layer is logged and the layer is skipped — the
//!   app never crashes for a bad keybinds file.
//!
//! Partial configs are supported via serde's `default` attribute: an unset
//! field falls back to its hardcoded default, so a user can override a
//! single action without respecifying everything.
//!
//! Actions are the abstract operations the UI cares about. The event loop
//! queries `ResolvedKeybinds::action_for(...)` given a winit key event and
//! the current modifier state, then dispatches on the returned `Action`.

use log::warn;
use serde::{Deserialize, Serialize};

/// High-level user actions that can be bound to keys. Add a new variant
/// here when a new keyboard interaction is introduced, extend
/// `KeybindConfig` with a matching field + default, and handle the variant
/// in the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Undo the last action on the document.
    Undo,
    /// Enter reparent mode for the currently selected nodes.
    EnterReparentMode,
    /// Enter connect mode for the currently selected node.
    EnterConnectMode,
    /// Delete the current selection (currently: selected edge).
    DeleteSelection,
    /// Cancel the current mode (reparent / connect).
    CancelMode,
    /// Create a new unattached (orphan) node at the cursor position. The
    /// node starts with no parent so users can build a piece in isolation
    /// and attach it with reparent mode (Ctrl+P) later.
    CreateOrphanNode,
    /// Detach every currently selected node from its parent, promoting it
    /// to a root node. Each selected node's full subtree stays attached to
    /// it — this only severs the link between the selection and its
    /// former parent, not the selection and its children.
    OrphanSelection,
}

/// A parsed keybinding: a logical key name plus modifier flags. Key names
/// are normalized to lowercase during parsing so comparisons are
/// case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBind {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyBind {
    /// Parse a binding string like `"Ctrl+Z"`, `"Shift+Alt+Delete"`, or
    /// just `"Escape"`. Modifier order doesn't matter; whitespace is
    /// tolerated; key names are matched case-insensitively.
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key: Option<String> = None;

        for raw in input.split('+') {
            let part = raw.trim().to_ascii_lowercase();
            if part.is_empty() {
                continue;
            }
            match part.as_str() {
                "ctrl" | "control" | "cmd" | "command" | "meta" | "super" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                _ => {
                    if key.is_some() {
                        return Err(format!(
                            "keybind '{}' has multiple non-modifier keys",
                            input
                        ));
                    }
                    key = Some(part);
                }
            }
        }

        match key {
            Some(key) => Ok(KeyBind { key, ctrl, shift, alt }),
            None => Err(format!("keybind '{}' has no key", input)),
        }
    }

    /// Returns true if this binding matches the given logical key name and
    /// modifier state. The caller is expected to have normalized `key_name`
    /// to lowercase via `normalize_key_name`.
    pub fn matches(&self, key_name: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        self.key == key_name && self.ctrl == ctrl && self.shift == shift && self.alt == alt
    }
}

/// Normalize a winit logical-key representation to the same lowercase form
/// `KeyBind::parse` uses. The caller passes the string form it extracted
/// from its key event (character or named-key debug name) and this function
/// lowercases and trims it.
pub fn normalize_key_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// Convert a winit `Key` into the lowercase string form that
/// `KeyBind::parse` produces, so keybind comparison is symmetric.
/// Pairs with `normalize_key_name`; the two together produce comparable
/// strings from either the stored-config side or the live-event side.
pub fn key_to_name(key: &winit::keyboard::Key) -> Option<String> {
    use winit::keyboard::Key;
    match key {
        Key::Character(c) => Some(normalize_key_name(c.as_ref())),
        Key::Named(named) => Some(normalize_key_name(&format!("{:?}", named))),
        _ => None,
    }
}

/// The raw, user-editable config. Every field is a list of binding strings
/// so users can assign multiple keys to the same action (e.g. Ctrl+Z and
/// the Undo key both mapped to `Undo`). Fields default via serde so a
/// partial config only has to mention the actions the user wants to
/// override.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybindConfig {
    pub undo: Vec<String>,
    pub enter_reparent_mode: Vec<String>,
    pub enter_connect_mode: Vec<String>,
    pub delete_selection: Vec<String>,
    pub cancel_mode: Vec<String>,
    pub create_orphan_node: Vec<String>,
    pub orphan_selection: Vec<String>,
}

impl Default for KeybindConfig {
    fn default() -> Self {
        KeybindConfig {
            undo: vec!["Ctrl+Z".into(), "Undo".into()],
            enter_reparent_mode: vec!["Ctrl+P".into()],
            enter_connect_mode: vec!["Ctrl+D".into()],
            delete_selection: vec!["Delete".into()],
            cancel_mode: vec!["Escape".into()],
            create_orphan_node: vec!["Ctrl+N".into()],
            orphan_selection: vec!["Ctrl+O".into()],
        }
    }
}

impl KeybindConfig {
    /// Parse a JSON string into a config. Missing fields fall back to
    /// defaults thanks to `#[serde(default)]` on the struct.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("parse keybinds JSON: {}", e))
    }

    /// Parse every binding string into concrete `KeyBind` values. Any
    /// binding that fails to parse is logged and skipped so a single typo
    /// doesn't break the entire config.
    pub fn resolve(&self) -> ResolvedKeybinds {
        let mut binds: Vec<(Action, KeyBind)> = Vec::new();
        let sets = [
            (Action::Undo, &self.undo),
            (Action::EnterReparentMode, &self.enter_reparent_mode),
            (Action::EnterConnectMode, &self.enter_connect_mode),
            (Action::DeleteSelection, &self.delete_selection),
            (Action::CancelMode, &self.cancel_mode),
            (Action::CreateOrphanNode, &self.create_orphan_node),
            (Action::OrphanSelection, &self.orphan_selection),
        ];
        for (action, strings) in sets {
            for s in strings {
                match KeyBind::parse(s) {
                    Ok(k) => binds.push((action, k)),
                    Err(e) => warn!("skipping invalid keybind '{}': {}", s, e),
                }
            }
        }
        ResolvedKeybinds { binds }
    }

    /// Load a config from a file on disk. Desktop-only; WASM users load
    /// via `load_from_web`. Failures return an error string the caller can
    /// log.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {}", path.display(), e))?;
        Self::from_json(&json)
    }

    /// Load a config on desktop, with layered fallback: explicit CLI path
    /// > default user-config path > hardcoded defaults. Never fails —
    /// missing or invalid files are logged and the next layer is tried.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_for_desktop(explicit_path: Option<&std::path::Path>) -> Self {
        if let Some(p) = explicit_path {
            match Self::load_from_file(p) {
                Ok(cfg) => {
                    log::info!("loaded keybinds from {}", p.display());
                    return cfg;
                }
                Err(e) => warn!("keybinds load failed for explicit path: {}", e),
            }
        }
        if let Some(default_path) = default_desktop_config_path() {
            if default_path.exists() {
                match Self::load_from_file(&default_path) {
                    Ok(cfg) => {
                        log::info!("loaded keybinds from {}", default_path.display());
                        return cfg;
                    }
                    Err(e) => warn!("keybinds load failed for default path: {}", e),
                }
            }
        }
        Self::default()
    }

    /// Load a config on WASM, with layered fallback: URL `?keybinds=<json>`
    /// query param (inline JSON, URL-encoded) > `localStorage` value under
    /// the `mandala_keybinds` key > hardcoded defaults.
    #[cfg(target_arch = "wasm32")]
    pub fn load_for_web() -> Self {
        if let Some(json) = read_keybinds_from_query() {
            match Self::from_json(&json) {
                Ok(cfg) => {
                    log::info!("loaded keybinds from URL query param");
                    return cfg;
                }
                Err(e) => warn!("keybinds query param parse failed: {}", e),
            }
        }
        if let Some(json) = read_keybinds_from_local_storage() {
            match Self::from_json(&json) {
                Ok(cfg) => {
                    log::info!("loaded keybinds from localStorage");
                    return cfg;
                }
                Err(e) => warn!("keybinds localStorage parse failed: {}", e),
            }
        }
        Self::default()
    }
}

/// The resolved form of a `KeybindConfig`: a flat list of `(Action,
/// KeyBind)` pairs. Lookup is linear but the list is tiny (under a dozen
/// entries), so a hash map would only add overhead.
#[derive(Debug, Clone)]
pub struct ResolvedKeybinds {
    binds: Vec<(Action, KeyBind)>,
}

impl ResolvedKeybinds {
    /// Return the action bound to the given key event, if any. The caller
    /// passes the normalized key name (see `normalize_key_name`) and the
    /// current modifier state.
    pub fn action_for(&self, key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<Action> {
        for (action, bind) in &self.binds {
            if bind.matches(key, ctrl, shift, alt) {
                return Some(*action);
            }
        }
        None
    }

    /// Returns true if the given key event is bound to the given action.
    /// Convenience for the event loop.
    pub fn is(&self, action: Action, key: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        self.action_for(key, ctrl, shift, alt) == Some(action)
    }
}

/// Conventional default path for the desktop user's keybinds config. Uses
/// `$XDG_CONFIG_HOME/mandala/keybinds.json` if set, falling back to
/// `$HOME/.config/mandala/keybinds.json`. Returns `None` if neither env
/// variable is set.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_desktop_config_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            let mut p = PathBuf::from(xdg);
            p.push("mandala");
            p.push("keybinds.json");
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let mut p = PathBuf::from(home);
            p.push(".config");
            p.push("mandala");
            p.push("keybinds.json");
            return Some(p);
        }
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn read_keybinds_from_query() -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    // Expect format: "?keybinds=<url-encoded-json>" or
    // "?map=foo&keybinds=<url-encoded-json>"
    let trimmed = search.trim_start_matches('?');
    for pair in trimmed.split('&') {
        if let Some(val) = pair.strip_prefix("keybinds=") {
            // Manual URL-decode: replace + with space, percent-decode the rest.
            let decoded = js_sys::decode_uri_component(val).ok()?;
            return decoded.as_string();
        }
    }
    None
}

#[cfg(target_arch = "wasm32")]
fn read_keybinds_from_local_storage() -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item("mandala_keybinds").ok()?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_key() {
        let k = KeyBind::parse("Escape").unwrap();
        assert_eq!(k.key, "escape");
        assert!(!k.ctrl && !k.shift && !k.alt);
    }

    #[test]
    fn test_parse_ctrl_z() {
        let k = KeyBind::parse("Ctrl+Z").unwrap();
        assert_eq!(k.key, "z");
        assert!(k.ctrl);
        assert!(!k.shift && !k.alt);
    }

    #[test]
    fn test_parse_is_case_insensitive() {
        let k1 = KeyBind::parse("ctrl+z").unwrap();
        let k2 = KeyBind::parse("CTRL+Z").unwrap();
        let k3 = KeyBind::parse("Ctrl+Z").unwrap();
        assert_eq!(k1, k2);
        assert_eq!(k2, k3);
    }

    #[test]
    fn test_parse_all_modifiers() {
        let k = KeyBind::parse("ctrl+shift+alt+delete").unwrap();
        assert_eq!(k.key, "delete");
        assert!(k.ctrl && k.shift && k.alt);
    }

    #[test]
    fn test_parse_whitespace_tolerated() {
        let k = KeyBind::parse(" Ctrl + Z ").unwrap();
        assert_eq!(k.key, "z");
        assert!(k.ctrl);
    }

    #[test]
    fn test_parse_modifier_aliases() {
        // cmd/command/meta/super all map to ctrl for cross-platform muscle memory
        assert!(KeyBind::parse("Cmd+Z").unwrap().ctrl);
        assert!(KeyBind::parse("Meta+Z").unwrap().ctrl);
        assert!(KeyBind::parse("Super+Z").unwrap().ctrl);
        // option aliases alt
        assert!(KeyBind::parse("Option+Z").unwrap().alt);
    }

    #[test]
    fn test_parse_rejects_empty() {
        assert!(KeyBind::parse("").is_err());
        assert!(KeyBind::parse("Ctrl+").is_err());
    }

    #[test]
    fn test_parse_rejects_multiple_keys() {
        assert!(KeyBind::parse("Z+X").is_err());
        assert!(KeyBind::parse("Ctrl+Z+X").is_err());
    }

    #[test]
    fn test_matches_modifiers_exactly() {
        let k = KeyBind::parse("Ctrl+Z").unwrap();
        assert!(k.matches("z", true, false, false));
        // Extra shift mustn't match
        assert!(!k.matches("z", true, true, false));
        // Missing ctrl mustn't match
        assert!(!k.matches("z", false, false, false));
    }

    #[test]
    fn test_default_config_has_all_actions() {
        let cfg = KeybindConfig::default();
        let resolved = cfg.resolve();
        assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
        assert_eq!(resolved.action_for("p", true, false, false), Some(Action::EnterReparentMode));
        assert_eq!(resolved.action_for("d", true, false, false), Some(Action::EnterConnectMode));
        assert_eq!(resolved.action_for("delete", false, false, false), Some(Action::DeleteSelection));
        assert_eq!(resolved.action_for("escape", false, false, false), Some(Action::CancelMode));
        assert_eq!(resolved.action_for("n", true, false, false), Some(Action::CreateOrphanNode));
        assert_eq!(resolved.action_for("o", true, false, false), Some(Action::OrphanSelection));
    }

    #[test]
    fn test_default_config_has_undo_alias() {
        // Ctrl+Z and the bare "Undo" key should both fire undo
        let cfg = KeybindConfig::default();
        let resolved = cfg.resolve();
        assert_eq!(resolved.action_for("undo", false, false, false), Some(Action::Undo));
    }

    #[test]
    fn test_partial_json_uses_defaults_for_missing_fields() {
        // A user who only wants to rebind one action should be able to omit
        // every other field and get the defaults for them.
        let json = r#"{ "undo": ["Ctrl+Y"] }"#;
        let cfg = KeybindConfig::from_json(json).unwrap();
        assert_eq!(cfg.undo, vec!["Ctrl+Y"]);
        // Other fields should still have defaults
        assert_eq!(cfg.enter_reparent_mode, vec!["Ctrl+P"]);
        assert_eq!(cfg.cancel_mode, vec!["Escape"]);
    }

    #[test]
    fn test_resolve_skips_invalid_bindings() {
        let cfg = KeybindConfig {
            undo: vec!["Ctrl+Z".into(), "Z+X".into()], // second is invalid
            ..KeybindConfig::default()
        };
        let resolved = cfg.resolve();
        // Valid binding still works
        assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
    }

    #[test]
    fn test_user_override_replaces_default() {
        // A user who specifies undo bindings should get only those — not
        // theirs merged with the hardcoded list. This matches common
        // config-file intuition.
        let json = r#"{ "undo": ["Ctrl+Y"] }"#;
        let cfg = KeybindConfig::from_json(json).unwrap();
        let resolved = cfg.resolve();
        assert_eq!(resolved.action_for("y", true, false, false), Some(Action::Undo));
        // Original Ctrl+Z no longer bound
        assert_eq!(resolved.action_for("z", true, false, false), None);
    }

    #[test]
    fn test_json_roundtrip() {
        let cfg = KeybindConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed = KeybindConfig::from_json(&json).unwrap();
        let resolved = parsed.resolve();
        assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
    }

    #[test]
    fn test_normalize_key_name() {
        assert_eq!(normalize_key_name("Escape"), "escape");
        assert_eq!(normalize_key_name("  Delete  "), "delete");
        assert_eq!(normalize_key_name("Z"), "z");
    }
}
