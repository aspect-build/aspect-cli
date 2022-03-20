#!/bin/bash

set -o errexit -o nounset -o pipefail

# First verify if the repository is clean.
if ! git diff --exit-code; then
    error_msg="ERROR: The repository is not clean - please verify the changes and fix them."
    echo "::error ::${error_msg}"
    exit 1
fi

BAZEL="bazel --bazelrc=.github/workflows/ci.bazelrc --bazelrc=.bazelrc"

# Then, run a series of commands that could produce changes to the source tree.
# For each command, we check if the repository is still clean and proceed.
commands=(
    "${BAZEL} build @go_sdk//:bin/go //:update_go_deps //:gazelle //docs:command_list_update"
    "${BAZEL} run @go_sdk//:bin/go -- fmt \$(go list ./... | grep -v /bazel-/)"
    "${BAZEL} run @go_sdk//:bin/go -- mod tidy"
    "${BAZEL} run //:update_go_deps"
    "${BAZEL} run //:gazelle"
    "${BAZEL} run //docs:command_list_update"
)

for cmd in "${commands[@]}"; do
    /bin/bash -c "${cmd}"
    if ! git diff --exit-code; then
        error_msg="ERROR: Please run '${cmd}' and commit the changes."
        echo "::error ::${error_msg}"
        exit 1
    fi
done
