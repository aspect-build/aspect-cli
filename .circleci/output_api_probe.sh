#!/usr/bin/env bash
#
# Direct, agent-free probe of the CircleCI runner "output" API.
#
# Background: lib/circleci.axl already uploads *artifacts* by talking to the
# runner output API directly (token from the /tmp/circleci-ts.sock socket ->
# /api/v2/output/{config,credentials} -> presigned S3 PUT). This probe checks
# whether the *test-results* surface of that same API behaves the way it was
# reverse-engineered from the circleci-agent binary, WITHOUT invoking the
# agent:
#
#   POST /api/v2/output/test-result          body={"step_id":N} -> presigned Key
#   PUT  <tar of junit xml> to Key.location  (using Key.method + Key.headers)
#   POST /api/v2/output/test-result/process  body={"step_id":N} -> TestProcessSummary
#   GET  /api/v2/output/prev-test-result/find                   -> BestPreviousJob | 404
#   GET  /api/v2/output/prev-test-result/job/{id}               -> {tasks:[{keys:[Key]}]}
#   GET  <Key.location> (presigned)                             -> processed Case JSONL
#
# Protocol details recovered from build-agent's StoreTestResultsStep.Run ->
# output/storage.(*Store).UploadTestResult:
#   * The request body of BOTH the test-result POST and the test-result/process
#     POST is a JSON object, not a bare int. The boxed body type in the agent
#     is `struct { StepID int32 "json:\"step_id\"" }`, so the wire form is
#     {"step_id": <N>}. The same step_id is sent to both endpoints. A bare int,
#     or an unknown step_id, is rejected with an empty-body HTTP 400.
#   * step_id is the agent's task-config step index (read from the step object,
#     not an env var). This probe discovers it by trying small integers until
#     the output service accepts one (override with CIRCLECI_PROBE_STEP_ID).
#   * The bytes PUT to the presigned URL are a single *plain (uncompressed) tar*
#     built by pkg/tar.CreateArchiveFromPaths (no gzip in the chain) containing
#     the JUnit files at their relative paths — not the raw XML.
#   * The runner-API POSTs use Content-Type "application/json" (no charset
#     suffix), matching the artifact-upload path in lib/circleci.axl.
#
# It is intentionally self-contained (curl + python3, same tools the runner
# image already has) and isolated in its own CI job so it cannot affect the
# rest of the pipeline. Synthetic test results uploaded here simply show up in
# this job's "Tests" tab — that is the expected, benign side effect.
#
# Exit non-zero only if the core upload trio fails, so the CI job's red/green
# state reflects whether the direct protocol actually works.

set -uo pipefail

SOCK="${CIRCLECI_TS_SOCK:-/tmp/circleci-ts.sock}"
DEFAULT_HOST="https://runner.circleci.com"

fail=0
note() { printf '\n=== %s ===\n' "$*"; }
ok() { printf '  PASS  %s\n' "$*"; }
bad() {
    printf '  FAIL  %s\n' "$*"
    fail=1
}

# ---------------------------------------------------------------------------
# 1. Read {token, runner_host} from the runner socket.
#
# The socket is a raw connect/read (no HTTP): connect, half-close the write
# side so the peer sees EOF and replies, read everything back. That is exactly
# what std.net.try_unix_request does for lib/circleci.axl. Mirror it here with
# python3 (preferred) or socat (fallback).
# ---------------------------------------------------------------------------
read_socket() {
    if command -v python3 >/dev/null 2>&1; then
        python3 - "$SOCK" <<'PY'
import socket, sys
p = sys.argv[1]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
try:
    s.connect(p)
    s.shutdown(socket.SHUT_WR)   # EOF on write side -> peer replies
    buf = b""
    while True:
        b = s.recv(65536)
        if not b:
            break
        buf += b
    sys.stdout.write(buf.decode("utf-8", "replace"))
except OSError as e:
    sys.stderr.write("socket error: %s\n" % e)
    sys.exit(3)
PY
    elif command -v socat >/dev/null 2>&1; then
        socat -t5 - "UNIX-CONNECT:${SOCK}" </dev/null
    else
        echo "no python3 or socat to read unix socket" >&2
        return 3
    fi
}

