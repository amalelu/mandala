#!/bin/bash
# This script builds codebound properly for release
# Any files that should be included in a release-build will have to be excluded here
EXECUTABLE_NAME="mandala"
# Define the common exclusion patterns
EXCLUDE_PATTERN="! -name '${EXECUTABLE_NAME}' ! -name '${EXECUTABLE_NAME}.exe' ! -name 'build.log' ! -name '*.so' ! -name '*.a' ! -name '*.lib'"

echo $EXCLUDE_PATTERN
# Default variables
BUILD_DIR="target"
PROFILE="release"
HELP=false

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
    echo "  --help          Display this help text."
}

# Check for help flag
if [ "$HELP" = true ]; then
    display_help
    exit 0
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
