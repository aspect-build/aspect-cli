#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

git_commit=$(git rev-parse HEAD)
readonly git_commit

# Monorepo version. For example, 2025.34.0+201b9a8.
# Follows https://blog.aspect.build/versioning-releases-from-a-monorepo
monorepo_version=$(
    git describe --tags --long --match="[0-9][0-9][0-9][0-9].[0-9][0-9]" |
        sed -e 's/-/./;s/-g/+/'
)

# A short variant of the monorepo version. For example, 2025.34.0.
# The short version conforms with the version scheme Bazelisk supports.
# It assumes the upstream `bazel` binary releases are the only ones referenced,
# so we are forced to adopt a matching scheme.
# https://github.com/bazelbuild/bazelisk/blob/47f60477721681a117cbf905784ee5220100e68b/versions/versions.go#L20-L25
monorepo_short_version=$(sed 's/+.*//' <<<"$monorepo_version")

# Image repository compatible monrepo version. For example, 2025.34.0-201b9a8.
# AWS ECR does not allow `+` characters in tags so we swap with `-`.
monorepo_image_tag_version="${monorepo_version//+/-}"

function has_local_changes {
    if [ "$(git status --porcelain)" != "" ]; then
        echo dirty
    else
        echo clean
    fi
}

cat <<EOF
STABLE_BUILD_SCM_LOCAL_CHANGES $(has_local_changes)
STABLE_BUILD_SCM_SHA ${git_commit}
STABLE_GIT_COMMIT ${git_commit}
STABLE_GIT_SHORT_COMMIT ${git_commit:0:8}
STABLE_MONOREPO_VERSION ${monorepo_version}
STABLE_MONOREPO_SHORT_VERSION ${monorepo_short_version}
STABLE_MONOREPO_IMAGE_TAG_VERSION ${monorepo_image_tag_version}
EOF
