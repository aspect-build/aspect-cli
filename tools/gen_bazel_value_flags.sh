#!/usr/bin/env bash
#
# Regenerate the list of Bazel flags that take a separate value, which the CLI
# uses to collect `-c opt` as one flag-plus-value pair instead of leaving `opt`
# behind as a target pattern. Run after a Bazel release:
#
#   tools/gen_bazel_value_flags.sh
#
# The list is the union across every Bazel a workspace might be pinned to, so a
# repo on an older — or newer — Bazel gets the same spellings its Bazel accepts.
# Each entry in BAZEL_VERSIONS is a USE_BAZEL_VERSION spec that bazelisk
# resolves: `<major>.x` is that major's latest patch, `rolling` is the newest
# pre-release. `bazel help` runs in batch mode outside a workspace, so nothing
# here touches a Bazel server.
#
# Arity comes from Bazel's own rendering of each flag:
#
#   --jobs [-x] (an integer…)      takes a value        → listed (with -x)
#   --[no]announce_rc (a boolean…) takes none           → excluded
#   --remote_download_all          expansion, no value  → excluded (no `(`)
#
# A flag that takes a value in one version and is boolean in another is excluded
# outright: listing it would swallow the following target pattern for anyone on
# the boolean version, which costs more than the `=`-less spelling is worth.
#
# Deliberately omitted: label-shaped flags from rule sets (`--//pkg:flag`) and
# their `--flag_alias` spellings. Bazel *requires* `=` for those, so accepting a
# space-separated value would make `aspect` take command lines that plain `bazel`
# rejects, and the repro commands we print would not run when pasted. A repo that
# needs more can extend the list from `config.axl`; see `bazel/flags.axl`.
set -o errexit -o nounset -o pipefail

SCRIPTPATH="$(cd -- "$(dirname "$0")" >/dev/null 2>&1 && pwd -P)"
OUT="${SCRIPTPATH}/../crates/aspect-cli/src/builtins/aspect/bazel/value_flags.axl"
# USE_BAZEL_VERSION specs, resolved by bazelisk. Add a major when Bazel ships
# one; `rolling` tracks the newest pre-release, so flags land here before the
# release does.
BAZEL_VERSIONS=(6.x 7.x 8.x 9.x rolling)
COMMANDS=(build test run query cquery aquery coverage)

# Value-taking flags that no released or pre-release Bazel has yet, from a
# pull request expected to land. Each entry names its PR so it can be dropped
# once a version in BAZEL_VERSIONS ships the flag — leaving it costs nothing
# either way, since a Bazel without the flag rejects it on its own.
#
#   bazelbuild/bazel#29886 — generic sandbox backend spawn strategy
PENDING_VALUE_FLAGS=(
    --sandbox_backend
    --sandbox_backend_opt
)
# Floor per version: enough that a failed download, a command Bazel no longer
# has, or a changed help format fails loudly instead of silently contributing
# nothing.
MIN_FLAGS_PER_VERSION=100

# Run Bazel from an empty directory: outside a workspace it answers `help` in
# batch mode without starting a server, and an older Bazel never has to make
# sense of this repo's MODULE.bazel or `tools/bazel` wrapper.
NOWHERE="$(mktemp -d)"

bazel_at() {
    local spec="$1"
    shift
    (cd "$NOWHERE" && USE_BAZEL_VERSION="$spec" bazel "$@")
}

# `bazel help` for one version, over every command. Written to stdout for the
# callers below to parse.
help_output() {
    local spec="$1" command
    for command in "${COMMANDS[@]}"; do
        bazel_at "$spec" help "$command" 2>/dev/null || true
    done
}

# Startup options are a separate help topic and a separate slot on the command
# line, so they get their own list: `aspect --output_base /tmp/o build` has to
# collect the value the way `bazel --output_base /tmp/o build` does.
startup_help_output() {
    bazel_at "$1" help startup_options 2>/dev/null || true
}

