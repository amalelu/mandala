#!/bin/bash
# This script builds mandala properly for release
# Any files that should be included in a release-build will have to be excluded here
EXECUTABLE_NAME="mandala"
# Define the common exclusion patterns
EXCLUDE_PATTERN="! -name '${EXECUTABLE_NAME}' ! -name '${EXECUTABLE_NAME}.exe' ! -name 'build.log' ! -name '*.so' ! -name '*.a' ! -name '*.lib'"

echo $EXCLUDE_PATTERN
# Default variables
BUILD_DIR="target"
PROFILE="release"
HELP=false
WASM=false

# Parse parameters
for arg in "$@"; do
    case $arg in
        --dir=*)
        BUILD_DIR="${arg#*=}"
        shift
        ;;
        --debug)
	PROFILE="debug"
	shift
	;;
        --fat)
        PROFILE="release-lto"
        shift
        ;;
        --wasm)
        WASM=true
        shift
        ;;
        --help)
        HELP=true
        shift
        ;;
        *)
        echo "Unknown argument: $arg"
        echo "Use --help for usage information."
        exit 1
        ;;
    esac
done

# Function to display help text
display_help() {
    echo "Usage: ${0##*/} [options]"
    echo ""
    echo "Options:"
    echo "  --dir=<path>    Specify the build directory. Default is 'target'."
    echo "  --fat           Build using the 'release-lto' profile."
    echo "  --wasm          Build the WASM target via trunk instead of the native binary."
    echo "                  Respects --debug (dev build) vs. default (release build)."
    echo "  --help          Display this help text."
}

# Check for help flag
if [ "$HELP" = true ]; then
    display_help
    exit 0
fi

# WASM build path. Delegates to `trunk`, which reads Trunk.toml and
# emits to `dist/`. Per CODE_CONVENTIONS.md §8, cross-platform changes
# are expected to verify here before landing; making the flag first-class
# means the check is one command, not a recipe to remember.
if [ "$WASM" = true ]; then
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
    echo "Building WASM target via trunk..."
    if [ "$PROFILE" = "debug" ]; then
        trunk build
    else
        trunk build --release
    fi
    exit $?
fi

# Create the build directory
mkdir -p "$BUILD_DIR"
if [ $? -ne 0 ]; then
    echo "Error: Could not create build directory '$BUILD_DIR'."
    exit 1
fi

echo "Building project..."
echo "Build directory: $BUILD_DIR"
echo "Profile: $PROFILE"
TARGET_DIR="$BUILD_DIR/$PROFILE"
BUILD_LOG="$TARGET_DIR/build.log"
mkdir -p "$TARGET_DIR"
echo "Outputting build log to $BUILD_LOG"

# Build the project
echo "Building, please wait.."
cargo build --profile "$PROFILE" --target-dir "$BUILD_DIR" &> "$BUILD_LOG"
echo "Building complete."

if [ $? -ne 0 ]; then
    echo "Error: Cargo build failed."
    exit 1
fi

echo "Cleaning directory: $TARGET_DIR"

# First, find and delete all files within the target directory
#eval "find \"$TARGET_DIR\" -mindepth 1 -type f \( $EXCLUDE_PATTERN \) -exec rm -f {} +"
# eval: It takes the command string with variable substitutions and executes it as a shell command.
# Next, find and delete all directories within the target directory
#eval "find \"$TARGET_DIR\" -mindepth 1 -type d \( $EXCLUDE_PATTERN \) -exec rm -rf {} +"

exit 0
