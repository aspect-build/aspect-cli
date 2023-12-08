#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

# To add a new go dependency, make the required changes to the go files (import and use) and then
# run this file.

cd "${BUILD_WORKSPACE_DIRECTORY}"

bazel run @go_sdk//:bin/go -- mod tidy
bazel run //:gazelle_update_repos
bazel run //:gazelle

if [ "$(git status --porcelain | wc -l)" -gt 0 ]; then
	echo >&2 "ERROR: files changed, commit them"
	git >&2 diff
	exit 1
fi
