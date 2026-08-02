#!/usr/bin/env bash
# Mandala test runner. See TEST_CONVENTIONS.md for the testing philosophy.
set -euo pipefail

export RUST_BACKTRACE=1

COVERAGE=0
LINT=0
BENCH=0

usage() {
  cat <<'EOF'
Usage: ./test.sh [--coverage] [--lint] [--bench] [--help]

  (no flags)   Run the full test suite across every workspace member
               (mandala, baumhard, mandala_derive, maptool), then
               type-check the benchmark targets and the WASM target so
               neither can rot silently between merges.
  --coverage   Run the suite under cargo-llvm-cov and emit HTML + LCOV.
  --lint       Also run cargo fmt --check, cargo clippy and cargo doc
               (advisory, never fails the run). fmt runs once — rustfmt
               parses rather than compiles, so it is target-independent.
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
  echo "== fmt (advisory) =="
  cargo fmt --all -- --check || echo "(fmt diffs present — not failing the run)"
  echo "== clippy, host target (advisory) =="
  cargo clippy --workspace --all-targets 2>&1 || echo "(clippy issues present — not failing the run)"
  # Both doc gates are at zero warnings and are meant to stay there,
  # so `-D warnings` turns a new broken intra-doc link into a visible
  # failure line instead of one more warning scrolling past. Still
  # advisory: `|| echo` keeps --lint from failing the run.
  echo "== doc, host target (advisory) =="
  RUSTDOCFLAGS="-D warnings" cargo doc -p baumhard --no-deps 2>&1 \
    || echo "(doc warnings present — this gate expects 0 — not failing the run)"
  RUSTDOCFLAGS="-D warnings" cargo doc -p mandala --no-deps --document-private-items 2>&1 \
    || echo "(doc warnings present — this gate expects 0 — not failing the run)"

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
  # unconditional wasm32 check below is: without the guard these legs
  # print "issues present" on a machine that has never run
  # `rustup target add`, which reports a missing toolchain as a code
  # problem.
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
    echo "== clippy, wasm32-unknown-unknown (advisory) =="
    cargo clippy --workspace --target wasm32-unknown-unknown 2>&1 \
      || echo "(clippy issues present — not failing the run)"
    # 11 broken intra-doc links lived here undetected until this leg
    # existed — every one a link to a native-gated item from a
    # cross-platform file, invisible to the host leg by construction.
    # All 11 are fixed (named as plain code-spans, which is the only
    # form that resolves on both targets), so the expected count is 0
    # and `-D warnings` keeps it there.
    #
    # `-p mandala`, i.e. the host invocation with only the target
    # changed, rather than `--workspace`: baumhard cannot be
    # documented standalone for wasm32, because its `rand` ->
    # `getrandom` edge needs the `wasm_js` feature that only the root
    # manifest declares (see the getrandom note in Cargo.toml), and
    # `--workspace --document-private-items` additionally surfaces
    # baumhard's private-item links, which the host baumhard gate
    # does not document either. mandala is the crate that carries the
    # #[cfg(target_arch = "wasm32")] modules.
    echo "== doc, wasm32-unknown-unknown (advisory) =="
    RUSTDOCFLAGS="-D warnings" cargo doc -p mandala --no-deps --document-private-items \
      --target wasm32-unknown-unknown 2>&1 \
      || echo "(doc warnings present — this gate expects 0 — not failing the run)"
  else
    echo "== clippy + doc, wasm32-unknown-unknown =="
    echo "(wasm32-unknown-unknown target not installed — skipping. Install with:"
    echo "    rustup target add wasm32-unknown-unknown)"
  fi
fi

if [ "$COVERAGE" -eq 1 ]; then
  if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "cargo-llvm-cov not found."
    echo "Install with: cargo install cargo-llvm-cov"
    echo "(llvm-tools-preview is already present via rustup.)"
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
  # dropped `mandala_derive`'s 13 tests for as long as that crate has
  # existed — a list of members is a copy of the `[workspace]` table
  # and copies go stale. `--coverage` above and the wasm32 gate below
  # were already `--workspace`; this is the odd one out rejoining
  # them.
  cargo test --workspace 2>&1 | tee "$TEST_LOG"

  TOTAL=$(grep -E '^test result: ok\. [0-9]+ passed' "$TEST_LOG" \
    | awk '{ sum += $4 } END { print sum+0 }')
  echo
  echo "== $TOTAL tests passed =="
fi

if [ "$BENCH" -eq 1 ]; then
  echo "== benches =="
  # baumhard is the only crate with a benchmark harness
  # (TEST_CONVENTIONS §T2.3); keep this in step with ./bench.sh.
  #
  # Maintainers only. AGENTS.md forbids automated agents from running
  # benchmarks at all, which is also why the type-check below is
  # unconditional rather than folded in here.
  cargo bench -p baumhard
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
