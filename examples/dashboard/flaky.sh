#!/bin/sh
# Fails its first $1 attempts, then passes: a flaky test for the dashboard's
# attempts (`}` and `{` step between them). The attempts so far are counted
# in a file outside the sandbox (the targets are `no-sandbox`), removed once
# it passes so the next run starts again.
fails="${1:-1}"
count="/tmp/aspect-dashboard-flaky-$(echo "$TEST_TARGET" | tr '/:' '__')-${TEST_SHARD_INDEX:-0}"
n=$(cat "$count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "attempt $n (shard ${TEST_SHARD_INDEX:-0}), failing the first $fails"
if [ "$n" -le "$fails" ]; then
    echo "$n" >"$count"
    echo "FAIL: attempt $n of a flaky test" >&2
    exit 1
fi
rm -f "$count"
echo "passed on attempt $n"
