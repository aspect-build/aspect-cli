#!/bin/bash

set -o errexit -o nounset -o pipefail

# First verify if the repository is clean.
if ! git diff --exit-code; then
    >&2 echo "ERROR: The repository is not clean - please verify the changes and fix them."
    exit 1
fi

# Then, run a series of commands that could produce changes to the source tree.
# For each command, we check if the repository is still clean and proceed.
commands=(
    "bazel run @go_sdk//:bin/go -- fmt \$(go list ./... | grep -v /bazel-/)"
    "bazel run @go_sdk//:bin/go -- mod tidy"
    "bazel run //:update_go_deps"
    "bazel run //:gazelle"
)

for cmd in "${commands[@]}"; do
    /bin/bash -c "${cmd}"
    if ! git diff --exit-code; then
        >&2 echo "ERROR: Please run '${cmd}' and commit the changes."
        exit 1
    fi
done
