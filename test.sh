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

  (no flags)   Run the full test suite across baumhard + mandala, then
               type-check the WASM target so cross-platform drift fails
               the run instead of sneaking into a merge.
  --coverage   Run the suite under cargo-llvm-cov and emit HTML + LCOV.
  --lint       Also run cargo fmt --check and cargo clippy (advisory, never fails the run).
  --bench      Also run cargo bench after tests pass.
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
  echo "== fmt (advisory) =="
  cargo fmt --all -- --check || echo "(fmt diffs present — not failing the run)"
  echo "== clippy (advisory) =="
  cargo clippy --workspace --all-targets 2>&1 || echo "(clippy issues present — not failing the run)"
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
  cargo test -p baumhard -p mandala -p maptool 2>&1 | tee "$TEST_LOG"

  TOTAL=$(grep -E '^test result: ok\. [0-9]+ passed' "$TEST_LOG" \
    | awk '{ sum += $4 } END { print sum+0 }')
  echo
  echo "== $TOTAL tests passed =="
fi

if [ "$BENCH" -eq 1 ]; then
  echo "== benches =="
  cargo bench -p baumhard -p mandala
fi

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
