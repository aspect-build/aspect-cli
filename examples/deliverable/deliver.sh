#!/usr/bin/env bash
set -euo pipefail
# BUILD.bazel passes `$(location :delivery_payload)` as the first argument.
# `bazel run` expands that to the runfiles-relative path, which resolves from
# the cwd (<runfiles>/_main). An execpath (bazel-out/...) does not exist here,
# so this check fails whenever the runner expands `args` differently.
payload="${1:?expected the \$(location) of :delivery_payload as the first argument}"
if [[ ! -f "$payload" ]]; then
    echo "deliver.sh: payload '$payload' does not exist relative to $PWD" >&2
    exit 1
fi
echo "Delivering Aspect CLI! payload=$payload contents=$(cat "$payload") extra_args=${*:2}"
# The delivery task stores this metadata with the delivery and returns it when
# a later run reuses it.
json_string() {
    local s="${1//\\/\\\\}"
    s="${s//\"/\\\"}"
    printf '"%s"' "${s//$'\n'/\\n}"
}
if [[ -n "${ASPECT_DELIVERY_METADATA_JSON_FILE:-}" ]]; then
    printf '{"payload": %s, "contents": %s}\n' "$(json_string "$payload")" "$(json_string "$(cat "$payload")")" >"$ASPECT_DELIVERY_METADATA_JSON_FILE"
fi