# awk rather than sed: a line can yield two flags (`--jobs [-j]`), and emitting
# both from one sed rule needs `\n` in the replacement, which is a GNU extension.
# Chained `s///p` rules are not an alternative — each substitution rewrites the
# pattern space the next rule would have matched. The shape check below is the
# backstop if a parse ever mangles a name anyway.
#
# `  --flag (a string…)` / `  --flag [-x] (…)` take a value; `  --[no]flag (…)`
# does not; an expansion flag renders bare and matches neither.
value_flags() {
    awk '
        /^  --[a-z_]+ \(/ { print $1; next }
        /^  --[a-z_]+ \[-[a-zA-Z]\] \(/ {
            print $1
            short = $2
            gsub(/[][]/, "", short)
            print short
        }
    '
}

boolean_flags() {
    awk '
        /^  --\[no\][a-z_]+ (\[-[a-zA-Z]\] )?\(/ {
            long = $1
            sub(/^--\[no\]/, "--", long)
            print long
            if ($2 ~ /^\[-[a-zA-Z]\]$/) {
                short = $2
                gsub(/[][]/, "", short)
                print short
            }
        }
    '
}

values="$(mktemp)"
booleans="$(mktemp)"
startup_values="$(mktemp)"
startup_booleans="$(mktemp)"
versions=()
trap 'rm -rf "$values" "$booleans" "$startup_values" "$startup_booleans" "$NOWHERE"' EXIT

for spec in "${BAZEL_VERSIONS[@]}"; do
    version="$(bazel_at "$spec" --version | awk '{print $2}')"
    versions+=("$version")

    help="$(help_output "$spec")"
    mapfile -t version_values < <(printf '%s\n' "$help" | value_flags | sort -u)
    if [ "${#version_values[@]}" -lt "$MIN_FLAGS_PER_VERSION" ]; then
        echo "error: Bazel ${version} yielded only ${#version_values[@]} value-taking flags;" \
            "is \`bazel help\` output shaped as expected?" >&2
        exit 1
    fi
    printf '%s\n' "${version_values[@]}" >>"$values"
    printf '%s\n' "$help" | boolean_flags >>"$booleans"

    startup_help="$(startup_help_output "$spec")"
    printf '%s\n' "$startup_help" | value_flags >>"$startup_values"
    printf '%s\n' "$startup_help" | boolean_flags >>"$startup_booleans"

    echo "  ${version}: ${#version_values[@]} value-taking command flags" >&2
done

# Value-taking in every version that has the flag at all, plus the pending ones.
printf '%s\n' "${PENDING_VALUE_FLAGS[@]}" >>"$values"
mapfile -t flags < <(sort -u "$values" | comm -23 - <(sort -u "$booleans"))
mapfile -t startup_flags < <(sort -u "$startup_values" | comm -23 - <(sort -u "$startup_booleans"))

# Name what the arity rule dropped. Bazel 10 gave several flags an optional
# value (`--[no]disk_cache`), which costs those flags their `--flag value`
# spelling here; that should be a visible outcome of regenerating, not a
# silent one.
mapfile -t conflicted < <(
    sort -u "$values" | comm -12 - <(sort -u "$booleans")
    sort -u "$startup_values" | comm -12 - <(sort -u "$startup_booleans")
)
if [ "${#conflicted[@]}" -gt 0 ]; then
    echo "  excluded (value-taking in one version, boolean in another): ${conflicted[*]}" >&2
fi

# Every entry has to be a flag Bazel could actually accept. Catches a parsing
# slip that leaves the count plausible but the names mangled.
for flag in "${flags[@]}" "${startup_flags[@]}"; do
    case "$flag" in
    --[a-z_]* | -[a-zA-Z]) ;;
    *)
        echo "error: parsed a flag name that cannot be one: ${flag}" >&2
        exit 1
        ;;
    esac
done
if [ "${#startup_flags[@]}" -lt 5 ]; then
    echo "error: only ${#startup_flags[@]} value-taking startup options parsed" >&2
    exit 1
fi

versions_list="$(
    IFS=,
    echo "${versions[*]}" | sed 's/,/, /g'
)"

# Spelled into the docstring so the answer to "why isn't --disk_cache here?"
# lives in the file itself.
if [ "${#conflicted[@]}" -gt 0 ]; then
    conflicted_list="$(
        IFS=,
        echo "${conflicted[*]}" | sed 's/,/, /g'
    )"
    conflicted_note="Left out on those grounds: ${conflicted_list}."
else
    conflicted_note="No flag currently disagrees across the versions above."
fi

{
    cat <<EOF
"""The Bazel flags that take a separate value (\`-c opt\`, \`--jobs 8\`).

GENERATED by tools/gen_bazel_value_flags.sh from \`bazel help\` — do not edit by
hand. Regenerate after a Bazel release.

The union across every Bazel a workspace might be pinned to, so a repo on an
older — or newer — Bazel gets the same spellings its Bazel accepts: ${versions_list}.

A flag that takes a value in one version and is boolean in another is left out —
listing it would swallow the following target pattern for anyone on the boolean
version, so those keep their \`--flag=value\` spelling only. ${conflicted_note}

A few entries come from a pull request expected to land rather than from any
Bazel above; see PENDING_VALUE_FLAGS in the generator.

Consumed by \`bazel/flags.axl\`, which hands it to the post-command
\`args.passthrough()\` bucket so a forwarded flag's value is collected with the
flag instead of being read as a target pattern. Version skew is benign in both
directions: a flag missing from this list simply still needs \`--flag=value\`,
and a stale entry only matters if someone types it.

A repo can replace the list for its own wrapper flags:

    load("@aspect//bazel.axl", bzl = "bazel")

    def config(ctx):
        ctx.tasks["build"].args.bazel_value_flags = bzl.flags.BAZEL_VALUE_FLAGS + ["--my_wrapper_flag"]

Label-shaped flags (\`--//pkg:flag\`) are absent on purpose: Bazel requires \`=\`
for those, so accepting a space-separated value would diverge from Bazel and
break the \`bazel …\` repro commands the CLI prints.
"""

BAZEL_VALUE_FLAGS = [
EOF
    printf '    "%s",\n' "${flags[@]}"
    cat <<'EOF'
]

# Startup options that take a separate value, for the pre-command bucket:
# `aspect --output_base /tmp/o build` collects the value the way Bazel does.
BAZEL_STARTUP_VALUE_FLAGS = [
EOF
    printf '    "%s",\n' "${startup_flags[@]}"
    echo "]"
} >"$OUT"

echo "wrote ${#flags[@]} command flags and ${#startup_flags[@]} startup options to ${OUT#"${SCRIPTPATH}/../"}"
