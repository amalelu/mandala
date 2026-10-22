#!/usr/bin/env bash
# Mandala test runner. See TEST_CONVENTIONS.md for the testing philosophy.
set -euo pipefail

export RUST_BACKTRACE=1

COVERAGE=0
LINT=0
BENCH=0
# Set by the hard lint gates (fmt, both clippy legs, both doc legs) on
# failure; checked at the very end of the run so every other gate
# still reports first (#134).
LINT_FAILED=0

usage() {
  cat <<'EOF'
Usage: ./test.sh [--coverage] [--lint] [--bench] [--help]

  (no flags)   Run the full test suite across every workspace member
               (mandala, baumhard, mandala_derive, maptool), then
               type-check the benchmark targets and the WASM target so
               neither can rot silently between merges.
  --coverage   Run the suite under cargo-llvm-cov and emit HTML + LCOV.
  --lint       Also run cargo fmt --check, cargo clippy and cargo doc.
               fmt, both clippy legs and both doc legs (host:
               --workspace with private items; wasm32: -p mandala)
               are hard gates: their baselines are zero (#130, #134
               for fmt and doc; clippy followed once its warnings were
               cleared), so a regression prints a FAILED: line and the
               run exits non-zero at the end, after every other gate
               has reported. clippy runs with -D warnings, because
               cargo clippy exits 0 on warnings. fmt runs once —
               rustfmt parses rather than compiles, so it is
               target-independent.
               clippy and doc run twice, on the host target *and* on
               wasm32-unknown-unknown: a host-target compile drops
               everything under #[cfg(target_arch = "wasm32")], so the
               browser half of the app — its lints and its intra-doc
               links alike — is invisible without the second leg. The
               wasm32 legs are skipped with a note when the target
               isn't installed.
  --bench      Also *run* cargo bench after tests pass. Maintainers
               only — AGENTS.md forbids automated agents this flag.
               The unconditional bench type-check needs no flag.
  --help       Show this message.
EOF
}

for arg in "$@"; do
  case "$arg" in
    --coverage) COVERAGE=1 ;;
    --lint)     LINT=1 ;;
    --bench)    BENCH=1 ;;
    --help|-h)  usage; exit 0 ;;
    *) echo "Unknown flag: $arg"; usage; exit 1 ;;
  esac
done

