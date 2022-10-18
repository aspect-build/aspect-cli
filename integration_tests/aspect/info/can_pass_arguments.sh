#!/usr/bin/env bash

set -o pipefail -o errexit -o nounset
HOME="$TEST_TMPDIR"
mkdir -p "$HOME/.aspect/cli"
touch "$HOME/.aspect/cli/config.yaml"
ASPECT="$TEST_SRCDIR/build_aspect_cli/cmd/aspect/aspect_/aspect"
export HOME
touch WORKSPACE

# Only capture stdout
info=$($ASPECT info bazel-bin --color=no 2> /dev/null) || "$ASPECT" info bazel-bin --color=no

# Should include a path section that contains bazel-out
[[ "$info" =~ "/bazel-out/" ]] || {
    echo >&2 "Expected 'aspect info bazel-bin --color=no' stdout to contain 'bazel-out', but was"
    echo "$info"
    exit 1
}
