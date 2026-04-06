#!/usr/bin/env bash
# Sample shell script demonstrating shellcheck lint integration.
# This file intentionally contains shellcheck findings to show how
# aspect lint reports issues.

name=$1

# SC2086: Double quote to prevent globbing and word splitting.
# shellcheck will flag the unquoted $name below.
if [ $name = "world" ]; then
    echo "Hello, World!"
else
    echo "Hello, $name!"
fi

# SC2164: Use 'cd ... || exit' or 'cd ... || return' in case cd fails.
cd /tmp
echo "Working in $(pwd)"
