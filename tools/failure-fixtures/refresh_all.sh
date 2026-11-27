#!/usr/bin/env bash
# Re-record every failure fixture for the given Bazel versions: refresh_all.sh 7.7.3 8.9.0 9.3.3
# Then `aspect dev test-failure-replay --update` rewrites the replay goldens; review the diff.
HERE=$(cd "$(dirname "$0")" && pwd)
FIXTURES=$(cd "$HERE/../../crates/aspect-cli/tests/testdata/failures" && pwd)
SCRATCH=${FIXTURE_SCRATCH:-${TMPDIR:-/tmp}/aspect-failure-fixtures}
for v in "$@"; do
    for c in $(ls "$FIXTURES"); do [ -d "$FIXTURES/$c/src" ] && "$HERE/record.sh" "$c" "$v"; done
    (cd "$SCRATCH/runws" && env -i HOME="$SCRATCH/home" PATH=/opt/homebrew/bin:/usr/bin:/bin BAZELISK_HOME="$HOME/Library/Caches/bazelisk" USE_BAZEL_VERSION="$v" bazel --nosystem_rc shutdown)
done
