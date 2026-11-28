#!/bin/sh
# Fails every time, to show a failed test's log in the dashboard.
echo "running the checks"
echo "FAIL: expected 2, got 3" >&2
exit 1
