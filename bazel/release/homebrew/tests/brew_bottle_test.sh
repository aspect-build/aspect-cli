#!/usr/bin/env bash

# --- begin runfiles.bash initialization v3 ---
# Copy-pasted from the Bazel Bash runfiles library v3.
set -uo pipefail; set +e; f=bazel_tools/tools/bash/runfiles/runfiles.bash
# shellcheck disable=SC1090
source "${RUNFILES_DIR:-/dev/null}/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "${RUNFILES_MANIFEST_FILE:-/dev/null}" | cut -f2- -d' ')" 2>/dev/null || \
  source "$0.runfiles/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.exe.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  { echo>&2 "ERROR: cannot find $f"; exit 1; }; f=; set -e
# --- end runfiles.bash initialization v3 ---

# MARK - Locate Deps

assertions_sh="$(rlocation "${ASSERTIONS_LIB}")" ||
    (echo >&2 "Failed to locate ${ASSERTIONS_LIB}" && exit 1)
# shellcheck source=/dev/null
source "${assertions_sh}"

monterey_bottle_tar_gz_location=_main/bazel/release/homebrew/tests/monterey_bottle.tar.gz
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