jget() { # jget <json-on-stdin> <python-expr over `d`>
    python3 -c 'import sys,json;d=json.load(sys.stdin);print(eval(sys.argv[1]))' "$1"
}

note "runner socket token ($SOCK)"
TOKEN_JSON="$(read_socket || true)"
if [ -z "${TOKEN_JSON:-}" ]; then
    bad "no response from runner socket — cannot authenticate; aborting"
    echo "(this probe must run inside a CircleCI runner job)"
    exit 1
fi
TOKEN="$(printf '%s' "$TOKEN_JSON" | jget 'd.get("token","")' 2>/dev/null || true)"
HOST="$(printf '%s' "$TOKEN_JSON" | jget 'd.get("runner_host","")' 2>/dev/null || true)"
[ -z "$HOST" ] && HOST="$DEFAULT_HOST"
if [ -n "$TOKEN" ]; then
    ok "got token (len=${#TOKEN}), runner_host=$HOST"
else
    bad "socket replied but no token field; raw: ${TOKEN_JSON:0:120}"
    exit 1
fi

AUTH=(-H "Authorization: Bearer ${TOKEN}")
JSONH=(-H "Content-Type: application/json")
api() { printf '%s%s' "$HOST" "$1"; }

# curl helper: prints "<http_code>\n<body>"; never fails the script itself.
req() { # req METHOD URL [curl-args...]
    local m="$1" url="$2"
    shift 2
    curl -sS -X "$m" -w $'\n%{http_code}' "$@" "$url" 2>&1
}
split_code() { tail -n1 <<<"$1"; }
split_body() { sed '$d' <<<"$1"; }

# ---------------------------------------------------------------------------
# 2. Baseline: read-only endpoints that artifact upload already relies on.
#    Confirms auth + reachability before we touch the test-result surface.
# ---------------------------------------------------------------------------
note "baseline GET /api/v2/output/config"
R="$(req GET "$(api /api/v2/output/config)" "${AUTH[@]}")"
C="$(split_code "$R")"
B="$(split_body "$R")"
BUCKET=""
REGION=""
if [ "$C" = "200" ]; then
    ok "config 200: $(printf '%s' "$B" | head -c 160)"
    BUCKET="$(printf '%s' "$B" | jget 'd.get("bucket","")' 2>/dev/null || true)"
    REGION="$(printf '%s' "$B" | jget 'd.get("region","")' 2>/dev/null || true)"
else
    bad "config HTTP $C: $(printf '%s' "$B" | head -c 200)"
fi

# ---------------------------------------------------------------------------
# 3. STORE: upload a synthetic JUnit file via the reverse-engineered trio.
# ---------------------------------------------------------------------------
# Build one synthetic JUnit file, then tar it the way the agent does
# (pkg/tar.CreateArchiveFromPaths: a plain, uncompressed tar of the files at
# their relative paths; no gzip).
WORK="$(mktemp -d /tmp/probe-tr-XXXX)"
JUNIT="$WORK/probe-junit.xml"
# Mix of outcomes so we can confirm failed/errored/skipped cases render in the
# job's "Tests" tab, not just passing ones: 1 pass, 1 <failure>, 1 <error>,
# 1 <skipped>. The testsuite counts must match or some parsers complain.
cat >"$JUNIT" <<'XML'
<testsuites>
  <testsuite name="aspect.output_api_probe" tests="4" failures="1" errors="1" skipped="1" time="0.178">
    <testcase classname="probe.Direct" file="probe_test.sh" name="upload_roundtrip" time="0.123"/>
    <testcase classname="probe.Direct" file="probe_test.sh" name="presigned_put_should_fail" time="0.045">
      <failure message="synthetic failure: expected 200 but got 500" type="AssertionError">probe-induced failure to verify failed tests render in the CircleCI Tests tab</failure>
    </testcase>
    <testcase classname="probe.Direct" file="probe_test.sh" name="process_raises" time="0.010">
      <error message="synthetic error: unexpected exception" type="RuntimeError">probe-induced error to verify errored tests render in the CircleCI Tests tab</error>
    </testcase>
    <testcase classname="probe.Direct" file="probe_test.sh" name="split_unsupported" time="0.000">
      <skipped message="probe-induced skip"/>
    </testcase>
  </testsuite>
