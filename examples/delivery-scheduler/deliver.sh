#!/usr/bin/env bash
# Stand-in for a deploy step: announce, take $2 seconds, finish. With
# $DELIVERY_EVENTS set, also append `start <name>` / `end <name>` to that file,
# which test.sh reads to check the order.
set -euo pipefail
name="${1:?name}"
record() { [[ -z "${DELIVERY_EVENTS:-}" ]] || echo "$1 ${name}" >>"$DELIVERY_EVENTS"; }
record start
echo "$(date +%T) ${name}: delivering"
sleep "${2:-1}"
echo "$(date +%T) ${name}: done"
record end
