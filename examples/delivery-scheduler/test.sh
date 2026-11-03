#!/usr/bin/env bash
# Run this example with the aspect-cli binary given as $1 and check the order
# the scheduler in .aspect/config.axl promises: api starts only after migrate
# has finished, web only after api, and worker runs alongside migrate. FIFO
# dispatch fails the first two checks; a parallelism of one fails the third.
set -euo pipefail
cli="${1:?usage: test.sh <path to aspect-cli>}"
cd "$(dirname "$0")"
events="$(mktemp)"
trap 'rm -f "$events"' EXIT

DELIVERY_EVENTS="$events" "$cli" delivery \
    --mode=always \
    --track-state=false \
    --max-parallelization=4 \
    --github-status-checks:enabled=false \
    --github-status-comments:enabled=false

line() { grep -n -x "$1" "$events" | cut -d: -f1; }
before() {
    local a b
    a="$(line "$1")"
    b="$(line "$2")"
    if [[ -z "$a" || -z "$b" || "$a" -ge "$b" ]]; then
        echo "ERROR: expected '$1' before '$2'. Recorded events:" >&2
        cat "$events" >&2
        exit 1
    fi
}
before "end migrate" "start api"
before "end api" "start web"
before "start worker" "end migrate"
echo "delivery-scheduler example: deliveries ran in dependency order"