</testsuites>
XML
TARBALL="$WORK/test-results.tar.gz"
# -C so the entry is stored at its relative path (probe-junit.xml), matching
# pkg/tar.toRelativePaths. The Key the server hands back names the object
# "<n>.tar.gz" and the agent logs "compressing test results", so the stored
# bytes are a gzipped tar.
tar -czf "$TARBALL" -C "$WORK" "$(basename "$JUNIT")"

# The body of BOTH test-result POSTs is a JSON object {"step_id": <int>} (the
# field tag recovered from the boxed body type in the agent:
# `struct { StepID int32 "json:\"step_id\"" }`). step_id is the agent's own
# task-config step index; it is not exposed as an env var, so a raw client must
# either be told it (CIRCLECI_PROBE_STEP_ID) or discover it by probing which
# small integer the output service accepts for this task. A wrong/unknown
# step_id is rejected with an empty-body HTTP 400 — exactly what we first saw.
step_body() { printf '{"step_id":%s}' "$1"; }

STEP_ID=""
LOC=""
HDR_ARGS=()
CANDIDATES="${CIRCLECI_PROBE_STEP_ID:-$(seq 0 30)}"
note "POST /api/v2/output/test-result  (discover step_id; body {\"step_id\":k})"
for k in $CANDIDATES; do
    R="$(req POST "$(api /api/v2/output/test-result)" "${AUTH[@]}" "${JSONH[@]}" -d "$(step_body "$k")")"
    C="$(split_code "$R")"
    B="$(split_body "$R")"
    if [ "$C" = "200" ] || [ "$C" = "201" ]; then
        STEP_ID="$k"
        ok "step_id=$k accepted ($C) -> $(printf '%s' "$B" | head -c 160)"
        LOC="$(printf '%s' "$B" | jget 'd.get("location","")' 2>/dev/null || true)"
        PMETHOD="$(printf '%s' "$B" | jget 'd.get("method","PUT")' 2>/dev/null || echo PUT)"
        while IFS=$'\t' read -r hk hv; do [ -n "$hk" ] && HDR_ARGS+=(-H "$hk: $hv"); done < <(
            printf '%s' "$B" | python3 -c 'import sys,json;d=json.load(sys.stdin);[print("%s\t%s"%(k,v)) for k,v in (d.get("headers") or {}).items()]' 2>/dev/null || true
        )
        break
    fi
done

# Upload the gzipped tar to the location from the Key. Unlike artifacts (full
# presigned URL), the test-result Key.location is a *relative S3 object key*
# (e.g. "storage/test-raw/<uuid>/<uuid>/0/0.tar.gz"). The agent uploads it
# through its S3 object-store backend, i.e. a sigv4 PUT to the config bucket
# using the short-lived STS creds from /api/v2/output/credentials — the same
# signing path lib/circleci.axl already uses for artifacts. If a location ever
# comes back as a full URL we PUT to it directly with the Key headers instead.
s3_put() { # s3_put <url>
    note "S3 sigv4 PUT gz(tar) -> $1"
    local creds ak sk st
    creds="$(req GET "$(api /api/v2/output/credentials)" "${AUTH[@]}")"
    if [ "$(split_code "$creds")" != "200" ]; then
        bad "credentials HTTP $(split_code "$creds"): $(split_body "$creds" | head -c 160)"
        return
    fi
    creds="$(split_body "$creds")"
    ak="$(printf '%s' "$creds" | jget 'd["s3"]["AccessKeyID"]' 2>/dev/null || true)"
    sk="$(printf '%s' "$creds" | jget 'd["s3"]["SecretAccessKey"]' 2>/dev/null || true)"
    st="$(printf '%s' "$creds" | jget 'd["s3"]["SessionToken"]' 2>/dev/null || true)"
    if [ -z "$ak" ] || [ -z "$sk" ]; then
        bad "no S3 credentials in /output/credentials response"
        return
    fi
    local R2 C2
    R2="$(req "$PMETHOD" "$1" \
        --aws-sigv4 "aws:amz:${REGION}:s3" \
        --user "${ak}:${sk}" \
        -H "x-amz-security-token: ${st}" \
        "${HDR_ARGS[@]}" \
        --data-binary "@$TARBALL")"
    C2="$(split_code "$R2")"
    if [[ "$C2" =~ ^2 ]]; then ok "S3 upload HTTP $C2"; else bad "S3 upload HTTP $C2: $(split_body "$R2" | head -c 200)"; fi
}

