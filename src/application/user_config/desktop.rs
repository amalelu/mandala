// SPDX-License-Identifier: MPL-2.0

//! The desktop user-tier fallback chain, composed once.
//!
//! Native has the same two candidate sources for every user-tier
//! config file — an explicit CLI path, then
//! `$XDG_CONFIG_HOME/mandala/<filename>` (with the `$HOME/.config`
//! fallback) — in that order. [`load_desktop_layered`] is that
//! composition, and it is the native peer of
//! `web_storage::load_web_layered`: both name their platform's
//! layers once and hand them to the shared
//! [`super::layered::load_layered`] driver.
//!
//! The three desktop loaders (keybinds, mutations, macros) used to
//! open-code the layer construction, so the `exists()` policy below
//! lived as a comment in two files and a third variant in the
//! macros loader. It lives here now, once.

use std::path::Path;

use super::{load_layered, read_capped, xdg::xdg_mandala_path, ConfigLayer};

/// Load a user-tier config on desktop: the explicit CLI path first,
/// then `$XDG_CONFIG_HOME/mandala/<filename>` (or the
/// `$HOME/.config` fallback), then nothing.
///
/// `label` names the config in log lines (`"keybinds"`,
/// `"mutations"`, `"macros"`). `explicit_path` is `None` for a config
/// that has no CLI flag — the layer is then not built at all rather
/// than built and skipped. Returns the parsed value plus the resolved
/// path of the layer that produced it, owned so the caller can log it
/// after the borrowed layer list is gone.
///
/// Never fails: every layer failure is logged by the driver and the
/// walk continues, so `None` simply means "use your defaults".
///
/// **Precedence asymmetry, deliberate and singly-owned:** only the
/// XDG layer is filtered on `exists()`. An absent user config is the
/// normal case and must stay silent, whereas an explicit
/// `--keybinds <path>` that does not resolve is a user error worth a
/// warning — so the explicit layer is offered to the driver
/// unfiltered and its `stat` failure surfaces. Changing that policy
/// is a change to this function, not to three call sites.
///
/// Costs: one small `Vec` of at most two layers, one `xdg_mandala_path`
/// resolution, and one `Display` render per resolved path — both are
/// rendered up front, before the walk, so the layer list can borrow
/// them (see the comment on that pair below). The `stat` and the read
/// only happen for layers the driver actually reaches.
pub fn load_desktop_layered<T>(
    label: &str,
    filename: &str,
    explicit_path: Option<&Path>,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Option<(T, String)> {
    let default_path = xdg_mandala_path(filename);
    if default_path.is_none() {
        log::debug!(
            "{}: no HOME / XDG_CONFIG_HOME; the default {} layer is disabled",
            label,
            filename
        );
    }

    // Rendered up front so the layer list can borrow them: the
    // driver hands the winning `source` straight back, and it has to
    // outlive the walk.
    let explicit_source = explicit_path.map(|p| p.display().to_string());
    let default_source = default_path.as_deref().map(|p| p.display().to_string());

    let explicit = || explicit_path.map(read_capped);
    let default = || default_path.as_deref().filter(|p| p.exists()).map(read_capped);

    let mut layers: Vec<ConfigLayer<'_, '_>> = Vec::with_capacity(2);
    if let Some(source) = explicit_source.as_deref() {
        layers.push(ConfigLayer {
            source,
            fetch: &explicit,
        });
    }
    if let Some(source) = default_source.as_deref() {
        layers.push(ConfigLayer {
            source,
            fetch: &default,
        });
    }

    load_layered(label, layers, parse).map(|(value, source)| (value, source.to_string()))
}

#[cfg(test)]
mod tests {
    // The XDG layer's own coverage — it winning when no explicit path
    // is given, and losing to one when it is — lives in `xdg.rs`'s
    // test module, beside the path resolution it is about. The
    // `XDG_CONFIG_HOME` override both modules need comes from
    // `super::test_env`: one harness for the whole suite, because a
    // second mutex would serialize only its own callers.
    use super::*;

    /// Accepts anything that is not `!`-prefixed, uppercased so the
    /// test can tell parsed output from raw payload.
    fn parse(src: &str) -> Result<String, String> {
        if src.starts_with('!') {
            Err("payload rejected by parser".to_string())
        } else {
            Ok(src.trim().to_ascii_uppercase())
        }
    }

    /// A path that reliably exists, is readable, and is comfortably
    /// under the cap. These tests need "some real file" rather than
    /// any particular content, and reading one the repo already ships
    /// beats writing a scratch file — nothing to clean up, and
    /// nothing for a parallel `cargo test` in a sibling worktree to
    /// race against.
    fn an_existing_readable_file() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
    }

    /// The explicit CLI path is the top layer, and the returned
    /// source names it — that resolved-path string is what every
    /// desktop loader prints in its "loaded from" line, so it is part
    /// of the wrapper's contract rather than an implementation
    /// detail.
    #[test]
    fn test_desktop_explicit_path_wins_and_names_itself() {
        let existing = an_existing_readable_file();
        let got = load_desktop_layered("test", "no-such-config.json", Some(&existing), parse);
        let (value, source) = got.expect("an existing explicit path must win");
        assert!(
            !value.is_empty(),
            "the explicit layer's payload must reach the parser"
        );
        assert_eq!(
            source,
            existing.display().to_string(),
            "the winning layer must report its resolved path"
        );
    }

    /// An explicit path that does not resolve falls through rather
    /// than aborting — the caller gets `None` and substitutes its
    /// defaults.
    #[test]
    fn test_desktop_missing_explicit_path_falls_through() {
        let got = load_desktop_layered(
            "test",
            "no-such-config.json",
            Some(Path::new("/nonexistent/mandala/config.json")),
            parse,
        );
        assert!(got.is_none());
    }

    /// A config with no CLI flag passes `None` and simply has no
    /// explicit layer built — so an existing file at some unrelated
    /// path cannot win by accident, and the XDG layer is the only
    /// candidate left.
    #[test]
    fn test_desktop_no_explicit_layer_leaves_only_the_xdg_layer() {
        let got = load_desktop_layered("test", "mandala-no-such-config.json", None, parse);
        assert!(got.is_none());
    }
}
