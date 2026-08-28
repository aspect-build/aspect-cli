#!/usr/bin/env bash
#
# Run every `aspect dev test-*` suite.
#
# The suites are discovered from `aspect dev --help` rather than listed, so a
# newly registered suite is covered the moment it exists. Both CI pipelines
# call this; it replaced a pair of hand-maintained allowlists that had drifted
# to the point where 30 of 44 suites ran nowhere.
#
# Run: ./tools/run-axl-suites.sh [path-to-aspect]
set -euo pipefail

ASPECT="${1:-aspect}"

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

while read -r suite; do
    echo "--- $ASPECT dev $suite"
    "$ASPECT" dev "$suite"
done <"$SUITES"
