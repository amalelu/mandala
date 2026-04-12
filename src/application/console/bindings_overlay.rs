//! Persistence layer for `mutate bind` / `mutate unbind`.
//!
//! Separate from `keybinds.json` so console-set bindings don't
//! trample a user's hand-edited config file. Lives at
//! `$XDG_CONFIG_HOME/mandala/console_bindings.json` on desktop and
//! `localStorage["mandala_console_bindings"]` on WASM.
//!
//! At startup the overlay's bindings are merged *on top of* the
//! ones parsed from `keybinds.json` so a user-edited entry with the
//! same combo wins (matching the resolve-time semantics where the
//! most recent insertion into `custom_binds` takes precedence).
//!
//! File format:
//!
//! ```json
//! {
//!   "version": 1,
//!   "bindings": { "Ctrl+Shift+M": "my-mutation-id" }
//! }
//! ```

use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleBindingsOverlay {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub bindings: HashMap<String, String>,
}

fn default_version() -> u32 {
    1
}

impl Default for ConsoleBindingsOverlay {
    fn default() -> Self {
        Self {
            version: 1,
            bindings: HashMap::new(),
        }
    }
}

impl ConsoleBindingsOverlay {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("parse console bindings overlay: {}", e))
    }

    pub fn to_pretty_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("serialize overlay: {}", e))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_for_desktop() -> Self {
        let path = match default_desktop_path() {
            Some(p) => p,
            None => return Self::default(),
        };
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::from_json(&s).unwrap_or_else(|e| {
                warn!("console bindings overlay {} parse failed: {}", path.display(), e);
                Self::default()
            }),
            Err(e) => {
                warn!("console bindings overlay {} read failed: {}", path.display(), e);
                Self::default()
            }
        }
    }

    /// Persist the overlay to its desktop path. Best-effort: logs
    /// and returns on any failure — a failed save must not abort
    /// the interactive session.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_for_desktop(&self) {
        let path = match default_desktop_path() {
            Some(p) => p,
            None => return,
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("console bindings overlay mkdir {}: {}", parent.display(), e);
                return;
            }
        }
        match self.to_pretty_json() {
            Ok(s) => {
                if let Err(e) = std::fs::write(&path, s) {
                    warn!("console bindings overlay write {}: {}", path.display(), e);
                }
            }
            Err(e) => warn!("console bindings overlay serialize: {}", e),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn load_for_web() -> Self {
        let window = match web_sys::window() {
            Some(w) => w,
            None => return Self::default(),
        };
        let storage = match window.local_storage().ok().flatten() {
            Some(s) => s,
            None => return Self::default(),
        };
        let raw = match storage.get_item("mandala_console_bindings").ok().flatten() {
            Some(r) => r,
            None => return Self::default(),
        };
        Self::from_json(&raw).unwrap_or_else(|e| {
            warn!("console bindings overlay parse failed: {}", e);
            Self::default()
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub fn save_for_web(&self) {
        let window = match web_sys::window() {
            Some(w) => w,
            None => return,
        };
        let storage = match window.local_storage().ok().flatten() {
            Some(s) => s,
            None => return,
        };
        if let Ok(s) = self.to_pretty_json() {
            let _ = storage.set_item("mandala_console_bindings", &s);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn default_desktop_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            let mut p = PathBuf::from(xdg);
            p.push("mandala");
            p.push("console_bindings.json");
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let mut p = PathBuf::from(home);
            p.push(".config");
            p.push("mandala");
            p.push("console_bindings.json");
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bindings_overlay_empty_json_parses() {
        let json = r#"{"version": 1, "bindings": {}}"#;
        let o = ConsoleBindingsOverlay::from_json(json).unwrap();
        assert!(o.bindings.is_empty());
    }

    #[test]
    fn test_bindings_overlay_round_trip() {
        let mut o = ConsoleBindingsOverlay::default();
        o.bindings.insert("Ctrl+Shift+M".into(), "my-mutation".into());
        let json = o.to_pretty_json().unwrap();
        let back = ConsoleBindingsOverlay::from_json(&json).unwrap();
        assert_eq!(
            back.bindings.get("Ctrl+Shift+M").map(|s| s.as_str()),
            Some("my-mutation")
        );
    }

    #[test]
    fn test_bindings_overlay_malformed_errors() {
        assert!(ConsoleBindingsOverlay::from_json("not json").is_err());
    }
}
