// SPDX-License-Identifier: MPL-2.0

//! Runs baumhard's `FONT_SYSTEM` helpers on `wasm32-unknown-unknown`,
//! in a real WebAssembly engine (node), and fails if they trap.
//!
//! **Why this test exists, and why it is an exception.**
//! `TEST_CONVENTIONS.md` §T9 trusts the wasm32 *compile* in `test.sh`
//! to validate wasm-specific paths, and that holds for everything a
//! compiler can see. It could not see this: `acquire_font_system_write`
//! called `std::time::Instant::now()` before its first attempt, which
//! compiles for wasm32 and traps the first time it runs there — in the
//! browser, at font initialization, with no message. Only executing the
//! code finds that, so this one test executes it.
//!
//! It is deliberately narrow. It is not `wasm-bindgen-test`, there is
//! no browser, and it touches no DOM or GPU: it loads a `cdylib` that
//! calls the lock helpers and checks two things the browser depends on.
//! The uncontended path works and reads no clock, and every same-thread
//! re-entrancy case panics with its own message — rather than hanging,
//! or trapping for some unrelated reason.
//!
//! **What it needs.** The `wasm32-unknown-unknown` target and `node`.
//! Without either, the test prints a note and passes, the same
//! conditional-locally, unconditional-in-CI arrangement as `test.sh`'s
//! wasm32 gate. Set `MANDALA_REQUIRE_WASM_PROBE` (CI does) to turn a
//! missing prerequisite into a failure instead.
//!
//! **How it builds.** `probe.rs` is copied into a staging crate under
//! `target/`, together with the workspace's `Cargo.lock`, and built for
//! wasm32 there. That compiles the dependency graph the app ships and
//! needs no second lockfile in the repository to go stale. The first
//! run compiles baumhard for wasm32 from scratch, which takes a couple
//! of minutes; later runs are incremental.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Set to make a missing prerequisite a failure rather than a skip.
const REQUIRE_ENV: &str = "MANDALA_REQUIRE_WASM_PROBE";
const TARGET: &str = "wasm32-unknown-unknown";
const PROBE_CRATE: &str = "wasm_font_lock_probe";

/// Environment the probe's nested `cargo build` must not inherit from
/// this test process; see where it is removed.
const INHERITED_HOST_FLAGS: &[&str] = &[
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "LLVM_PROFILE_FILE",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above lib/baumhard")
}

/// Where staging happens: beside the workspace's own build output, so
/// `cargo clean` removes it and it is already git-ignored.
fn staging_dir(root: &Path) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    target.join("wasm_font_lock")
}

/// A missing prerequisite: skip with a note, or fail when CI demands
/// that the test really run.
fn unavailable(what: &str) {
    if std::env::var_os(REQUIRE_ENV).is_some() {
        panic!("{what}, and {REQUIRE_ENV} is set, so this test may not skip");
    }
    eprintln!(
        "SKIPPED: {what}. The wasm32 font-lock probe did not run, so a green result \
         here says nothing about the browser build. (Set {REQUIRE_ENV} to make this a failure.)"
    );
}

fn wasm_target_installed() -> bool {
    Command::new("rustc")
        .args(["--print", "target-libdir", "--target", TARGET])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
        .is_some_and(|dir| dir.is_dir())
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Rewrite `path` only when its content changes, so an unchanged file
/// keeps its mtime and cargo has no reason to rebuild.
fn write_if_changed(path: &Path, content: &[u8]) {
    if fs::read(path).ok().as_deref() != Some(content) {
        fs::write(path, content).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
    }
}

#[test]
fn test_font_lock_helpers_run_on_wasm32() {
    if !wasm_target_installed() {
        return unavailable(&format!(
            "the {TARGET} target is not installed (rustup target add {TARGET})"
        ));
    }
    if !node_available() {
        return unavailable("`node` is not on PATH");
    }

    let root = workspace_root();
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wasm_font_lock");
    let stage = staging_dir(&root);
    fs::create_dir_all(&stage).expect("the staging directory must be creatable");

    // The staging crate. `[workspace]` keeps cargo from adopting it
    // into the real workspace or demanding that it be listed there.
    // `getrandom`'s `wasm_js` feature is the same ask the root
    // manifest makes: without it baumhard's `rand` edge has no backend
    // on wasm32 and the build fails.
    let manifest = format!(
        "[package]\n\
         name = \"{PROBE_CRATE}\"\n\
         version = \"0.0.0\"\n\
         edition = \"2021\"\n\
         \n\
         [lib]\n\
         path = \"probe.rs\"\n\
         crate-type = [\"cdylib\"]\n\
         \n\
         [dependencies]\n\
         baumhard = {{ path = {baumhard:?} }}\n\
         getrandom = {{ version = \"0.4.2\", features = [\"wasm_js\"] }}\n\
         \n\
         [workspace]\n",
        baumhard = root.join("lib/baumhard").display().to_string(),
    );
    write_if_changed(&stage.join("Cargo.toml"), manifest.as_bytes());
    write_if_changed(
        &stage.join("probe.rs"),
        &fs::read(here.join("probe.rs")).expect("probe.rs ships beside this test"),
    );
    // The app's lockfile, so the probe resolves the app's versions.
    // Cargo adds the probe's own package and prunes what it does not
    // use; the copy is refreshed each run for that reason.
    fs::copy(root.join("Cargo.lock"), stage.join("Cargo.lock"))
        .expect("the workspace Cargo.lock must be readable");

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut build = Command::new(&cargo);
    // The flags this test process was built with are the *host's*, and
    // a nested cargo would otherwise inherit them for a wasm32 build:
    // `cargo llvm-cov` (`./test.sh --coverage`) injects
    // `-C instrument-coverage`, for which wasm32 has no profiler
    // runtime, and a `-C target-cpu=native` names a CPU wasm32 has not
    // got. The probe is meant to build the way the browser bundle
    // does, so it gets none of them.
    for var in INHERITED_HOST_FLAGS {
        build.env_remove(var);
    }
    let build = build
        .args(["build", "--target", TARGET, "--manifest-path"])
        .arg(stage.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(stage.join("target"))
        .output()
        .expect("cargo must be runnable");
    assert!(
        build.status.success(),
        "building the wasm32 probe failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let wasm = stage.join(format!("target/{TARGET}/debug/{PROBE_CRATE}.wasm"));
    assert!(
        wasm.is_file(),
        "the build succeeded but {} is missing",
        wasm.display()
    );

    let run = Command::new("node")
        .arg(here.join("runner.mjs"))
        .arg(&wasm)
        .output()
        .expect("node must be runnable");
    let report = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "the font-lock helpers misbehave on {TARGET}:\n{report}{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
