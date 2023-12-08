#!/usr/bin/env bash

# MARK - Locate Deps

assertions_sh_location=aspect_bazel_lib/shlib/lib/assertions.sh
assertions_sh="$(rlocation "${assertions_sh_location}")" ||
	(echo >&2 "Failed to locate ${assertions_sh_location}" && exit 1)
# shellcheck source=/dev/null
source "${assertions_sh}"

stage_for_dev_sh_location=build_aspect_cli/release/stage_for_dev.sh
stage_for_dev_sh="$(rlocation "${stage_for_dev_sh_location}")" ||
	(echo >&2 "Failed to locate ${stage_for_dev_sh_location}" && exit 1)

aspect_version_location=build_aspect_cli/release/aspect_version.version
aspect_version="$(rlocation "${aspect_version_location}")" ||
	(echo >&2 "Failed to locate ${aspect_version_location}" && exit 1)

# MARK - Test

expected_version="$(<"${aspect_version}")"

# Copy the bottles and formula to the correct spots
"${stage_for_dev_sh}"

# Confirm that
output="$(aspect --version 2>/dev/null)"

# If the expected version is the placeholder (i.e., --stamp was not specified),
# then expect the unstamped version message.
if [[ "${expected_version}" == "0.0.0-VERSION-PLACEHOLDER" ]]; then
	expected_version="unknown \[not built with --stamp\]"
fi

assert_match "aspect ${expected_version}" "${output}"
