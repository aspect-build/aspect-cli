#!/usr/bin/env bash

# MARK - Locate Deps

assertions_sh_location=aspect_bazel_lib~/shlib/lib/assertions.sh
assertions_sh="$(rlocation "${assertions_sh_location}")" ||
    (echo >&2 "Failed to locate ${assertions_sh_location}" && exit 1)
# shellcheck source=/dev/null
source "${assertions_sh}"

monterey_bottle_tar_gz_location=_main/bazel/release/brew/tests/monterey_bottle.tar.gz
monterey_bottle_tar_gz="$(rlocation "${monterey_bottle_tar_gz_location}")" ||
    (echo >&2 "Failed to locate ${monterey_bottle_tar_gz_location}" && exit 1)

# MARK - Test

actual_listing="$(tar -tf "${monterey_bottle_tar_gz}")"
expected_listing="$(
    cat <<-EOF
		myapp/
		myapp/1.2.3/
		myapp/1.2.3/README.md
		myapp/1.2.3/bin/
		myapp/1.2.3/bin/goodbye
		myapp/1.2.3/bin/hello
	EOF
)"
assert_equal "${expected_listing}" "${actual_listing}"
