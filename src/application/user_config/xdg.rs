// SPDX-License-Identifier: MPL-2.0

//! Resolve the canonical user-config path for a given filename
//! under `mandala/`. `$XDG_CONFIG_HOME` if set, else
//! `$HOME/.config`. Returns `None` only in degenerate environments
//! where neither variable is set — the caller treats that as
//! "no user config available, fall back to defaults."

use std::path::PathBuf;

/// `$XDG_CONFIG_HOME/mandala/<filename>` if `XDG_CONFIG_HOME` is set
/// and non-empty; else `$HOME/.config/mandala/<filename>` if `HOME`
/// is set and non-empty; else `None`. The `mandala` directory is the
/// project's XDG namespace. Callers pass the leaf filename (e.g.
/// `"keybinds.json"`); this helper does the directory-joining.
///
/// O(1) modulo the env-lookup syscalls; allocates the returned
/// `PathBuf` only on the success branch.
pub fn xdg_mandala_path(filename: &str) -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("mandala").join(filename));
        }
    }
    let home = std::env::var("HOME").ok().filter(|s| !s.is_empty())?;
    Some(PathBuf::from(home).join(".config").join("mandala").join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::user_config::load_desktop_layered;
    use crate::application::user_config::test_env::with_env;
    use baumhard::util::test_temp::TempDir;

    #[test]
    fn xdg_wins_over_home_when_both_set() {
        with_env(Some("/tmp/xdg"), Some("/tmp/home"), || {
            let p = xdg_mandala_path("keybinds.json").unwrap();
            assert_eq!(p, PathBuf::from("/tmp/xdg/mandala/keybinds.json"));
        });
    }

    #[test]
    fn empty_xdg_falls_through_to_home() {
        with_env(Some(""), Some("/tmp/home"), || {
            let p = xdg_mandala_path("mutations.json").unwrap();
            assert_eq!(p, PathBuf::from("/tmp/home/.config/mandala/mutations.json"),);
        });
    }

    #[test]
    fn neither_set_returns_none() {
        with_env(None, None, || {
            assert!(xdg_mandala_path("macros.json").is_none());
        });
    }

    #[test]
    fn empty_home_with_unset_xdg_returns_none() {
        with_env(None, Some(""), || {
            assert!(xdg_mandala_path("macros.json").is_none());
        });
    }

    /// Accepts any payload, uppercased so an assertion can tell
    /// parsed output from the raw bytes on disk. These tests are about
    /// which layer wins, so the parser never rejects — `desktop.rs`'s
    /// own fixture covers the rejecting case.
    fn parse(src: &str) -> Result<String, String> {
        Ok(src.trim().to_ascii_uppercase())
    }

    /// Lay out `<dir>/mandala/<filename>` with `payload` in it — the
    /// exact shape [`xdg_mandala_path`] resolves to — and return the
    /// resolved path so the caller can assert on the reported source.
    fn write_xdg_config(dir: &TempDir, filename: &str, payload: &str) -> PathBuf {
        let cfg_dir = dir.join("mandala");
        std::fs::create_dir_all(&cfg_dir).expect("create the mandala config dir");
        let cfg = cfg_dir.join(filename);
        std::fs::write(&cfg, payload).expect("write the xdg config file");
        cfg
    }

    /// The XDG layer of `load_desktop_layered` wins when there is no
    /// explicit CLI path, and reports the path it resolved.
    ///
    /// This is the pin on the default layer's existence: deleting the
    /// `layers.push` that builds it leaves every test in `desktop.rs`
    /// green, because none of them puts a file where the XDG layer
    /// looks. Without this test a refactor could drop
    /// `~/.config/mandala/*.json` support for every desktop user in
    /// silence.
    ///
    /// It lives here rather than beside `load_desktop_layered`
    /// because it is about what `xdg_mandala_path` resolves to; the
    /// `XDG_CONFIG_HOME` override it needs comes from the shared
    /// [`crate::application::user_config::test_env`] harness, which
    /// serializes it against every other env-touching test in the
    /// suite.
    #[test]
    fn test_desktop_xdg_layer_wins_when_there_is_no_explicit_path() {
        let dir = TempDir::new("xdg-layer-wins");
        let cfg = write_xdg_config(&dir, "keybinds.json", "from-xdg");
        with_env(Some(&dir.path().display().to_string()), None, || {
            let (value, source) = load_desktop_layered("test", "keybinds.json", None, parse)
                .expect("an existing XDG config must win when no explicit path is given");
            assert_eq!(value, "FROM-XDG", "the XDG layer's payload must reach the parser");
            assert_eq!(
                source,
                cfg.display().to_string(),
                "the winning layer must report its resolved XDG path"
            );
        });
    }

    /// An explicit CLI path outranks an existing XDG config — the
    /// precedence the wrapper's layer order exists to express, and
    /// untestable without an XDG file actually in place to lose.
    #[test]
    fn test_desktop_explicit_path_beats_the_xdg_layer() {
        let dir = TempDir::new("xdg-layer-outranked");
        write_xdg_config(&dir, "keybinds.json", "from-xdg");
        let explicit = dir.join("explicit-keybinds.json");
        std::fs::write(&explicit, "from-explicit").expect("write the explicit config file");
        with_env(Some(&dir.path().display().to_string()), None, || {
            let (value, source) = load_desktop_layered("test", "keybinds.json", Some(&explicit), parse)
                .expect("the explicit path must win over the XDG config");
            assert_eq!(value, "FROM-EXPLICIT", "the explicit layer must outrank XDG");
            assert_eq!(
                source,
                explicit.display().to_string(),
                "the winning layer must report the explicit path"
            );
        });
    }
}
