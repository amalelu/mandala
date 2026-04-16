#!/usr/bin/env bash
# Release build script for Mandala. Produces both artifacts — the
# native binary (target/<profile>/mandala) and the WASM bundle
# (dist/) — from a clean state on every invocation, so no stale
# output from a prior build can leak into a release. Per
# CODE_CONVENTIONS.md §2 native and WASM are equal citizens; the
# release tooling builds them together.
set -euo pipefail

DEBUG=false
FAT=false
HELP=false

for arg in "$@"; do
    case "$arg" in
        --debug) DEBUG=true ;;
        --fat)   FAT=true ;;
        --help)  HELP=true ;;
        *)
            echo "Unknown argument: $arg"
            echo "Use --help for usage information."
            exit 1
            ;;
    esac
done

if [ "$HELP" = true ]; then
    cat <<EOF
Usage: ${0##*/} [options]

Builds both the native binary and the WASM bundle after removing
any prior output for the chosen profile. Native and WASM are
always built together; drop to \`cargo build\` / \`trunk build\`
directly if you need one alone.

Options:
  --debug    Build the dev profile (output in target/debug/ and
             an unoptimised WASM bundle in dist/).
  --fat      Build the native binary with the 'release-lto' profile.
             The WASM leg still builds with trunk's --release
             (trunk has no LTO concept to forward).
  --help     Display this help text.
EOF
    exit 0
fi

# Resolve the profile triplet: the user-facing name, the string to
# pass to `cargo --profile` (cargo calls the default dev profile
# "dev" but outputs to target/debug/), and the target directory to
# wipe. Also pick the trunk --release flag for WASM.
if [ "$DEBUG" = true ]; then
    CARGO_PROFILE="dev"
    OUTPUT_DIR="target/debug"
    TRUNK_RELEASE=""
elif [ "$FAT" = true ]; then
    CARGO_PROFILE="release-lto"
    OUTPUT_DIR="target/release-lto"
    TRUNK_RELEASE="--release"
else
    CARGO_PROFILE="release"
    OUTPUT_DIR="target/release"
    TRUNK_RELEASE="--release"
fi

# Preflight the WASM toolchain before cleaning anything — if trunk
# or the wasm32 target isn't installed, fail loudly before we've
# nuked the prior build output.
if ! command -v trunk >/dev/null 2>&1; then
    echo "Error: 'trunk' not found on PATH."
    echo "Install with: cargo install trunk"
    exit 1
fi
if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
    echo "Error: wasm32-unknown-unknown target not installed."
    echo "Install with: rustup target add wasm32-unknown-unknown"
    exit 1
fi

echo "Cleaning prior build output ($OUTPUT_DIR/, dist/)..."
rm -rf "$OUTPUT_DIR" dist

echo "Building native ($CARGO_PROFILE)..."
cargo build --profile "$CARGO_PROFILE"

echo "Building WASM (trunk $TRUNK_RELEASE)..."
# Intentional: $TRUNK_RELEASE is empty in debug mode and the empty
# argument is exactly what trunk expects to select its default.
trunk build $TRUNK_RELEASE

echo
echo "Build complete."
echo "  Native: $OUTPUT_DIR/mandala"
echo "  WASM:   dist/"
