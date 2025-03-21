#!/usr/bin/env bash
IFS=':' read -ra LIBS <<<"$BATS_LIB_PATH"

NEW_LIBS=()
for RAW_LIB_PATH in "${LIBS[@]}"; do
    NEW_PATH=$(cd "${RAW_LIB_PATH}" && pwd)
    NEW_LIBS+=("$NEW_PATH")
done

BATS_LIB_PATH=$(
    IFS=:
    echo "${NEW_LIBS[*]}"
)
export BATS_LIB_PATH
export BATS_TEST_TIMEOUT="$TEST_TIMEOUT"
export BATS_TMPDIR="$TEST_TMPDIR"

if [ -n "$TESTBRIDGE_TEST_ONLY" ]; then
    exec "${BIN}" --filter "$TESTBRIDGE_TEST_ONLY" "$@"
else
    exec "${BIN}" "$@"
fi
