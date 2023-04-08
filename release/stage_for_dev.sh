#!/usr/bin/env bash

set -o errexit -o nounset -o pipefail

# Locate

aspect_cli_brew_artifacts_dev_rb="$PWD/cli/aspect-cli_brew_artifacts_dev.rb"
aspect_cli_brew_artifacts_dev_bottles="$PWD/cli/aspect-cli_brew_artifacts_dev_bottles"

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
    warn "$@"
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
    cat <<- EOF
Builds, stages, and installs locally built Homebrew formula and bottles.

Usage:
${utility} [OPTION]...

Options:
  --help                Show help message.
EOF
}

# Process Args

# Default location for nginx on Macos installed by Homebrew.
default_nginx_www_path="/usr/local/var/www"
bottles_output_basename="bottles"
bottles_output_dir="${default_nginx_www_path}/${bottles_output_basename}"
homebrew_taps_dir="$(brew --repository)/Library/Taps"

formula="aspect"
brew_repo_user="aspect-build"
brew_repo_name="aspect"
brew_repo_url="https://github.com/${brew_repo_user}/homebrew-${brew_repo_name}"
aspect_tap="${brew_repo_user}/${brew_repo_name}"
fully_qualified_formula="${aspect_tap}/${formula}"
aspect_tap_dir="${homebrew_taps_dir}/${brew_repo_user}/homebrew-${brew_repo_name}"
formula_out_path="${aspect_tap_dir}/Formula/${formula}.rb"

while (("$#")); do
    case "${1}" in
        "--help")
            show_usage
            exit 0
            ;;
        *)
            usage_error "Unrecognized argument. ${1}"
            ;;
    esac
done

# Check for external tool dependencies
which > /dev/null brew \
    || fail "Homebrew must be installed to proceed. For instructions, please see https://brew.sh/."
which > /dev/null nginx \
    || fail "nginx must be installed to proceed. Run 'brew install nginx'."

# Copy the bottles to the local web server
mkdir -p "${bottles_output_dir}"
echo "Copy bottles to local web server directory. ${bottles_output_dir}"
cp -fv "${aspect_cli_brew_artifacts_dev_bottles}"/* "${bottles_output_dir}/"

# Uninstall the Aspect CLI, if it is installed
brew list --formula "${fully_qualified_formula}" > /dev/null 2>&1 \
    && echo "Uninstall formula. ${fully_qualified_formula}" \
    && brew uninstall "${fully_qualified_formula}"

# If the tap is not present, add it.
(brew tap | grep "aspect-build/aspect") \
    || (echo "Add tap. ${aspect_tap}" && brew tap "${aspect_tap}" "${brew_repo_url}")

# Copy the formula
echo "Update formula. ${formula_out_path}"
mkdir -p "$(dirname "${formula_out_path}")"
cp -f "${aspect_cli_brew_artifacts_dev_rb}" "${formula_out_path}"

# Uninstalling Bazel & Bazelisk
brew unlink bazelisk > /dev/null 2>&1 || true
brew unlink bazel > /dev/null 2>&1 || true

# Install Aspect CLI
echo "Install formula. ${fully_qualified_formula}"
brew install --verbose "${fully_qualified_formula}"