if [ "$LINT" -eq 1 ]; then
  # `cargo fmt` is target-independent — rustfmt parses source, it does
  # not compile it — so one run covers both targets and there is no
  # wasm32 leg for it. The clippy and rustdoc legs below are the ones
  # that need doubling, because both go through a real compile and a
  # host compile drops everything under #[cfg(target_arch = "wasm32")].
  echo "== fmt (hard) =="
  cargo fmt --all -- --check \
    || { echo "FAILED (exit $?): cargo fmt --all -- --check (#130) — formatting diffs, or rustfmt itself failed"; LINT_FAILED=1; }
  echo "== clippy, host target (hard) =="
  cargo clippy --workspace --all-targets -- -D warnings 2>&1 \
    || { echo "FAILED (exit $?): cargo clippy --workspace --all-targets — this gate holds a zero-warning baseline"; LINT_FAILED=1; }
  # The doc and clippy legs hold a zero-warning baseline, which is
  # what lets them be hard (#134 for doc; clippy's followed once its
  # 152 warnings were cleared). `-D warnings` turns a warning into a
  # failure — for clippy that is the whole mechanism, since `cargo
  # clippy` exits 0 on warnings, so a hard leg that left it out would
  # gate nothing. The handler records the failure and the run exits
  # non-zero at the end, after the remaining gates have reported. An
  # `|| echo` used to sit here instead, and it swallowed a red `cargo
  # doc` for a full review round. The direction rule for links whose target is
  # cfg-stripped (`#[cfg(test)]`, platform gates) lives in
  # CODE_CONVENTIONS.md §8 — rustdoc never compiles the stripped
  # half, so the link breaks from live code and goes unchecked from
  # stripped code.
  #
  # `--workspace`, not a hand-written crate list, for the same reason
  # the test run below is `--workspace`: a list of members is a copy
  # of the `[workspace]` table and copies go stale. The pair of `-p`
  # legs this replaces silently omitted maptool and mandala_derive,
  # and maptool's doc build was red on the very day the gate was
  # hardened — a red no gate could see. `--document-private-items`
  # closes the same shape of hole one level down: private-item links
  # were checked in mandala and nowhere else, and baumhard was
  # carrying seven broken ones.
  echo "== doc, host target (hard) =="
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items 2>&1 \
    || { echo "FAILED (exit $?): cargo doc --workspace, host — this gate holds a zero baseline (#134)"; LINT_FAILED=1; }

  # The host-target runs above compile nothing gated behind
  # #[cfg(target_arch = "wasm32")], so every lint and every broken
  # intra-doc link in src/application/app/run_wasm/ is invisible to
  # them. CODE_CONVENTIONS §4 makes the browser build a peer, not an
  # afterthought, so it gets its own clippy leg *and* its own rustdoc
  # leg. `cargo check --target wasm32-unknown-unknown` (run
  # unconditionally below) is rustc, not clippy and not rustdoc, and
  # substitutes for neither.
  #
  # Guarded on the target being installed, the same way the
  # unconditional wasm32 check below is: without the guard these hard
  # legs would print FAILED on a machine that lacks the target, which
  # reports a missing toolchain as a code problem. rust-toolchain.toml
  # lists the target, so `rustup toolchain install` in this checkout
  # supplies it; the guard is for a machine that has not run that.
  #
  # `--workspace`, so baumhard's cross-platform discipline
  # (lib/baumhard/CONVENTIONS.md) is clippy-checked for wasm and not
  # only rustc-checked. Not `--all-targets`: the test and bench
  # targets pull in criterion and rayon, which refuse to build for
  # wasm32 at all ("Rayon cannot be used when targeting wasi32"), plus
  # maptool's test target needs `baumhard::util::test_temp`, which is
  # native-gated. Test code is clippy-checked on the host leg above,
  # which is where it runs.
  if rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
    echo "== clippy, wasm32-unknown-unknown (hard) =="
    cargo clippy --workspace --target wasm32-unknown-unknown -- -D warnings 2>&1 \
      || { echo "FAILED (exit $?): cargo clippy --workspace, wasm32 — this gate holds a zero-warning baseline"; LINT_FAILED=1; }
    # 11 broken intra-doc links lived here undetected until this leg
    # existed — every one a link to a native-gated item from a
    # cross-platform file, invisible to the host leg by construction.
    # All 11 are fixed (named as plain code-spans, which is the only
    # form that resolves on both targets), so the expected count is 0
    # and this leg is as hard as the host legs above (#134).
    #
    # `-p mandala`, not `--workspace`: baumhard cannot be documented
    # standalone for wasm32 — its `rand` -> `getrandom` edge needs
    # the `wasm_js` feature that only the root manifest declares (see
    # the getrandom note in Cargo.toml) — and maptool and
    # mandala_derive are native-only tools with no wasm32 half to
    # document. mandala is the crate that carries the
    # #[cfg(target_arch = "wasm32")] modules. Stated honestly: if
    # baumhard ever grows a wasm32-gated doc link, no leg checks it.
    echo "== doc, wasm32-unknown-unknown (hard) =="
    RUSTDOCFLAGS="-D warnings" cargo doc -p mandala --no-deps --document-private-items \
      --target wasm32-unknown-unknown 2>&1 \
      || { echo "FAILED (exit $?): cargo doc -p mandala, wasm32 — this gate holds a zero baseline (#134)"; LINT_FAILED=1; }
  else
    echo "== clippy + doc, wasm32-unknown-unknown =="
    echo "(wasm32-unknown-unknown target not installed — skipping. Install with:"
    echo "    rustup target add wasm32-unknown-unknown)"
    echo "(until it is, wasm32-gated code goes unchecked in this run — its clippy"
    echo "lints and its doc links alike)"
  fi
fi

if [ "$COVERAGE" -eq 1 ]; then
  if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "cargo-llvm-cov not found."
    echo "Install with: cargo install cargo-llvm-cov"
    # Checked, not asserted. This line used to state flatly that
    # llvm-tools-preview "is already present via rustup" — true of
    # the machine it was written on and false of a fresh container,
    # where it sent the reader off with one of the two prerequisites
    # silently unmet. A message that tells you the state of your own
    # toolchain has to read it.
    if rustup component list --installed 2>/dev/null | grep -q '^llvm-tools'; then
      echo "(llvm-tools-preview is present via rustup.)"
    else
      echo "It also needs the llvm-tools-preview component, which is not installed:"
      echo "    rustup component add llvm-tools-preview"
    fi
    exit 1
  fi
  echo "== tests with coverage =="
  cargo llvm-cov clean --workspace
  cargo llvm-cov --workspace \
    --ignore-filename-regex '(^|/)(benches|build\.rs|shaders)/' \
    --lcov --output-path target/llvm-cov/lcov.info
  cargo llvm-cov report --workspace \
    --ignore-filename-regex '(^|/)(benches|build\.rs|shaders)/' \
    --html --output-dir target/llvm-cov/html
  cargo llvm-cov report --workspace \
    --ignore-filename-regex '(^|/)(benches|build\.rs|shaders)/' \
    --summary-only
  echo
  echo "HTML report: target/llvm-cov/html/index.html"
  echo "LCOV file:   target/llvm-cov/lcov.info"
