#!/usr/bin/env bash
# Re-record every failure fixture for the given Bazel versions: refresh_all.sh 7.7.3 8.9.0 9.3.3
# Then `aspect dev test-failure-replay --update` rewrites the replay goldens; review the diff.
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=env.sh
. "$HERE/env.sh"
for v in "$@"; do
    for c in $(ls "$FIXTURES"); do [ -d "$FIXTURES/$c/src" ] && "$HERE/record.sh" "$c" "$v"; done
    bazel_env "$v"
    (cd "$SCRATCH/runws" && "${BAZEL_ENV[@]}" "$BAZELISK" --nosystem_rc shutdown)
done
