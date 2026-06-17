#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

git_commit=$(git rev-parse HEAD)
readonly git_commit

# Monorepo version as semver, e.g. 2025.34.1+201b9a8:
# - major = year (2025), minor = ISO week (34),
# - patch = commits since the week's tag (1), +build = short commit (201b9a8).
# The two --match globs cover single- and double-digit week tags (2025.1-2025.59).
# Follows https://aspect.build/blog/versioning-releases-from-a-monorepo
#
# When no matching version tag is reachable — a shallow CI checkout that didn't
# fetch tags, a fork, or a fresh clone — `git describe` exits 128. Fall back to
# `<year>.<isoweek>.0+<short_commit>` (patch 0: no commits since a known tag) so
# stamping and delivery still succeed instead of failing the whole build.
if monorepo_version=$(
    git describe --tags --long \
        --match="2[0-9][0-9][0-9].[1-9]" \
        --match="2[0-9][0-9][0-9].[1-5][0-9]" 2>/dev/null
); then
    monorepo_version=$(sed -e 's/-/./;s/-g/+/' <<<"$monorepo_version")
else
    year=$(git show -s --format=%cd --date=format:%Y HEAD)
    # Force base-10 so a zero-padded ISO week (e.g. 08, 09) isn't read as octal.
    week=$((10#$(git show -s --format=%cd --date=format:%V HEAD)))
    monorepo_version="${year}.${week}.0+${git_commit:0:8}"
fi

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
