// SPDX-License-Identifier: MPL-2.0

//! Collision-free scratch paths for tests that touch the filesystem.
//!
//! Tests that wrote to a *fixed* name under [`std::env::temp_dir`] —
//! `/tmp/mandala_determinism_a.mindmap.json` and friends — share that
//! path with every other process on the machine. Two `cargo test`
//! runs in parallel (two checkouts, or a test run beside a CI job on
//! the same host) race on the same file: one truncates it while the
//! other reads, and the reader fails with a diff nobody can reproduce
//! afterwards. The failures look like flaky assertions in the code
//! under test, which is the expensive kind of wrong.
//!
//! [`TempDir`](crate::util::test_temp::TempDir) hands each caller its
//! own directory, keyed by process id plus a per-process counter, and
//! removes it on drop. Nothing is shared, so nothing races, and a
//! panicking test leaves one identifiable directory behind rather
//! than corrupting the next run.
//!
//! Native-only: the whole module is `cfg`-gated away on `wasm32`,
//! where there is no filesystem to scratch on. Browser-side tests
//! that need a fixture should build it in memory.

// The `TempDir` link in the header names its full path deliberately.
// `util` carries an outer `///` summary on `pub mod test_temp;`, and
// rustdoc merges that with this inner header before resolving links —
// in the *parent* module's scope, where a bare `[`TempDir`]` binds to
// nothing and rustdoc warns.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-process counter. Combined with the pid this makes every
/// directory unique both across concurrent processes (differing pids)
/// and within one process (differing counter values), without
/// consulting the clock — `SystemTime` is unavailable on some targets
/// and would make the name non-reproducible in logs.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// An owned scratch directory that deletes itself on drop.
///
/// Purpose: give a filesystem test a private directory so concurrent
/// runs cannot collide.
/// Inputs: a `label`, used verbatim in the directory name so a leaked
/// directory names the test that leaked it.
/// Costs: one `create_dir_all` on construction and one
/// `remove_dir_all` on drop; no allocation beyond the path itself.
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates `<temp_dir>/mandala-test-<label>-<pid>-<n>/`.
    ///
    /// Panics if the directory cannot be created — a test that cannot
    /// obtain scratch space has nothing meaningful to report, and
    /// failing loudly here beats failing obscurely three lines later.
    pub fn new(label: &str) -> Self {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mandala-test-{label}-{pid}-{n}",
            pid = std::process::id()
        ));
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("could not create scratch dir {}: {e}", path.display()));
        Self { path }
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A path to `name` inside this directory. The file is not
    /// created; callers write it themselves.
    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best-effort: a test that already failed should report its
        // own assertion, not a cleanup error on top of it. The
        // directory name identifies the owner if one is left behind.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two `TempDir`s alive at once never share a path — the property
    /// the fixed-name pattern violated.
    #[test]
    fn test_temp_dirs_are_unique_within_a_process() {
        let a = TempDir::new("unique");
        let b = TempDir::new("unique");
        assert_ne!(a.path(), b.path(), "concurrent scratch dirs must differ");
        assert!(a.path().is_dir(), "scratch dir a must exist");
        assert!(b.path().is_dir(), "scratch dir b must exist");
    }

    /// The directory and its contents are gone once the guard drops.
    #[test]
    fn test_temp_dir_removes_itself_and_its_contents_on_drop() {
        let path;
        {
            let dir = TempDir::new("cleanup");
            path = dir.path().to_path_buf();
            std::fs::write(dir.join("payload.json"), b"{}").expect("write payload");
            assert!(path.join("payload.json").exists(), "payload must be written");
        }
        assert!(!path.exists(), "scratch dir must be removed on drop");
    }

    /// The label survives into the name so a leaked directory is
    /// traceable to the test that leaked it.
    #[test]
    fn test_temp_dir_name_carries_the_label_and_pid() {
        let dir = TempDir::new("labeled-probe");
        let name = dir.path().file_name().unwrap().to_string_lossy().to_string();
        assert!(name.contains("labeled-probe"), "name must carry the label: {name}");
        assert!(
            name.contains(&std::process::id().to_string()),
            "name must carry the pid: {name}"
        );
    }
}
