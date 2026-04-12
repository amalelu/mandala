//! User-defined custom-mutations file.
//!
//! Path (desktop): `$XDG_CONFIG_HOME/mandala/mutations.json` with a
//! fallback to `$HOME/.config/mandala/mutations.json`. WASM reads
//! `localStorage["mandala_user_mutations"]`.
//!
//! File format (serde-compatible with [`CustomMutation`]'s derive):
//!
//! ```json
//! {
//!   "version": 1,
//!   "mutations": [
//!     {
//!       "id": "nudge-right",
//!       "name": "Nudge right",
//!       "mutations": [],
//!       "target_scope": "SelfOnly"
//!     }
//!   ],
//!   "aliases": {
//!     "a": "anchor set from auto"
//!   }
//! }
//! ```
//!
//! The file is best-effort: a missing or malformed file is logged
//! and skipped. Creating or editing the file is out of scope for
//! the console — the console only reads it at startup and merges
//! the mutations into the registry at the *lowest* precedence
//! (user < map < inline).

use baumhard::mindmap::custom_mutation::CustomMutation;
use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMutationsFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub mutations: Vec<CustomMutation>,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

fn default_version() -> u32 {
    1
}

impl Default for UserMutationsFile {
    fn default() -> Self {
        Self {
            version: 1,
            mutations: Vec::new(),
            aliases: HashMap::new(),
        }
    }
}

impl UserMutationsFile {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("parse user mutations: {}", e))
    }

    /// Load the user-mutations file from disk on desktop, returning
    /// `Default::default()` (empty) if the file doesn't exist or
    /// fails to parse.
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
            Ok(s) => match Self::from_json(&s) {
                Ok(f) => {
                    log::info!(
                        "loaded {} user mutations from {}",
                        f.mutations.len(),
                        path.display()
                    );
                    f
                }
                Err(e) => {
                    warn!("user mutations {} parse failed: {}", path.display(), e);
                    Self::default()
                }
            },
            Err(e) => {
                warn!("user mutations {} read failed: {}", path.display(), e);
                Self::default()
            }
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
        let raw = match storage.get_item("mandala_user_mutations").ok().flatten() {
            Some(r) => r,
            None => return Self::default(),
        };
        match Self::from_json(&raw) {
            Ok(f) => f,
            Err(e) => {
                warn!("user mutations localStorage parse failed: {}", e);
                Self::default()
            }
        }
    }
}

/// Conventional default path for the desktop user's mutations file,
/// mirroring `keybinds::default_desktop_config_path`. Returns `None`
/// when neither `XDG_CONFIG_HOME` nor `HOME` is set (headless CI).
#[cfg(not(target_arch = "wasm32"))]
pub fn default_desktop_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            let mut p = PathBuf::from(xdg);
            p.push("mandala");
            p.push("mutations.json");
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            let mut p = PathBuf::from(home);
            p.push(".config");
            p.push("mandala");
            p.push("mutations.json");
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_mutations_file_empty_json_parses() {
        let json = r#"{"version": 1, "mutations": [], "aliases": {}}"#;
        let f = UserMutationsFile::from_json(json).unwrap();
        assert_eq!(f.version, 1);
        assert!(f.mutations.is_empty());
        assert!(f.aliases.is_empty());
    }

    #[test]
    fn test_user_mutations_file_missing_fields_default() {
        // Only `version` set — mutations and aliases should default.
        let json = r#"{"version": 1}"#;
        let f = UserMutationsFile::from_json(json).unwrap();
        assert_eq!(f.version, 1);
        assert!(f.mutations.is_empty());
        assert!(f.aliases.is_empty());
    }

    #[test]
    fn test_user_mutations_file_with_mutation_round_trip() {
        let json = r#"{
            "version": 1,
            "mutations": [
                {
                    "id": "test-mutation",
                    "name": "Test",
                    "mutations": [],
                    "target_scope": "SelfOnly"
                }
            ],
            "aliases": {"t": "mutate run test-mutation"}
        }"#;
        let f = UserMutationsFile::from_json(json).unwrap();
        assert_eq!(f.mutations.len(), 1);
        assert_eq!(f.mutations[0].id, "test-mutation");
        assert_eq!(
            f.aliases.get("t").map(|s| s.as_str()),
            Some("mutate run test-mutation")
        );
    }

    #[test]
    fn test_user_mutations_file_malformed_json_errors() {
        let json = "{not json";
        assert!(UserMutationsFile::from_json(json).is_err());
    }

    #[test]
    fn test_user_mutations_file_default_is_empty() {
        let f = UserMutationsFile::default();
        assert!(f.mutations.is_empty());
        assert!(f.aliases.is_empty());
    }
}
