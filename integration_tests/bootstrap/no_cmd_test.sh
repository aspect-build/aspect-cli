#!/usr/bin/env bash

set -o pipefail -o errexit -o nounset

# Functions

fail() {
  local err_msg="${1:-}"
  [[ -n "${err_msg}" ]] || err_msg="Unspecified error occurred."
  echo >&2 "${err_msg}"
  exit 1
}

make_err_msg() {
  local err_msg="${1}"
  local prefix="${2:-}"
  [[ -z "${prefix}" ]] || \
    local err_msg="${prefix} ${err_msg}"
  echo "${err_msg}"
}

assert_match() {
  local pattern=${1}
  local actual="${2}"
  local err_msg="$(make_err_msg "Expected to match. pattern: ${pattern}, actual: ${actual}" "${3:-}")"
  [[ "${actual}" =~ ${pattern} ]] || fail "${err_msg}"
}

# Variables

bootstrap="$TEST_SRCDIR/build_aspect_cli/cmd/bootstrap/bootstrap_/bootstrap"

# Tests

output=$($bootstrap 2>/dev/null)
assert_match "Hello, World!" "${output}"
