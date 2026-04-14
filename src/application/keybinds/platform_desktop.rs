//! Desktop config-source plumbing: file-based `KeybindConfig` loading
//! with the `$XDG_CONFIG_HOME` / `$HOME/.config` fallback, plus the
//! layered `load_for_desktop` driver. Not compiled on WASM.

use log::warn;
use std::path::{Path, PathBuf};

use super::config::KeybindConfig;

impl KeybindConfig {
    /// Load a config from a file on disk. Desktop-only; WASM users load
    /// via `load_from_web`. Failures return an error string the caller can
    /// log.
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {}", path.display(), e))?;
        Self::from_json(&json)
    }

    /// Load a config on desktop, with layered fallback: explicit CLI path
    /// > default user-config path > hardcoded defaults. Never fails —
    /// missing or invalid files are logged and the next layer is tried.
    pub fn load_for_desktop(explicit_path: Option<&Path>) -> Self {
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
}

/// Conventional default path for the desktop user's keybinds config. Uses
/// `$XDG_CONFIG_HOME/mandala/keybinds.json` if set, falling back to
/// `$HOME/.config/mandala/keybinds.json`. Returns `None` if neither env
/// variable is set.
pub fn default_desktop_config_path() -> Option<PathBuf> {
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
