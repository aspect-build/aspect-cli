#!/usr/bin/env bash
# Simulates a flaky test by using a counter file in /tmp (survives between attempts).
# The counter file is keyed by target label so parallel tests don't collide.
# With --flaky_test_attempts=2 and --nocache_test_results:
#   attempt 1 (counter=1) → exit 1  (fails)
#   attempt 2 (counter=2) → exit 0  (passes on retry)
# Bazel reports overall_status=FLAKY in test_summary.
LABEL_SAFE="${TEST_TARGET//[^a-zA-Z0-9]/_}"
COUNTER_FILE="/tmp/flaky_counter_${LABEL_SAFE}"

# Initialize or increment counter
if [ ! -f "$COUNTER_FILE" ]; then
    echo 1 > "$COUNTER_FILE"
else
    COUNT=$(cat "$COUNTER_FILE")
    echo $((COUNT + 1)) > "$COUNTER_FILE"
fi

COUNT=$(cat "$COUNTER_FILE")
echo "flaky_test: attempt $COUNT (counter=$COUNTER_FILE)"

if [ "$COUNT" -le 1 ]; then
    echo "flaky_test: FAILING on attempt $COUNT (simulated flaky)"
    exit 1
fi

echo "flaky_test: PASSING on attempt $COUNT (recovered)"
# Clean up so the next test run starts fresh
rm -f "$COUNTER_FILE"
exit 0
