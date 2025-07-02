#!/usr/bin/env bash
set -o errexit -o nounset -o pipefail

# Determine the script path
SCRIPTPATH="$(cd -- "$(dirname "$0")" >/dev/null 2>&1 || exit; pwd -P)"
if [ -z "$SCRIPTPATH" ]; then echo "Error: Could not determine script path"; exit 1; fi

# Run the workspace_status script and capture its output
output=$("${SCRIPTPATH}/../bazel/workspace_status.sh")

# Extract the STABLE_MONOREPO_SHORT_VERSION value using grep and awk
version=$(echo "$output" | grep '^STABLE_MONOREPO_SHORT_VERSION ' | awk '{print $2}')

# Check if the version was found (optional, but good practice to avoid errors)
if [ -z "$version" ]; then
  echo "Error: STABLE_MONOREPO_SHORT_VERSION not found in output."
  exit 1
fi

# Push the release tag
set -x
git tag "$version"
git push origin "$version"
