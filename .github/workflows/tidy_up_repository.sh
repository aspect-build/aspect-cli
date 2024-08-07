#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

# First verify if the repository is clean.
if ! git diff --exit-code; then
    error_msg="ERROR: The repository is not clean - please verify the changes and fix them."
    echo "::error ::${error_msg}"
    exit 1
fi

# Then, run a series of commands that could produce changes to the source tree.
# For each command, we check if the repository is still clean and proceed.
commands=(
    "bazel run @go_sdk//:bin/go -- mod tidy"
    "bazel run //:gazelle_update_repos"
    "bazel run //:gazelle"
    "bazel run //docs:command_list_update"
)

for cmd in "${commands[@]}"; do
    echo "+ ${cmd}"
    /bin/bash -c "${cmd}"
    if ! git diff --exit-code; then
        error_msg="ERROR: Please run '${cmd}' and commit the changes."
        echo "::error ::${error_msg}"
        exit 1
    fi
done
