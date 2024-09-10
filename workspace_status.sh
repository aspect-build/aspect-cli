#!/usr/bin/env bash
# This script is called by Bazel when it needs info about the git state.
# The --workspace_status_command flag tells Bazel the location of this script.
# This is configured in `/.bazelrc`.
set -o pipefail -o errexit -o nounset

function has_local_changes {
    if [ "$(git status --porcelain)" != "" ]; then
        echo dirty
    else
        echo clean
    fi
}

# "volatile" keys, these will not cause a re-build because they're assumed to change on every build
# and its okay to use a stale value in a stamped binary
echo "BUILD_TIME $(date "+%Y-%m-%d %H:%M:%S %Z")"

# "stable" keys, should remain constant over rebuilds, therefore changed values will cause a
# rebuild of any stamped action that uses ctx.info_file or genrule with stamp = True
# Note, BUILD_USER is automatically available in the stable-status.txt, it matches $USER
echo "STABLE_BUILD_SCM_SHA $(git rev-parse HEAD)"
echo "STABLE_BUILD_SCM_LOCAL_CHANGES $(has_local_changes)"

if [ "$(git tag | wc -l)" -gt 0 ]; then
    # Follows https://blog.aspect.build/versioning-releases-from-a-monorepo
    monorepo_version=$(
        git describe --tags --long --match="[0-9][0-9][0-9][0-9].[0-9][0-9]" |
            sed -e 's/-/./;s/-g/-/'
    )

    # Variant of monorepo_version that conforms with the version scheme Bazelisk supports.
    # It assumes the upstream `bazel` binary releases are the only ones referenced,
    # so we are forced to adopt a matching scheme.
    # https://github.com/bazelbuild/bazelisk/blob/47f60477721681a117cbf905784ee5220100e68b/versions/versions.go#L20-L25
    # shellcheck disable=SC2001
    bazelisk_compat_version=$(sed 's/-.*//' <<<"$monorepo_version")

    echo "STABLE_ASPECT_CLI_BAZELISK_COMPAT_VERSION ${bazelisk_compat_version}"
    echo "STABLE_ASPECT_CLI_VERSION ${monorepo_version}"
fi
