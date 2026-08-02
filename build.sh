#!/usr/bin/env bash
# Release build script for Mandala. Produces both artifacts — the
# native binary (target/<profile>/mandala) and the WASM bundle
# (dist/) — from a clean state on every invocation, so no stale
# output from a prior build can leak into a release. "Clean" here
# means the previous output is set aside and discarded only once its
# replacement exists: a failed build leaves the prior artifacts in
# place rather than destroying them (see `stash_output` below). Per
# CODE_CONVENTIONS.md §4 native and WASM are equal citizens; the
# release tooling builds them together.
set -euo pipefail

DEBUG=false
FAT=false
HELP=false
NO_KEEP=false

for arg in "$@"; do
    case "$arg" in
        --debug)   DEBUG=true ;;
        --fat)     FAT=true ;;
        --no-keep) NO_KEEP=true ;;
        --help)    HELP=true ;;
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

Builds both the native binary and the WASM bundle, replacing any
prior output for the chosen profile. Prior output is discarded
only after its replacement has been built, so a failing build
leaves the artifacts you already had intact — at the cost of
holding a second copy of the output tree for the duration, which
roughly doubles peak disk usage. Native and WASM are always built
together; drop to \`cargo build\` / \`trunk build\` directly if you
need one alone.

Options:
  --debug    Build the dev profile (output in target/debug/ and
             an unoptimized WASM bundle in dist/).
  --fat      Build the native binary with the 'release-lto' profile
             (whole-graph LTO, one codegen unit — slower to build,
             faster to run). The WASM leg still builds with trunk's
             --release (trunk has no LTO concept to forward).
  --no-keep  Delete the prior output before building instead of
             setting it aside. Frees its space up front; a failed
             build then leaves you with no artifacts at all.
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

# Preflight the WASM toolchain before either leg starts — if trunk or
# the wasm32 target isn't installed, fail loudly before the native
# build has spent several minutes earning an output directory the WASM
# leg was never going to be able to join.
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

# Outputs are replaced, never preemptively destroyed. `rm -rf`ing the
# output directories before cargo runs means any build failure — an
# undefined `--profile`, a compile error, a full disk — leaves the
# user with neither the new artifacts nor the ones they already had.
# Each output directory is instead renamed aside immediately before
# the step that rebuilds it and deleted only once that step has
# succeeded; a failure restores the previous artifacts. That covers
# every way a build can fail rather than the one preflighting the
# profile would.
#
# What it costs, stated plainly because one of the failures above is
# "a full disk": the moves themselves are same-filesystem renames and
# so are constant-time, but the predecessor is *retained* for the
# whole build instead of being freed before cargo starts. Peak disk is
# therefore roughly double the size of the output tree — and
# target/<profile>/ runs to multiple gigabytes here. On a tight disk
# that makes the very failure this protects against more likely, so
# `stash_output` warns when the free space does not cover the copy,
# and `--no-keep` restores the old destroy-first behavior for anyone
# who would rather have the space than the safety net.
STASHED=()

# Rename $1 aside, recording it as restorable. A no-op if it does not
# exist, so a first-ever build stashes nothing and restores nothing.
# Under --no-keep the directory is deleted outright, freeing its space
# before cargo runs and giving up the ability to restore it.
stash_output() {
    local dir="$1"
    [ -e "$dir" ] || return 0
    if [ "$NO_KEEP" = true ]; then
        rm -rf "$dir"
        return 0
    fi
    local used avail
    used=$(du -sk "$dir" 2>/dev/null | awk '{print $1}')
    avail=$(df -Pk "$(dirname "$dir")" 2>/dev/null | awk 'NR==2 {print $4}')
    if [ -n "$used" ] && [ -n "$avail" ] && [ "$avail" -lt "$used" ]; then
        echo "Warning: keeping the previous $dir/ needs ~$((used / 1024)) MB of headroom"
        echo "         and only $((avail / 1024)) MB is free. The build may run out of"
        echo "         disk. Re-run with --no-keep to delete it up front instead."
    fi
    rm -rf "$dir.prev"
    mv "$dir" "$dir.prev"
    STASHED+=("$dir")
}

# Put back everything still stashed. Registered as the EXIT trap, so
# it runs on `set -e` aborts and on Ctrl+C alike; after a successful
# step `commit_stashed` has emptied the list and this does nothing.
#
# `${STASHED[@]+"${STASHED[@]}"}` rather than a `${#STASHED[@]}` guard:
# bash before 4.4 — macOS still ships 3.2, and the shebang is `env
# bash` — treats an empty array as unset, so under `set -u` the length
# of an empty STASHED is an error. That is reachable on any first-ever
# build, where nothing was stashed.
restore_stashed() {
    local dir
    for dir in ${STASHED[@]+"${STASHED[@]}"}; do
        echo "Build did not complete — restoring previous $dir/"
        rm -rf "$dir"
        mv "$dir.prev" "$dir"
    done
    STASHED=()
}
trap restore_stashed EXIT

# Accept the newly built output: discard the stashed predecessor and
# stop treating it as restorable.
commit_stashed() {
    local dir
    for dir in ${STASHED[@]+"${STASHED[@]}"}; do
        rm -rf "$dir.prev"
    done
    STASHED=()
}

echo "Building native ($CARGO_PROFILE)..."
stash_output "$OUTPUT_DIR"
cargo build --profile "$CARGO_PROFILE"
commit_stashed

echo "Building WASM (trunk $TRUNK_RELEASE)..."
stash_output dist
# Intentional: $TRUNK_RELEASE is empty in debug mode and the empty
# argument is exactly what trunk expects to select its default.
trunk build $TRUNK_RELEASE
commit_stashed

echo
echo "Build complete."
echo "  Native: $OUTPUT_DIR/mandala"
echo "  WASM:   dist/"
