#!/usr/bin/env bash
#
# Run every `aspect dev test-*` suite.
#
# The suites are discovered from `aspect dev --help` rather than listed, so a
# newly registered suite is covered the moment it exists. Both CI pipelines
# call this; it replaced a pair of hand-maintained allowlists that had drifted
# to the point where 30 of 44 suites ran nowhere.
#
# An optional SHARD/TOTAL (1-based, e.g. `2/3`) runs every TOTAL-th suite of
# the discovered list, starting at SHARD, so CI can fan the suites out across
# parallel jobs without keeping a list of which suite runs where. Every suite
# lands in exactly one shard. Omit it to run them all.
#
# Run: ./tools/run-axl-suites.sh [path-to-aspect] [SHARD/TOTAL]
set -euo pipefail

ASPECT="${1:-aspect}"
SHARD_SPEC="${2:-1/1}"

if [[ ! "$SHARD_SPEC" =~ ^([1-9][0-9]*)/([1-9][0-9]*)$ ]] || ((BASH_REMATCH[1] > BASH_REMATCH[2])); then
    echo "ERROR: shard must be SHARD/TOTAL with 1 <= SHARD <= TOTAL, got '$SHARD_SPEC'." >&2
    exit 1
fi
SHARD="${BASH_REMATCH[1]}"
TOTAL="${BASH_REMATCH[2]}"

SUITES="$(mktemp)"
trap 'rm -f "$SUITES"' EXIT

# `dev --help` is plain when stdout is not a TTY, but CLICOLOR_FORCE=1 makes it
# emit SGR escapes that would hide the `Tasks:` header from the match below.
# There is no machine-readable task listing to use instead.
"$ASPECT" dev --help |
    sed $'s/\x1b\[[0-9;]*[a-zA-Z]//g' |
    awk '/^Tasks:/ { in_tasks = 1; next } in_tasks && /^  test-/ { print $1 }' >"$SUITES"

count="$(wc -l <"$SUITES" | tr -d ' ')"
if [[ "$count" -eq 0 ]]; then
    echo "ERROR: no 'dev test-*' suites discovered — refusing to pass having tested nothing." >&2
    exit 1
fi
echo "Discovered $count AXL test suites."

awk -v shard="$SHARD" -v total="$TOTAL" '(NR - 1) % total == shard - 1' "$SUITES" >"$SUITES.shard"
mv "$SUITES.shard" "$SUITES"
if [[ ! -s "$SUITES" ]]; then
    echo "ERROR: shard $SHARD_SPEC of $count suites is empty — refusing to pass having tested nothing." >&2
    exit 1
fi
echo "Running shard $SHARD_SPEC: $(wc -l <"$SUITES" | tr -d ' ') suites."

while read -r suite; do
    echo "--- $ASPECT dev $suite"
    "$ASPECT" dev "$suite"
done <"$SUITES"
