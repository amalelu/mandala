// SPDX-License-Identifier: MPL-2.0

//! Desktop config-source plumbing for keybinds: explicit CLI path
//! first, then `$XDG_CONFIG_HOME/mandala/keybinds.json` (or the
//! `$HOME/.config` fallback), then the hardcoded defaults. Not
//! compiled on WASM.
//!
//! The read, the size cap, the layer construction, and the fallback
//! walk all belong to [`crate::application::user_config`]; this
//! module only names the file and the parser. The mutations and
//! macros desktop loaders name theirs the same way against the same
//! wrapper.

use std::path::Path;

use super::config::KeybindConfig;
use crate::application::user_config::load_desktop_layered;

impl KeybindConfig {
    /// Load a config on desktop, with layered fallback: the explicit
    /// CLI path first, then the default user-config path, then the
    /// hardcoded defaults. Never fails — missing or invalid files are
    /// logged and the next layer is tried.
    pub fn load_for_desktop(explicit_path: Option<&Path>) -> Self {
        match load_desktop_layered("keybinds", "keybinds.json", explicit_path, Self::from_json) {
            Some((cfg, source)) => {
                log::info!("loaded keybinds from {}", source);
                cfg
            }
            None => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use baumhard::util::test_temp::TempDir;

    use crate::application::user_config::MAX_USER_PAYLOAD_BYTES;

    #[test]
    fn test_load_for_desktop_missing_explicit_path_falls_back_to_defaults() {
        let cfg = KeybindConfig::load_for_desktop(Some(Path::new("/nonexistent/keybinds.json")));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
    }

    #[test]
    fn test_load_for_desktop_oversized_file_falls_back_to_defaults() {
        let scratch = TempDir::new("oversized-keybinds");
        let tmp = scratch.join("keybinds.json");
        std::fs::write(&tmp, vec![b' '; MAX_USER_PAYLOAD_BYTES + 1]).unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
    }

    #[test]
    fn test_load_for_desktop_malformed_file_falls_back_to_defaults() {
        let scratch = TempDir::new("bad-keybinds");
        let tmp = scratch.join("keybinds.json");
        std::fs::write(&tmp, "{ this is not json").unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(cfg.undo, KeybindConfig::default().undo);
    }

    #[test]
    fn test_load_for_desktop_valid_explicit_file_wins() {
        let scratch = TempDir::new("good-keybinds");
        let tmp = scratch.join("keybinds.json");
        std::fs::write(&tmp, r#"{"undo": ["Ctrl+Alt+U"]}"#).unwrap();
        let cfg = KeybindConfig::load_for_desktop(Some(&tmp));
        assert_eq!(
            cfg.undo,
            vec!["Ctrl+Alt+U".to_string()],
            "the explicit file must win over the defaults"
        );
    }
}
