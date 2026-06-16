#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

git_commit=$(git rev-parse HEAD)
readonly git_commit

# Monorepo version as semver, e.g. 2025.34.1+201b9a8:
# - major = year (2025), minor = ISO week (34),
# - patch = commits since the week's tag (1), +build = short commit (201b9a8).
# The two --match globs cover single- and double-digit week tags (2025.1-2025.59).
# Follows https://aspect.build/blog/versioning-releases-from-a-monorepo
monorepo_version=$(
    git describe --tags --long --match="2[0-9][0-9][0-9].[1-9]" --match="2[0-9][0-9][0-9].[1-5][0-9]" |
        sed -e 's/-/./;s/-g/+/'
)

# A short variant of the monorepo version that drops the +build metadata. For example, 2025.34.0.
monorepo_short_version=$(sed 's/+.*//' <<<"$monorepo_version")

# Registry-tag-safe variant, e.g. 2025.34.1-201b9a8: registries like AWS ECR
# disallow `+` in tags, so swap it for `-`.
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
