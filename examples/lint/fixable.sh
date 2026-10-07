#!/usr/bin/env bash
# Sample shell script whose shellcheck findings all carry an auto-fix, so
# `aspect lint` can show a suggested patch for each one.

name=$1

# SC2006: Use $(...) notation instead of legacy backticks.
now=`date +%s`

# SC2086: Double quote to prevent globbing and word splitting.
echo Hello, $name at $now

# SC2164: Use 'cd ... || exit' in case cd fails.
cd /tmp
echo "Working in $(pwd)"
