#!/usr/bin/env bash

# --- begin runfiles.bash initialization v2 ---
# Copy-pasted from the Bazel Bash runfiles library v2.
set -o nounset -o pipefail; f=bazel_tools/tools/bash/runfiles/runfiles.bash
source "${RUNFILES_DIR:-/dev/null}/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "${RUNFILES_MANIFEST_FILE:-/dev/null}" | cut -f2- -d' ')" 2>/dev/null || \
  source "$0.runfiles/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.exe.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  { echo>&2 "ERROR: cannot find $f"; exit 1; }; f=; set -o errexit
# --- end runfiles.bash initialization v2 ---

# Locate Resources

aspect_location=build_aspect_cli/cmd/aspect/aspect_/aspect
aspect="$(rlocation "${aspect_location}")" || \
  (echo >&2 "Failed to locate ${aspect_location}" && exit 1)

# Functions

warn() {
  local msg="${1:-}"
  shift 1
  while (("$#")); do
    msg="${msg:-}"$'\n'"${1}"
    shift 1
  done
  echo >&2 "${msg}"
}

# Echos the provided message to stderr and exits with an error (1).
fail() {
  warn "${1:-}"
  exit 1
}

# Print an error message and dump the usage/help for the utility.
# This function expects a get_usage function to be defined.
usage_error() {
  local msg="${1:-}"
  cmd=(fail)
  [[ -z "${msg:-}" ]] || cmd+=("${msg}" "")
  cmd+=("$(get_usage)")
  "${cmd[@]}"
}

show_usage() {
  get_usage
  exit 0
}

get_usage() {
  local utility
  utility="$(basename "${BASH_SOURCE[0]}")"
  cat <<-EOF
Install the Aspect CLI on the system.

Usage:
${utility} [OPTION]

Options:
  --help                 Show usage.
  --bin <bin_dir>        The directory where the binary should be installed.
EOF
}

# Process Arguments

bin_dir=/usr/local/bin
args=()
while (("$#")); do
  case "${1}" in
    "--help")
      show_usage
      exit 0
      ;;
    "--bin")
      bin_dir="${2}"
      shift 2
      ;;
    *)
      args+=("${1}")
      shift 1
      ;;
  esac
done

if [[ ! -d "${bin_dir}" ]]; then
  usage_error "The installation directory was not found. ${bin_dir}"
fi

dest_path="${bin_dir}/aspect"

# Remove an existing file
if [[ -e "${dest_path}" ]]; then
  warn "Removing existing file ${dest_path}."
  rm "${dest_path}"
fi

# Copy the binary
cp "${aspect}" "${dest_path}"

echo "Aspect CLI installed: ${dest_path}"
