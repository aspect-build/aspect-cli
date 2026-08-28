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

"$ASPECT" dev --help | awk '/^Tasks:/ { in_tasks = 1; next } in_tasks && /^  test-/ { print $1 }' >"$SUITES"

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
