// SPDX-License-Identifier: MPL-2.0

//! The one place the test suite overrides `XDG_CONFIG_HOME` / `HOME`.
//!
//! Environment variables are per-process, and cargo runs unit tests
//! on one thread per core, so every test that touches them has to
//! take the *same* lock — a second mutex somewhere else would
//! serialize its own callers against each other and nothing else.
//! That is why this is a module rather than a helper copied into each
//! test module that needs it.
//!
//! Two kinds of caller:
//!
//! - Tests *of* the path resolution ([`with_env`]) — `xdg.rs` sets
//!   the variables to fixed values and asserts on the path that comes
//!   out.
//! - Tests of anything that reaches a user-tier loader
//!   ([`with_no_user_config`]). `load_desktop_layered` builds up to
//!   two layers, so a test asserting that a broken explicit file
//!   falls back to the built-in defaults is really asserting that
//!   *and* that the machine running it has no
//!   `~/.config/mandala/<file>.json`. On a developer's own laptop it
//!   quietly does have one, the fallback lands on that file instead,
//!   and the test passes or fails on the contents of a file nobody
//!   wrote for it. Pointing the XDG layer at an empty scratch
//!   directory makes "the defaults" mean the defaults.
//!
//! Native-only: `wasm32` has no environment and no desktop loader to
//! isolate.

use std::sync::Mutex;

use baumhard::util::test_temp::TempDir;

/// Process-wide mutex serializing tests that mutate `std::env::*`.
///
/// Poisoning is tolerated: a panicking test still restores the
/// environment on its way out (see `EnvRestore`), so the only thing a
/// poisoned guard tells us is that some test failed — not that the
/// environment is dirty. Taking the lock anyway beats deadlocking the
/// rest of the suite behind it.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Run `body` with `XDG_CONFIG_HOME` and `HOME` overridden, then
/// restored. Holds [`ENV_LOCK`] for the duration, so concurrent
/// env-touching tests never observe each other's mid-mutation state.
///
/// `None` for either variable removes it for the duration.
///
/// Do not nest this inside itself or inside
/// [`with_no_user_config`] — [`ENV_LOCK`] is a plain `Mutex` and a
/// re-entrant acquisition deadlocks. Run the two phases one after
/// the other instead.
pub fn with_env<F: FnOnce()>(xdg: Option<&str>, home: Option<&str>, body: F) {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // Restoration is a `Drop`, not a statement after `body()`. A
    // failing assertion inside `body` unwinds, and a restore written
    // as a trailing statement is simply skipped — leaving `HOME` and
    // `XDG_CONFIG_HOME` pointing at a `TempDir` that the same unwind
    // just deleted, for every later test in the process. One red test
    // would become a run whose remaining results cannot be trusted,
    // which is the class this harness exists to close.
    //
    // Declared after `_guard` so it drops *before* the lock is
    // released: no other test can observe the intermediate state.
    let _restore = EnvRestore {
        xdg: std::env::var_os("XDG_CONFIG_HOME"),
        home: std::env::var_os("HOME"),
    };
    set_or_clear("XDG_CONFIG_HOME", xdg);
    set_or_clear("HOME", home);
    body();
}

/// Put `key` back the way [`EnvRestore`] found it, or remove it if it
/// was unset.
fn set_or_clear(key: &str, value: Option<&str>) {
    match value {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

/// Restores the two variables [`with_env`] overrides, on the way out
/// of the scope — normal return or unwind alike.
struct EnvRestore {
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match self.xdg.take() {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match self.home.take() {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}

/// Run `body` on a machine that has no user config of any kind:
/// both `XDG_CONFIG_HOME` and `HOME` point into a fresh scratch
/// directory with no `mandala/` inside it.
///
/// Reach for this in any test that asserts a loader fell back to its
/// built-in defaults. Both variables are set rather than unset
/// because unsetting them makes `xdg_mandala_path` return `None`,
/// which skips the layer for a *different* reason than the one the
/// test is exercising — an empty directory keeps the layer built and
/// filtered on `exists()`, which is the real code path.
///
/// The scratch directory is removed when `body` returns.
pub fn with_no_user_config<F: FnOnce()>(body: F) {
    let empty = TempDir::new("no-user-config");
    let root = empty.path().display().to_string();
    with_env(Some(&root), Some(&root), body);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The restore is a `Drop`, so it has to survive an unwind — the
    /// case it exists for. Written as a test rather than trusted from
    /// the type, because the failure mode is silent: a leaked `HOME`
    /// points at a `TempDir` the same unwind deleted, and every later
    /// test in the process reads it.
    #[test]
    fn test_with_env_restores_the_environment_when_the_body_panics() {
        // Snapshot through the lock, so a concurrently-running
        // `with_env` cannot be observed mid-override.
        let (before_xdg, before_home) = {
            let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            (std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))
        };

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_env(
                Some("/nonexistent-xdg-probe"),
                Some("/nonexistent-home-probe"),
                || {
                    assert_eq!(
                        std::env::var_os("HOME"),
                        Some(std::ffi::OsString::from("/nonexistent-home-probe")),
                        "the override must be in effect before we unwind out of it"
                    );
                    panic!("deliberate: the restore has to run anyway");
                },
            );
        }));
        assert!(panicked.is_err(), "the body must really have panicked");

        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            std::env::var_os("XDG_CONFIG_HOME"),
            before_xdg,
            "XDG_CONFIG_HOME leaked out of a panicking with_env"
        );
        assert_eq!(
            std::env::var_os("HOME"),
            before_home,
            "HOME leaked out of a panicking with_env"
        );
    }
}