if [ -z "$STEP_ID" ]; then
    bad "no step_id in [$CANDIDATES] accepted by test-result (last HTTP $C: $(printf '%s' "$B" | head -c 200))"
elif [ -n "$LOC" ]; then
    if [[ "$LOC" =~ ^https?:// ]]; then
        note "presigned $PMETHOD gz(tar) -> object store"
        R2="$(req "$PMETHOD" "$LOC" "${HDR_ARGS[@]}" --data-binary "@$TARBALL")"
        C2="$(split_code "$R2")"
        if [[ "$C2" =~ ^2 ]]; then ok "presigned upload HTTP $C2"; else bad "presigned upload HTTP $C2: $(split_body "$R2" | head -c 200)"; fi
    elif [ -n "$BUCKET" ] && [ -n "$REGION" ]; then
        s3_put "https://${BUCKET}.s3.${REGION}.amazonaws.com/${LOC}"
    else
        bad "relative location '$LOC' but no bucket/region from /output/config"
    fi

    note "POST /api/v2/output/test-result/process  (body {\"step_id\":$STEP_ID})"
    R="$(req POST "$(api /api/v2/output/test-result/process)" "${AUTH[@]}" "${JSONH[@]}" -d "$(step_body "$STEP_ID")")"
    C="$(split_code "$R")"
    B="$(split_body "$R")"
    if [ "$C" = "200" ]; then ok "process 200 -> $(printf '%s' "$B" | head -c 240)"; else bad "process HTTP $C: $(printf '%s' "$B" | head -c 300)"; fi
else
    bad "test-result accepted step_id=$STEP_ID but returned no presigned 'location'"
fi

# ---------------------------------------------------------------------------
# 4. SPLIT-PREP: the read side a parallel run would use for timing data.
#    On the first ever run there is no prior job -> 404 is the documented,
#    expected "no timings yet" signal, not a failure.
# ---------------------------------------------------------------------------
note "GET /api/v2/output/prev-test-result/find"
R="$(req GET "$(api /api/v2/output/prev-test-result/find)" "${AUTH[@]}")"
C="$(split_code "$R")"
B="$(split_body "$R")"
if [ "$C" = "200" ]; then
    ok "find 200 -> $(printf '%s' "$B" | head -c 200)"
    JID="$(printf '%s' "$B" | jget 'd.get("id","")' 2>/dev/null || true)"
    if [ -n "$JID" ]; then
        note "GET /api/v2/output/prev-test-result/job/$JID"
        R="$(req GET "$(api "/api/v2/output/prev-test-result/job/$JID")" "${AUTH[@]}")"
        C="$(split_code "$R")"
        if [ "$C" = "200" ]; then ok "job 200 -> $(split_body "$R" | head -c 240)"; else bad "job HTTP $C: $(split_body "$R" | head -c 200)"; fi
    fi
elif [ "$C" = "404" ]; then
    ok "find 404 — no prior job yet (expected on a fresh branch)"
else
    bad "find HTTP $C: $(printf '%s' "$B" | head -c 200)"
fi

rm -rf "$WORK"
note "summary"
if [ "$fail" = "0" ]; then
    echo "  ALL CORE CHECKS PASSED — direct (agent-free) test-results protocol works against real CircleCI."
else
    echo "  SOME CHECKS FAILED — see FAIL lines above."
fi
exit "$fail"
