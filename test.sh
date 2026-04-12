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

  (no flags)   Run the full test suite across baumhard + mandala.
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
