#!/usr/bin/env bash
# Launch the release build of Mandala on both targets at once: the
# native binary in one process, `trunk serve --release` for the
# WASM bundle in another. Ctrl+C stops both. Expects `./build.sh`
# to have produced the native artefact already; trunk rebuilds
# WASM itself.
set -euo pipefail

MAP="${1:-maps/testament.mindmap.json}"
NATIVE_BIN="target/release/mandala"

if [ ! -x "$NATIVE_BIN" ]; then
    echo "Error: $NATIVE_BIN not found or not executable."
    echo "Run ./build.sh first (it builds both native and WASM)."
    exit 1
fi

if ! command -v trunk >/dev/null 2>&1; then
    echo "Error: 'trunk' not found on PATH."
    echo "Install with: cargo install trunk"
    exit 1
fi

echo "Launching:"
echo "  Native: $NATIVE_BIN $MAP"
echo "  WASM:   trunk serve --release   (http://127.0.0.1:8080)"
echo
echo "Ctrl+C to stop both."
echo

# Start WASM serve in the background. Trunk rebuilds + watches; the
# `--release` flag matches the build.sh default so both processes
# run the same optimisation profile.
trunk serve --release &
TRUNK_PID=$!

"$NATIVE_BIN" "$MAP" &
NATIVE_PID=$!

# Clean shutdown: on Ctrl+C (or any exit path), stop both children.
# `|| true` in case one has already exited.
cleanup() {
    echo
    echo "Stopping..."
    kill "$TRUNK_PID" "$NATIVE_PID" 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# Wait for whichever child exits first, then the trap handles the
# other. `wait -n` returns as soon as any single job finishes.
wait -n
