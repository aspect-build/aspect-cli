#!/usr/bin/env bash

# --- begin runfiles.bash initialization v2 ---
# Copy-pasted from the Bazel Bash runfiles library v2.
set -o nounset -o pipefail; f=bazel_tools/tools/bash/runfiles/runfiles.bash
# shellcheck disable=SC1090
source "${RUNFILES_DIR:-/dev/null}/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "${RUNFILES_MANIFEST_FILE:-/dev/null}" | cut -f2- -d' ')" 2>/dev/null || \
  source "$0.runfiles/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.exe.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  { echo>&2 "ERROR: cannot find $f"; exit 1; }; f=; set -o errexit
# --- end runfiles.bash initialization v2 ---

# Locate Deps

assertions_sh_location=aspect_bazel_lib/shlib/lib/assertions.sh
assertions_sh="$(rlocation "${assertions_sh_location}")" || \
  (echo >&2 "Failed to locate ${assertions_sh_location}" && exit 1)
source "${assertions_sh}"

install_location=build_aspect_cli/install/install
install="$(rlocation "${install_location}")" || \
  (echo >&2 "Failed to locate ${install_location}" && exit 1)

aspect_location=build_aspect_cli/cmd/aspect/aspect_/aspect
aspect="$(rlocation "${aspect_location}")" || \
  (echo >&2 "Failed to locate ${aspect_location}" && exit 1)


# Functions

assert_file_exists() {
  local target="${1}"
  [[ -e "${target}" ]] || fail "File does not exist: ${target}"
}

assert_same_contents() {
  local first="${1}"
  local second="${2}"
  diff "${first}" "${second}" || fail "Contents for ${first} and ${second} differ."
}

# Test

bin_dir="${PWD}/bin"
expected_dest_path="${bin_dir}/aspect"

setup_test() {
  rm -rf "${bin_dir}"
  mkdir -p "${bin_dir}"
}

# Install the binary
setup_test
output="$("${install}" --bin "${bin_dir}")"
assert_file_exists "${expected_dest_path}"
assert_same_contents "${aspect}" "${expected_dest_path}"
assert_match "Aspect CLI installed: ${expected_dest_path}" "${output}"
