#!/usr/bin/env bash

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

get_usage() {
	local utility
	utility="$(basename "${BASH_SOURCE[0]}")"
	cat <<-EOF
		Generate a Homebrew formula from the provided parameters.

		Usage:
		${utility} [OPTION]... 

		Required:
		  --desc <description>        The description that is included in the formula.
		  --formula <formula>         The name of the formula (e.g., 'foo-bar').
		  --homepage <url>            The webpage that is exposed by Homebrew.
		  --url <url>                 The URL for the source code. This is required even 
		                              if one is providing bottles.
		  --version_file <path>       The file that contains the semver.

		Options:
		  --bottle_entry <entry>      A platform stanza (e.g., 'sha256: ...') for a 
		                              bottle.
		  --bottle_root_url <url>     The base URL from which bottles will be downloaded.
		  --license <license>         The license that governs the use of the application.
		  --out <path>                The output path where to write the formula. 
		                              If not provided, it is written to STDOUT.
		  --ruby_class_name <name>    The Ruby class name for the formula.
		  --additional_content <file> Path to a file containing additional content to add
		                              to the formula
	EOF
}

write() {
	local output="${1:-}"
	if [[ -n "${output:-}" ]]; then
		echo >&3 "${output}"
	else
		cat >&3
	fi
}

# Process Arguments

bottle_entry_paths=()
while (("$#")); do
	case "${1}" in
	"--help")
		get_usage
		exit 0
		;;
	"--bottle_entry")
		bottle_entry_paths+=("${2}")
		shift 2
		;;
	"--bottle_root_url")
		bottle_root_url="${2}"
		shift 2
		;;
	"--desc")
		desc="${2}"
		shift 2
		;;
	"--homepage")
		homepage="${2}"
		shift 2
		;;
	"--license")
		license="${2}"
		shift 2
		;;
	"--out")
		out_path="${2}"
		shift 2
		;;
	"--ruby_class_name")
		ruby_class_name="${2}"
		shift 2
		;;
	"--url")
		url="${2}"
		shift 2
		;;
	"--version_file")
		version_file="${2}"
		shift 2
		;;
	"--additional_content")
		additional_content="${2}"
		shift 2
		;;
	*)
		usage_error "Unrecognized argument. ${1}"
		;;
	esac
done

[[ -n "${desc:-}" ]] || usage_error "Missing value for 'desc'."
[[ -n "${homepage:-}" ]] || usage_error "Missing value for 'homepage'."
[[ -n "${ruby_class_name:-}" ]] || usage_error "Missing value for 'ruby_class_name'."
[[ -n "${url:-}" ]] || usage_error "Missing value for 'url'."
[[ -n "${version_file:-}" ]] || usage_error "Missing value for 'version_file'."

# All generated output will be sent to file descriptor 3. If an output file has
# been assigned, fd 3 will be assigned to that file. Otherwise, fd 3 will be
# assigned to STDOUT.
if [[ -n "${out_path:-}" ]]; then
	exec 3<>"${out_path}"
else
	# Redirect to STDOUT
	exec 3>&1
fi

# Cleanup

cleanup() {
	# Close file descriptor 3
	exec 3>&-
}
trap cleanup EXIT

# Generate Formula

# Read the version
version="$(<"${version_file}")"

# Output the start of the formula
write <<-EOF
	class ${ruby_class_name} < Formula
	  desc "${desc}"
	  homepage "${homepage}"
	  url "${url}"
	  version "${version}"
EOF

if [[ -n "${license:-}" ]]; then
	write <<-EOF
		  license "${license}"
	EOF
fi

if [[ ${#bottle_entry_paths[@]} -gt 0 ]]; then
	write "  bottle do"
	if [[ -n "${bottle_root_url:-}" ]]; then
		bottle_root_url="${bottle_root_url/0.0.0-PLACEHOLDER/$version}"
		write <<-EOF
			    root_url "${bottle_root_url}"
		EOF
	fi
	for bottle_entry_path in "${bottle_entry_paths[@]}"; do
		bottle_entry="$(<"${bottle_entry_path}")"
		write "    ${bottle_entry}"
	done
	write "  end"
fi

if [ "${additional_content:-}" ]; then
	if [ ! -f "${additional_content}" ]; then
		fail "ERROR: --additional_content file ${additional_content} does not exist"
	fi
	cat "${additional_content}" >&3
fi

# Ouptut the end of the formula
write "end"