else
  echo "== tests =="
  TEST_LOG=$(mktemp)
  trap 'rm -f "$TEST_LOG"' EXIT
  # `--workspace`, not a hand-written list of `-p` flags. The list
  # this replaces named three of the four members and silently
  # dropped every one of `mandala_derive`'s tests for as long as that
  # crate has existed — a list of members is a copy of the
  # `[workspace]` table and copies go stale. So did the count this
  # comment used to give for those tests, which is why it now gives
  # none: the run below prints what it ran, and a second copy written
  # up here can only disagree with it. `--coverage` above and the
  # wasm32 gate below were already `--workspace`; this is the odd one
  # out rejoining them.
  cargo test --workspace 2>&1 | tee "$TEST_LOG"

  # The `|| true` is load-bearing, not defensive noise. Under
  # `set -o pipefail` a `grep` that matches nothing exits 1, the
  # pipeline takes that status, the assignment takes the pipeline's,
  # and `set -e` ends the script there — *after* the suite passed,
  # printing nothing at all. Anything that changes cargo's summary
  # wording (a toolchain bump, a `--format` change, a run whose
  # binaries all report zero tests) turns a green run into a silent
  # non-zero exit that reads as a test failure. The count is a
  # convenience; it must not be able to fail the run.
  TOTAL=$( { grep -E '^test result: ok\. [0-9]+ passed' "$TEST_LOG" || true; } \
    | awk '{ sum += $4 } END { print sum+0 }')
  echo
  if [ "$TOTAL" -gt 0 ]; then
    echo "== $TOTAL tests passed =="
  else
    # Say which half is broken. The suite above already passed — a
    # zero here means the summary lines stopped matching, and a bare
    # "0 tests passed" would read as though nothing ran.
    echo "== tests passed; count unavailable =="
    echo "(no 'test result: ok. <N> passed' lines matched — cargo's summary format"
    echo " has changed, or every binary reported zero tests)"
  fi
fi

if [ "$BENCH" -eq 1 ]; then
  echo "== benches =="
  # Delegate to ./bench.sh rather than repeating its `cargo bench`
  # line. The two used to carry the same invocation with a comment on
  # each asking the reader to keep them in step, which is the shape
  # that drifts. What a benchmark run *is* — which crate has the
  # harness (TEST_CONVENTIONS §T2.3), which flags it takes — is
  # ./bench.sh's to say, in one place.
  #
  # Maintainers only. AGENTS.md forbids automated agents from running
  # benchmarks at all, which is also why the type-check below is
  # unconditional rather than folded in here.
  "$(dirname "$0")/bench.sh"
fi

# Bench-target type-check gate. Unconditional, and deliberately not
# behind --bench: this compiles the benchmark targets without running
# a single benchmark, so it is available to everyone AGENTS.md forbids
# from running one.
#
# It closes a hole this repo would otherwise have. `benches/test_bench.rs`
# imports `do_*()` bodies by path and is not compiled under `cfg(test)`,
# so `cargo test` cannot notice when one is renamed
# (lib/baumhard/CONVENTIONS.md §B8). §B8 names `cargo bench` and
# `./test.sh --bench` as the two mechanisms that would — and AGENTS.md
# forbids both. `autobenches = false` removed even the accidental net of
# cargo compiling a stray `benches/*.rs`. Without this line, a renamed
# `do_*()` breaks the bench file and no green run anywhere reports it.
echo "== bench targets type-check =="
cargo check --workspace --benches

# WASM type-check gate. Native tests can stay green while the WASM leg
# rots silently (see CODE_CONVENTIONS.md §2); this catches shared-helper
# signature drift, cfg-guard mistakes, and missing `wasm-bindgen` usage
# before the next `trunk serve`. Runs across the whole workspace so
# baumhard's cross-platform discipline (`lib/baumhard/CONVENTIONS.md`)
# is also enforced here — a native-only addition to baumhard would
# otherwise fail the eventual `trunk build` without failing the tests.
# `cargo check` is deliberately cheap — full `trunk build` belongs in
# ./build.sh. Skipped with a warning if the wasm32 target isn't
# installed so contributors who haven't run
# `rustup target add wasm32-unknown-unknown` aren't punished.
if rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
  echo "== wasm32 check =="
  cargo check --target wasm32-unknown-unknown --workspace
else
  echo "== wasm32 check =="
  echo "(wasm32-unknown-unknown target not installed — skipping. Install with:"
  echo "    rustup target add wasm32-unknown-unknown)"
fi

# Deferred hard-gate verdict (#134). The fmt, clippy and doc gates
# record failures instead of aborting so a single run reports every gate;
# the recorded verdict lands here, last, where it cannot be mistaken
# for advisory noise scrolling past.
if [ "$LINT_FAILED" -ne 0 ]; then
  echo
  echo "== FAILED: hard lint gates — see the FAILED: lines above =="
  exit 1
fi
