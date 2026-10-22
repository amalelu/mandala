#!/usr/bin/env bash
# Dev-profile build of both targets — the shorthand for
# `./build.sh --debug`, kept because it is the build run most often
# while iterating and the flag is easy to forget.
#
# Every other flag forwards, so `./debug_build.sh --no-keep` works
# the way `./build.sh --debug --no-keep` does.
set -euo pipefail

exec "$(dirname "$0")/build.sh" --debug "$@"
