//! Desktop user-file plumbing: filesystem-based user mutation loading
//! with `$XDG_CONFIG_HOME` / `$HOME/.config` fallback, mirroring the
//! shape of `keybinds::platform_desktop`. Not compiled on WASM.

use log::warn;
use std::path::{Path, PathBuf};

use baumhard::mindmap::custom_mutation::CustomMutation;

/// Load user mutations, with layered fallback: explicit CLI path >
/// `$XDG_CONFIG_HOME/mandala/mutations.json` >
/// `$HOME/.config/mandala/mutations.json` > empty. Never fails —
/// missing or invalid files are logged and the next layer is tried.
pub fn load_user(explicit_path: Option<&Path>) -> Vec<CustomMutation> {
    if let Some(p) = explicit_path {
        match read_and_parse(p) {
            Ok(v) => {
                log::info!("loaded {} user mutations from {}", v.len(), p.display());
                return v;
            }
            Err(e) => warn!("mutations load failed for explicit path: {}", e),
        }
    }
    if let Some(default_path) = default_user_mutations_path() {
        if default_path.exists() {
            match read_and_parse(&default_path) {
                Ok(v) => {
                    log::info!(
                        "loaded {} user mutations from {}",
                        v.len(),
                        default_path.display()
                    );
                    return v;
                }
                Err(e) => warn!("mutations load failed for default path: {}", e),
            }
        }
    }
    Vec::new()
}

fn read_and_parse(path: &Path) -> Result<Vec<CustomMutation>, String> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;
    super::parse_mutations_json(&src)
}

/// `$XDG_CONFIG_HOME/mandala/mutations.json` if set, else
/// `$HOME/.config/mandala/mutations.json`. `None` if neither env
/// variable is set.
pub fn default_user_mutations_path() -> Option<PathBuf> {
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
    fn missing_file_returns_empty_vec() {
        let p = Path::new("/nonexistent/path/mutations.json");
        let v = load_user(Some(p));
        assert!(v.is_empty());
    }

    #[test]
    fn malformed_file_returns_empty_vec_and_warns() {
        let tmp = std::env::temp_dir().join("mandala_test_bad_mutations.json");
        std::fs::write(&tmp, "{ this is not json").unwrap();
        let v = load_user(Some(&tmp));
        assert!(v.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn valid_file_loads_mutations() {
        let tmp = std::env::temp_dir().join("mandala_test_good_mutations.json");
        let src = r#"{
            "mutations": [{
                "id": "user-mut",
                "name": "User Mutation",
                "mutator": {"Macro": {"channel": 0, "mutations": {"Literal": []}}},
                "target_scope": "SelfOnly"
            }]
        }"#;
        std::fs::write(&tmp, src).unwrap();
        let v = load_user(Some(&tmp));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "user-mut");
        let _ = std::fs::remove_file(&tmp);
    }
}
