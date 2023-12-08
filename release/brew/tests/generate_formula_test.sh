#!/usr/bin/env bash

# MARK - Locate Deps

assertions_sh_location=aspect_bazel_lib/shlib/lib/assertions.sh
assertions_sh="$(rlocation "${assertions_sh_location}")" ||
	(echo >&2 "Failed to locate ${assertions_sh_location}" && exit 1)
# shellcheck source=/dev/null
source "${assertions_sh}"

generate_formula_sh_location=build_aspect_cli/release/brew/generate_formula.sh
generate_formula_sh="$(rlocation "${generate_formula_sh_location}")" ||
	(echo >&2 "Failed to locate ${generate_formula_sh_location}" && exit 1)

# Setup

monterey_bottle_entry_path="monterey.bottle_entry"
cat >"${monterey_bottle_entry_path}" <<-EOF
	sha256 cellar: :any_skip_relocation, monterey: "ASHA256A"
EOF

arm64_monterey_bottle_entry_path="arm64_monterey.bottle_entry"
cat >"${arm64_monterey_bottle_entry_path}" <<-EOF
	sha256 cellar: :any_skip_relocation, arm64_monterey: "ASHA256B"
EOF

version_file="version"
cat >"${version_file}" <<-EOF
	1.2.3
EOF

# MARK - Functions

assert_ml_match() {
	local actual="${1}"
	local errmsg="${2:-}"
	local expected
	expected="$(</dev/stdin)"

	local cmd=(assert_match "${expected}" "${actual}")
	[[ -n "${errmsg:-}" ]] && cmd+=("${errmsg}")
	"${cmd[@]}"
}

do_generate() {
	local cmd=("${generate_formula_sh}")
	cmd+=(--ruby_class_name MyApp)
	cmd+=(--desc "My awesome application")
	cmd+=(--homepage "https://example.com/myapp")
	cmd+=(--url "https://github.com/example/myapp.git")
	cmd+=(--version_file "${version_file}")
	cmd+=(--license "Apache-2.0")
	cmd+=(--bottle_root_url "https://cdn.example.com/bottles")
	cmd+=(--bottle_entry "${monterey_bottle_entry_path}")
	cmd+=(--bottle_entry "${arm64_monterey_bottle_entry_path}")
	[[ $# -gt 0 ]] && cmd+=("$@")

	"${cmd[@]}"
}

# MARK - Test

# Generate to STDOUT
output="$(do_generate)"
assert_match "class MyApp < Formula" "${output}"
assert_ml_match "${output}" <<-EOF
	  desc "My awesome application"
EOF
assert_ml_match "${output}" <<-EOF
	  homepage "https://example.com/myapp"
EOF
assert_ml_match "${output}" <<-EOF
	  url "https://github.com/example/myapp.git"
EOF
assert_ml_match "${output}" <<-EOF
	  version "1.2.3"
EOF
assert_ml_match "${output}" <<-EOF
	  license "Apache-2.0"
EOF
assert_ml_match "${output}" <<-EOF
	  bottle do
	    root_url "https://cdn.example.com/bottles"
	    sha256 cellar: :any_skip_relocation, monterey: "ASHA256A"
	    sha256 cellar: :any_skip_relocation, arm64_monterey: "ASHA256B"
	  end
EOF

# Generate to file
output_path="formula.rb"
do_generate --out "${output_path}"
output="$(<"${output_path}")"
assert_match "class MyApp < Formula" "${output}"
