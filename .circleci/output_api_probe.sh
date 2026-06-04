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
#   POST /api/v2/output/test-result          body=<int index>  -> presigned Key
#   PUT  <raw junit xml> to Key.location     (using Key.method + Key.headers)
#   POST /api/v2/output/test-result/process  body=<int count>  -> TestProcessSummary
#   GET  /api/v2/output/prev-test-result/find                  -> BestPreviousJob | 404
#   GET  /api/v2/output/prev-test-result/job/{id}              -> {tasks:[{keys:[Key]}]}
#   GET  <Key.location> (presigned)                            -> processed Case JSONL
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
ok()   { printf '  PASS  %s\n' "$*"; }
bad()  { printf '  FAIL  %s\n' "$*"; fail=1; }

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
JSONH=(-H "Content-Type: application/json; charset=utf-8")
api() { printf '%s%s' "$HOST" "$1"; }

# curl helper: prints "<http_code>\n<body>"; never fails the script itself.
req() { # req METHOD URL [curl-args...]
  local m="$1" url="$2"; shift 2
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
if [ "$C" = "200" ]; then ok "config 200: $(split_body "$R" | head -c 160)"; else bad "config HTTP $C: $(split_body "$R" | head -c 200)"; fi

# ---------------------------------------------------------------------------
# 3. STORE: upload a synthetic JUnit file via the reverse-engineered trio.
# ---------------------------------------------------------------------------
JUNIT="$(mktemp /tmp/probe-junit-XXXX.xml)"
cat > "$JUNIT" <<'XML'
<testsuites>
  <testsuite name="aspect.output_api_probe" tests="2">
    <testcase classname="probe.Direct" file="probe_test.sh" name="upload_roundtrip" time="0.123"/>
    <testcase classname="probe.Direct" file="probe_test.sh" name="presigned_put" time="0.045"/>
  </testsuite>
</testsuites>
XML

note "POST /api/v2/output/test-result  (body = bare int index 0)"
R="$(req POST "$(api /api/v2/output/test-result)" "${AUTH[@]}" "${JSONH[@]}" -d '0')"
C="$(split_code "$R")"; B="$(split_body "$R")"
if [ "$C" = "200" ] || [ "$C" = "201" ]; then
  ok "test-result $C -> $(printf '%s' "$B" | head -c 200)"
  LOC="$(printf '%s' "$B" | jget 'd.get("location","")' 2>/dev/null || true)"
  PMETHOD="$(printf '%s' "$B" | jget 'd.get("method","PUT")' 2>/dev/null || echo PUT)"
  # Build -H args from Key.headers
  HDR_ARGS=()
  while IFS=$'\t' read -r k v; do [ -n "$k" ] && HDR_ARGS+=(-H "$k: $v"); done < <(
    printf '%s' "$B" | python3 -c 'import sys,json;d=json.load(sys.stdin);[print("%s\t%s"%(k,v)) for k,v in (d.get("headers") or {}).items()]' 2>/dev/null || true)
  if [ -n "$LOC" ]; then
    note "presigned $PMETHOD raw JUnit -> object store"
    R2="$(req "$PMETHOD" "$LOC" "${HDR_ARGS[@]}" --data-binary "@$JUNIT")"
    C2="$(split_code "$R2")"
    if [[ "$C2" =~ ^2 ]]; then ok "presigned upload HTTP $C2"; else bad "presigned upload HTTP $C2: $(split_body "$R2" | head -c 200)"; fi
  else
    bad "no presigned 'location' in Key response"
  fi
else
  bad "test-result HTTP $C: $(printf '%s' "$B" | head -c 300)"
fi

note "POST /api/v2/output/test-result/process  (body = bare int count 1)"
R="$(req POST "$(api /api/v2/output/test-result/process)" "${AUTH[@]}" "${JSONH[@]}" -d '1')"
C="$(split_code "$R")"; B="$(split_body "$R")"
if [ "$C" = "200" ]; then ok "process 200 -> $(printf '%s' "$B" | head -c 240)"; else bad "process HTTP $C: $(printf '%s' "$B" | head -c 300)"; fi

# ---------------------------------------------------------------------------
# 4. SPLIT-PREP: the read side a parallel run would use for timing data.
#    On the first ever run there is no prior job -> 404 is the documented,
#    expected "no timings yet" signal, not a failure.
# ---------------------------------------------------------------------------
note "GET /api/v2/output/prev-test-result/find"
R="$(req GET "$(api /api/v2/output/prev-test-result/find)" "${AUTH[@]}")"
C="$(split_code "$R")"; B="$(split_body "$R")"
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

rm -f "$JUNIT"
note "summary"
if [ "$fail" = "0" ]; then
  echo "  ALL CORE CHECKS PASSED — direct (agent-free) test-results protocol works against real CircleCI."
else
  echo "  SOME CHECKS FAILED — see FAIL lines above."
fi
exit "$fail"
