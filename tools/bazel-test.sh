#!/usr/bin/env bash
#
# Tests for tools/bazel. Stubs both aspect and bazel so we can assert exactly
# what the wrapper exec's. Each stub prints its argv (one arg per line) plus
# the resolved BAZEL_REAL, which the test then compares against expected.
#
# Run: ./tools/bazel-test.sh
# Run under macOS's bash 3.2 (the wrapper's compatibility floor):
#   WRAPPER_BASH=/bin/bash ./tools/bazel-test.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WRAPPER="$HERE/bazel"

# Build a stub directory containing fake `aspect` and `bazel` binaries.
STUB_DIR="$(mktemp -d)"
trap 'rm -rf "$STUB_DIR"' EXIT

cat > "$STUB_DIR/aspect" <<'EOF'
#!/usr/bin/env bash
echo "INVOKED:aspect"
for a in "$@"; do printf 'ARG:%s\n' "$a"; done
EOF
cat > "$STUB_DIR/bazel" <<'EOF'
#!/usr/bin/env bash
echo "INVOKED:bazel"
for a in "$@"; do printf 'ARG:%s\n' "$a"; done
EOF
chmod +x "$STUB_DIR/aspect" "$STUB_DIR/bazel"

export BAZEL_REAL="$STUB_DIR/bazel"
export PATH="$STUB_DIR:$PATH"

PASS=0
FAIL=0
FAILED_NAMES=()

check() {
    local name="$1"
    local expected="$2"
    local actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS  $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL  $name"
        echo "  expected:"
        echo "$expected" | sed 's/^/    /'
        echo "  actual:"
        echo "$actual" | sed 's/^/    /'
        FAIL=$((FAIL + 1))
        FAILED_NAMES+=("$name")
    fi
}

# How to invoke the wrapper. Set WRAPPER_BASH to a specific bash (e.g.
# /bin/bash, the bash 3.2 macOS ships) to drive the wrapper under that
# interpreter rather than its own `#!/usr/bin/env bash` shebang — catches
# 3.2-only regressions like empty-array expansion under `set -u`. Used as an
# array so it expands to either `<wrapper>` or `<bash> <wrapper>`.
if [[ -n "${WRAPPER_BASH:-}" ]]; then
    WRAPPER_CMD=("$WRAPPER_BASH" "$WRAPPER")
else
    WRAPPER_CMD=("$WRAPPER")
fi
run() {
    "${WRAPPER_CMD[@]}" "$@" 2>&1
}

# A PATH dir holding only `bazel` (no `aspect`), to exercise the
# aspect-not-installed branches. BAZEL_REAL still points at the stub bazel, so
# resolve_bazel_real succeeds without PATH lookup.
NO_ASPECT_DIR="$(mktemp -d)"
trap 'rm -rf "$STUB_DIR" "$NO_ASPECT_DIR"' EXIT
cp "$STUB_DIR/bazel" "$NO_ASPECT_DIR/bazel"

# Run the wrapper with `aspect` absent from PATH.
run_no_aspect() {
    PATH="$NO_ASPECT_DIR:/usr/bin:/bin" "${WRAPPER_CMD[@]}" "$@" 2>&1
}

# =====================================================================
# Section 1: Pure dispatch — verb routing without any flag complexity
# =====================================================================

check "dispatch: aspect verb 'lint' → aspect" \
"INVOKED:aspect
ARG:lint" \
"$(run lint)"

check "dispatch: aspect verb 'format' with target → aspect" \
"INVOKED:aspect
ARG:format
ARG://some:target" \
"$(run format //some:target)"

check "dispatch: aspect verb 'delivery' with multi-target → aspect" \
"INVOKED:aspect
ARG:delivery
ARG://a:b
ARG://c:d" \
"$(run delivery //a:b //c:d)"

check "dispatch: aspect verb 'configure' → aspect" \
"INVOKED:aspect
ARG:configure" \
"$(run configure)"

check "dispatch: aspect verb 'buildifier' → aspect" \
"INVOKED:aspect
ARG:buildifier" \
"$(run buildifier)"

check "dispatch: bazel verb 'query' stays on bazel" \
"INVOKED:bazel
ARG:query
ARG:deps(//foo)" \
"$(run query 'deps(//foo)')"

check "dispatch: bazel verb 'cquery' stays on bazel" \
"INVOKED:bazel
ARG:cquery
ARG://..." \
"$(run cquery //...)"

check "dispatch: bazel verb 'info' stays on bazel" \
"INVOKED:bazel
ARG:info
ARG:workspace" \
"$(run info workspace)"

check "dispatch: bazel verb 'clean' stays on bazel" \
"INVOKED:bazel
ARG:clean
ARG:--expunge" \
"$(run clean --expunge)"

check "dispatch: bazel verb 'mod' stays on bazel" \
"INVOKED:bazel
ARG:mod
ARG:graph" \
"$(run mod graph)"

check "dispatch: no verb at all → bazel" \
"INVOKED:bazel
ARG:--version" \
"$(run --version)"

check "dispatch: bare invocation → bazel" \
"INVOKED:bazel" \
"$(run)"

# =====================================================================
# Section 2: Common verb — bare cases
# =====================================================================

check "common: build alone → aspect build" \
"INVOKED:aspect
ARG:build" \
"$(run build)"

check "common: build with target → aspect build" \
"INVOKED:aspect
ARG:build
ARG://..." \
"$(run build //...)"

check "common: test with target → aspect test" \
"INVOKED:aspect
ARG:test
ARG://..." \
"$(run test //...)"

check "common: run with target → aspect run" \
"INVOKED:aspect
ARG:run
ARG://foo:bin" \
"$(run run //foo:bin)"

# =====================================================================
# Section 3: Post-verb flag rewriting — bazel flags
# =====================================================================

check "post-verb bazel: boolean wrapped (--keep_going)" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--keep_going
ARG://..." \
"$(run build --keep_going //...)"

check "post-verb bazel: boolean negation wrapped (--nobuild)" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--nobuild
ARG://..." \
"$(run build --nobuild //...)"

check "post-verb bazel: =value form wrapped (--config=ci)" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--config=ci
ARG://..." \
"$(run build --config=ci //...)"

check "post-verb bazel: space-value form glued (--config ci)" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--config=ci
ARG://..." \
"$(run build --config ci //...)"

check "post-verb bazel: repeated --config space-value" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--config=ci
ARG:--bazel-flag=--config=remote
ARG://..." \
"$(run build --config ci --config remote //...)"

check "post-verb bazel: --jobs with space-value (number)" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--jobs=4
ARG://..." \
"$(run build --jobs 4 //...)"

check "post-verb bazel: --jobs with =value" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--jobs=8
ARG://..." \
"$(run build --jobs=8 //...)"

check "post-verb bazel: --remote_executor with value containing colons" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--remote_executor=grpc://exec.example.com:443
ARG://..." \
"$(run build --remote_executor grpc://exec.example.com:443 //...)"

check "post-verb bazel: --define KEY=VAL" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--define=foo=bar
ARG://..." \
"$(run build --define foo=bar //...)"

check "post-verb bazel: --action_env with space-value" \
"INVOKED:aspect
ARG:test
ARG:--bazel-flag=--action_env=HOME=/tmp
ARG://..." \
"$(run test --action_env HOME=/tmp //...)"

check "post-verb bazel: --test_arg with =value containing flag-shaped value" \
"INVOKED:aspect
ARG:test
ARG:--bazel-flag=--test_arg=--verbose
ARG://..." \
"$(run test --test_arg=--verbose //...)"

check "post-verb bazel: --test_output with space-value" \
"INVOKED:aspect
ARG:test
ARG:--bazel-flag=--test_output=errors
ARG://..." \
"$(run test --test_output errors //...)"

check "post-verb bazel: --test_filter with regex value" \
"INVOKED:aspect
ARG:test
ARG:--bazel-flag=--test_filter=^Foo.*Bar$
ARG://..." \
"$(run test --test_filter '^Foo.*Bar$' //...)"

check "post-verb bazel: --copt with space-value" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--copt=-O2
ARG://..." \
"$(run build --copt -O2 //...)"

check "post-verb bazel: trailing value-taking flag with no value" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--config" \
"$(run build --config)"

check "post-verb: unknown flag (typo / unlisted) passes through to aspect" \
"INVOKED:aspect
ARG:build
ARG:--keep_goinggg
ARG://..." \
"$(run build --keep_goinggg //...)"

# =====================================================================
# Section 4: Post-verb flag passthrough — aspect flags
# =====================================================================

check "post-verb aspect: kebab-case --task-key=val passthrough" \
"INVOKED:aspect
ARG:build
ARG:--task-key=mybuild
ARG://..." \
"$(run build --task-key=mybuild //...)"

check "post-verb aspect: kebab-case --task-key val (space form) passthrough" \
"INVOKED:aspect
ARG:build
ARG:--task-key
ARG:mybuild
ARG://..." \
"$(run build --task-key mybuild //...)"

check "post-verb aspect: colon-namespaced --artifact-upload:enabled=false" \
"INVOKED:aspect
ARG:build
ARG:--artifact-upload:enabled=false
ARG://..." \
"$(run build --artifact-upload:enabled=false //...)"

check "post-verb aspect: colon-namespaced --github-status-checks:mode=always" \
"INVOKED:aspect
ARG:build
ARG:--github-status-checks:mode=always
ARG://..." \
"$(run build --github-status-checks:mode=always //...)"

check "post-verb aspect: --timing=summary" \
"INVOKED:aspect
ARG:build
ARG:--timing=summary
ARG://..." \
"$(run build --timing=summary //...)"

check "post-verb aspect: --bazel-flag=--foo passthrough (already wrapped)" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--foo
ARG://..." \
"$(run build --bazel-flag=--foo //...)"

check "post-verb aspect: --cancel boolean passthrough" \
"INVOKED:aspect
ARG:build
ARG:--cancel
ARG://..." \
"$(run build --cancel //...)"

check "post-verb aspect: --coverage (test-only) passthrough" \
"INVOKED:aspect
ARG:test
ARG:--coverage
ARG://..." \
"$(run test --coverage //...)"

# =====================================================================
# Section 5: Mixed aspect + bazel flags
# =====================================================================

check "mixed: aspect kebab + bazel boolean + bazel =value" \
"INVOKED:aspect
ARG:build
ARG:--task-key=mybuild
ARG:--bazel-flag=--keep_going
ARG:--bazel-flag=--config=ci
ARG://..." \
"$(run build --task-key=mybuild --keep_going --config=ci //...)"

check "mixed: bazel space-value sandwiched between aspect flags" \
"INVOKED:aspect
ARG:build
ARG:--task-key=mybuild
ARG:--bazel-flag=--config=ci
ARG:--timing=summary
ARG://..." \
"$(run build --task-key=mybuild --config ci --timing=summary //...)"

check "mixed: many flags in random order" \
"INVOKED:aspect
ARG:test
ARG:--bazel-flag=--keep_going
ARG:--task-key=t1
ARG:--bazel-flag=--config=ci
ARG:--coverage
ARG:--bazel-flag=--test_output=errors
ARG://..." \
"$(run test --keep_going --task-key=t1 --config ci --coverage --test_output errors //...)"

# =====================================================================
# Section 6: Pre-verb (startup) flag handling
# =====================================================================

check "pre-verb: bazel startup flag → --bazel-startup-flag= (after verb)" \
"INVOKED:aspect
ARG:build
ARG:--bazel-startup-flag=--output_base=/tmp/foo
ARG://..." \
"$(run --output_base=/tmp/foo build //...)"

check "pre-verb: bazel startup flag space-value → glued and wrapped" \
"INVOKED:aspect
ARG:build
ARG:--bazel-startup-flag=--output_base=/tmp/foo
ARG://..." \
"$(run --output_base /tmp/foo build //...)"

check "pre-verb: aspect global flag (--task-key) before verb → pre-verb aspect" \
"INVOKED:aspect
ARG:--task-key=t1
ARG:build
ARG://..." \
"$(run --task-key=t1 build //...)"

check "pre-verb: aspect global flag space-form (--task-key t1)" \
"INVOKED:aspect
ARG:--task-key
ARG:t1
ARG:build
ARG://..." \
"$(run --task-key t1 build //...)"

check "pre-verb: mixed aspect global + bazel startup" \
"INVOKED:aspect
ARG:--task-key=t1
ARG:build
ARG:--bazel-startup-flag=--output_base=/tmp/x
ARG://..." \
"$(run --task-key=t1 --output_base=/tmp/x build //...)"

check "pre-verb: aspect global + bazel startup → bazel startup goes after verb" \
"INVOKED:aspect
ARG:--task-key=t1
ARG:build
ARG:--bazel-startup-flag=--output_base=/tmp/x
ARG:--bazel-flag=--keep_going
ARG://..." \
"$(run --task-key=t1 --output_base=/tmp/x build --keep_going //...)"

check "pre-verb: aspect verb (lint) with pre-verb aspect global" \
"INVOKED:aspect
ARG:--task-key=t1
ARG:lint" \
"$(run --task-key=t1 lint)"

check "pre-verb: aspect verb (lint) with pre-verb bazel startup → moved post-verb" \
"INVOKED:aspect
ARG:lint
ARG:--bazel-startup-flag=--output_base=/tmp/x" \
"$(run --output_base=/tmp/x lint)"

# =====================================================================
# Section 7: Edge cases — `--`, positionals, hyphen-led targets
# =====================================================================

check "edge: -- ends flag parsing, positionals untouched" \
"INVOKED:aspect
ARG:build
ARG:--
ARG://...
ARG:-//experimental/..." \
"$(run build -- //... -//experimental/...)"

check "edge: -- with bazel flag before, both after" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--keep_going
ARG:--
ARG://...
ARG:-//experimental/..." \
"$(run build --keep_going -- //... -//experimental/...)"

check "edge: -- with run args (aspect flags after -- still pass through verbatim)" \
"INVOKED:aspect
ARG:run
ARG://foo:bin
ARG:--
ARG:--task-key
ARG:value
ARG:--keep_going" \
"$(run run //foo:bin -- --task-key value --keep_going)"

check "edge: target before any flag" \
"INVOKED:aspect
ARG:build
ARG://...
ARG:--bazel-flag=--keep_going" \
"$(run build //... --keep_going)"

check "edge: flag with empty value (--config=)" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--config=
ARG://..." \
"$(run build --config= //...)"

check "edge: flag with value containing spaces" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--workspace_status_command=/path/to/cmd arg1
ARG://..." \
"$(run build --workspace_status_command '/path/to/cmd arg1' //...)"

check "edge: target with embedded equals sign in label is positional, not flag" \
"INVOKED:aspect
ARG:build
ARG://foo:bar=baz" \
"$(run build //foo:bar=baz)"

# =====================================================================
# Section 8: BAZEL_REAL plumbing
# =====================================================================

# Swap in a stub that also reports BAZEL_REAL so we can assert it.
cat > "$STUB_DIR/aspect" <<'EOF'
#!/usr/bin/env bash
echo "INVOKED:aspect"
echo "BAZEL_REAL=${BAZEL_REAL:-<unset>}"
for a in "$@"; do printf 'ARG:%s\n' "$a"; done
EOF

check "env: BAZEL_REAL forwarded to aspect (aspect verb)" \
"INVOKED:aspect
BAZEL_REAL=$STUB_DIR/bazel
ARG:lint" \
"$(run lint)"

check "env: BAZEL_REAL forwarded to aspect (common verb)" \
"INVOKED:aspect
BAZEL_REAL=$STUB_DIR/bazel
ARG:build
ARG://..." \
"$(run build //...)"

# Anti-inception layer 1: when BAZEL_REAL is UNSET (real bazel execs the
# wrapper without it), the wrapper resolves bazel via PATH and re-exports it to
# aspect, so aspect's child bazel uses the real binary instead of re-entering
# the wrapper. Assert aspect receives a concrete BAZEL_REAL (the stub on PATH),
# not <unset>.
check "env: BAZEL_REAL resolved from PATH and forwarded when unset" \
"INVOKED:aspect
BAZEL_REAL=$STUB_DIR/bazel
ARG:build
ARG://..." \
"$(env -u BAZEL_REAL "${WRAPPER_CMD[@]}" build //... 2>&1)"

# Restore the plain stub so subsequent assertions stay clean.
cat > "$STUB_DIR/aspect" <<'EOF'
#!/usr/bin/env bash
echo "INVOKED:aspect"
for a in "$@"; do printf 'ARG:%s\n' "$a"; done
EOF

# =====================================================================
# Section 9: Trace output (ASPECT_WRAPPER_TRACE / ASPECT_WRAPPER_QUIET)
# =====================================================================

# Under `$()` capture, stderr-is-not-a-TTY, so trace must be silent by
# default. Earlier sections already implicitly assert this — if trace
# bled through, every "$(run ...)" assertion would fail. Just sanity-check.
check "trace: silent under non-TTY by default (aspect path)" \
"INVOKED:aspect
ARG:lint" \
"$(run lint 2>&1)"

check "trace: silent under non-TTY by default (bazel path)" \
"INVOKED:bazel
ARG:query
ARG:deps(//foo)" \
"$(run query 'deps(//foo)' 2>&1)"

# Force trace on. Expect the [tools/bazel] line on stderr, then aspect's
# normal output. Strip ANSI escape sequences so the assertion isn't tied
# to the exact escape encoding.
strip_ansi() { sed $'s/\033\\[[0-9;]*m//g'; }

actual="$(ASPECT_WRAPPER_TRACE=1 "$WRAPPER" lint 2>&1 | strip_ansi)"
check "trace: ASPECT_WRAPPER_TRACE=1 prints '[tools/bazel] ...' for aspect route" \
"[tools/bazel] aspect lint
INVOKED:aspect
ARG:lint" \
"$actual"

actual="$(ASPECT_WRAPPER_TRACE=1 "$WRAPPER" build --keep_going --config=ci //... 2>&1 | strip_ansi)"
check "trace: shows rewritten flags" \
"[tools/bazel] aspect build --bazel-flag=--keep_going --bazel-flag=--config=ci //...
INVOKED:aspect
ARG:build
ARG:--bazel-flag=--keep_going
ARG:--bazel-flag=--config=ci
ARG://..." \
"$actual"

actual="$(ASPECT_WRAPPER_TRACE=1 "$WRAPPER" query 'deps(//foo)' 2>&1 | strip_ansi)"
check "trace: ASPECT_WRAPPER_TRACE=1 also shows bazel-only verb forwarding" \
"[tools/bazel] $STUB_DIR/bazel query 'deps(//foo)'
INVOKED:bazel
ARG:query
ARG:deps(//foo)" \
"$actual"

actual="$(ASPECT_WRAPPER_QUIET=1 ASPECT_WRAPPER_TRACE=1 "$WRAPPER" lint 2>&1 | strip_ansi)"
check "trace: ASPECT_WRAPPER_QUIET=1 wins over TRACE=1" \
"INVOKED:aspect
ARG:lint" \
"$actual"

# =====================================================================
# Section 10: ASPECT_WRAPPER_SKIP — total bypass, everything → vanilla bazel
# =====================================================================

check "skip: build forwards straight to bazel" \
"INVOKED:bazel
ARG:build
ARG://..." \
"$(ASPECT_WRAPPER_SKIP=1 "$WRAPPER" build //... 2>&1)"

check "skip: test forwards straight to bazel" \
"INVOKED:bazel
ARG:test
ARG:--keep_going
ARG://..." \
"$(ASPECT_WRAPPER_SKIP=1 "$WRAPPER" test --keep_going //... 2>&1)"

check "skip: run forwards straight to bazel" \
"INVOKED:bazel
ARG:run
ARG://foo:bin
ARG:--
ARG:arg1" \
"$(ASPECT_WRAPPER_SKIP=1 "$WRAPPER" run //foo:bin -- arg1 2>&1)"

# Skip is a TOTAL bypass: even aspect verbs go straight to vanilla bazel,
# untouched. (The user reaches for `aspect <verb>` directly when they want
# the wrapped behavior.)
check "skip: aspect verb (lint) also forwards straight to bazel" \
"INVOKED:bazel
ARG:lint" \
"$(ASPECT_WRAPPER_SKIP=1 "$WRAPPER" lint 2>&1)"

check "skip: aspect verb (format) also forwards straight to bazel" \
"INVOKED:bazel
ARG:format
ARG://..." \
"$(ASPECT_WRAPPER_SKIP=1 "$WRAPPER" format //... 2>&1)"

# Custom/unknown verb also bypasses (no aspect routing under skip).
check "skip: custom verb also forwards straight to bazel" \
"INVOKED:bazel
ARG:mytask" \
"$(ASPECT_WRAPPER_SKIP=1 "$WRAPPER" mytask 2>&1)"

check "skip: query (bazel-only verb) still goes to bazel" \
"INVOKED:bazel
ARG:query
ARG://..." \
"$(ASPECT_WRAPPER_SKIP=1 "$WRAPPER" query //... 2>&1)"

# Skip + flag preservation: bazel sees the raw flags, no --bazel-flag= wrapping.
check "skip: bazel-native flags pass through unwrapped" \
"INVOKED:bazel
ARG:build
ARG:--keep_going
ARG:--config=ci
ARG://..." \
"$(ASPECT_WRAPPER_SKIP=1 "$WRAPPER" build --keep_going --config=ci //... 2>&1)"

# Skip + TRACE shows the bazel forward.
actual="$(ASPECT_WRAPPER_SKIP=1 ASPECT_WRAPPER_TRACE=1 "$WRAPPER" build //... 2>&1 | strip_ansi)"
check "skip: TRACE=1 shows bazel forward under skip mode" \
"[tools/bazel] $STUB_DIR/bazel build //...
INVOKED:bazel
ARG:build
ARG://..." \
"$actual"

# Empty string means unset — make sure we treat "" as no skip.
check "skip: empty string ASPECT_WRAPPER_SKIP='' is treated as unset" \
"INVOKED:aspect
ARG:build
ARG://..." \
"$(ASPECT_WRAPPER_SKIP="" "$WRAPPER" build //... 2>&1)"

# =====================================================================
# Section 11: Anti-inception (ASPECT_CLI_RUNNING bypass)
# =====================================================================
#
# Aspect sets ASPECT_CLI_RUNNING=1 on every child bazel it spawns. The
# wrapper checks for this BEFORE doing any routing and forwards straight
# to bazel — otherwise aspect → tools/bazel → aspect → tools/bazel → …
# recurses forever.

check "inception: ASPECT_CLI_RUNNING=1 + build → straight to bazel" \
"INVOKED:bazel
ARG:build
ARG://..." \
"$(ASPECT_CLI_RUNNING=1 "$WRAPPER" build //... 2>&1)"

check "inception: ASPECT_CLI_RUNNING=1 + bazel-native flags pass through unwrapped" \
"INVOKED:bazel
ARG:build
ARG:--keep_going
ARG:--config=ci
ARG://..." \
"$(ASPECT_CLI_RUNNING=1 "$WRAPPER" build --keep_going --config=ci //... 2>&1)"

# Even aspect-only verbs forward to bazel under ASPECT_CLI_RUNNING. Aspect
# would never spawn `bazel lint`, but if it somehow did we still want the
# bypass to win — aspect MUST NOT recurse into itself.
check "inception: ASPECT_CLI_RUNNING=1 wins over aspect-only verbs too" \
"INVOKED:bazel
ARG:lint" \
"$(ASPECT_CLI_RUNNING=1 "$WRAPPER" lint 2>&1)"

check "inception: ASPECT_CLI_RUNNING=1 + info → straight to bazel" \
"INVOKED:bazel
ARG:info
ARG:workspace" \
"$(ASPECT_CLI_RUNNING=1 "$WRAPPER" info workspace 2>&1)"

# Empty string means unset — wrapper must treat it the same as no env var.
check "inception: empty ASPECT_CLI_RUNNING='' is treated as unset" \
"INVOKED:aspect
ARG:build
ARG://..." \
"$(ASPECT_CLI_RUNNING="" "$WRAPPER" build //... 2>&1)"

# Bypass should fire BEFORE the trace logic. Even with TRACE=1, the bypass
# runs and we get straight bazel — no aspect-route trace line.
actual="$(ASPECT_CLI_RUNNING=1 ASPECT_WRAPPER_TRACE=1 "$WRAPPER" build //... 2>&1 | strip_ansi)"
# Bypass exec's bazel directly without calling trace_exec, so TRACE has
# no effect. This is intentional: in the inception case the trace would
# fire INSIDE aspect's output stream and confuse the user.
check "inception: bypass runs before trace, even TRACE=1 is silent" \
"INVOKED:bazel
ARG:build
ARG://..." \
"$actual"

# =====================================================================
# Section 12: Bazel short-flag abbreviations that take a value (-c, -j)
# =====================================================================

check "short: -c opt (space) → --bazel-flag=-c=opt, opt not a target" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=-c=opt
ARG://..." \
"$(run build -c opt //...)"

check "short: -j 8 (space) → --bazel-flag=-j=8" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=-j=8
ARG://..." \
"$(run build -j 8 //...)"

check "short: -c=opt (already glued) → single token, unchanged" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=-c=opt
ARG://..." \
"$(run build -c=opt //...)"

check "short: boolean shorts (-k -s) stay single tokens, don't eat next arg" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=-k
ARG:--bazel-flag=-s
ARG://..." \
"$(run build -k -s //...)"

check "short: -c at end of argv (no value) → passes as single token" \
"INVOKED:aspect
ARG:build
ARG://...
ARG:--bazel-flag=-c" \
"$(run build //... -c)"

# =====================================================================
# Section 13: Aspect-verb bazel-flag forwarding (ASPECT_VERBS_WITH_BAZEL_FLAGS)
# =====================================================================

check "fwd: format --keep_going → aspect format --bazel-flag=--keep_going" \
"INVOKED:aspect
ARG:format
ARG:--bazel-flag=--keep_going" \
"$(run format --keep_going)"

check "fwd: lint space-value flag rewritten, aspect flag preserved" \
"INVOKED:aspect
ARG:lint
ARG:--github-lint-comments:enabled=true
ARG:--bazel-flag=--config=ci" \
"$(run lint --github-lint-comments:enabled=true --config ci)"

# =====================================================================
# Section 15: Closed-set model — bazel is the known set, rest → aspect
# =====================================================================

# Unknown verb (custom .axl task): routes to aspect, args verbatim (no
# bazel-flag rewriting — not in ASPECT_VERBS_WITH_BAZEL_FLAGS).
check "model: unknown verb (custom task) → aspect, args verbatim" \
"INVOKED:aspect
ARG:mytask
ARG:--keep_going
ARG://x" \
"$(run mytask --keep_going //x)"

# Bazel verb that aspect does NOT wrap → vanilla bazel untouched.
check "model: bazel verb 'coverage' (not wrapped) → vanilla bazel" \
"INVOKED:bazel
ARG:coverage
ARG://..." \
"$(run coverage //...)"

# Boolean negation (--no<flag>) is recognized as a bazel flag and wrapped.
check "model: --nokeep_going recognized as bazel boolean → wrapped" \
"INVOKED:aspect
ARG:build
ARG:--bazel-flag=--nokeep_going
ARG://..." \
"$(run build --nokeep_going //...)"

# A flag in no bazel list (aspect feature flag) passes through to aspect.
check "model: aspect feature flag (colon-namespaced) passes through" \
"INVOKED:aspect
ARG:build
ARG:--artifact-upload:enabled=false
ARG://..." \
"$(run build --artifact-upload:enabled=false //...)"

# =====================================================================
# Section 14: PATH fallback when BAZEL_REAL is unset (type -aP, not zsh)
# =====================================================================

# With BAZEL_REAL unset, resolve_bazel_real must find the stub bazel on PATH
# (skipping the wrapper itself). Regression test for the old `command -v -a`
# zsh-ism that exits non-zero under bash and found nothing.
check "path: BAZEL_REAL unset → finds real bazel on PATH" \
"INVOKED:bazel
ARG:version" \
"$(env -u BAZEL_REAL "${WRAPPER_CMD[@]}" version 2>&1)"

check "path: ASPECT_CLI_RUNNING + BAZEL_REAL unset → finds real bazel on PATH" \
"INVOKED:bazel
ARG:build
ARG://..." \
"$(env -u BAZEL_REAL ASPECT_CLI_RUNNING=1 "${WRAPPER_CMD[@]}" build //... 2>&1)"

# =====================================================================
# Section 16: aspect not installed — install hint + graceful fallback
# =====================================================================

# The verb-specific install hint, mirroring aspect_missing_hint in the wrapper.
hint() {
    printf '[tools/bazel] `bazel %s` runs through the Aspect CLI (`aspect`), which is not on your PATH.

  Install it with:

    curl -fsSL https://install.aspect.build | bash

  or see https://docs.aspect.build/cli/install' "$1"
}

# Verb that is BOTH aspect-wrapped and a real bazel command (build): hint with
# a fallback note, then run the command on vanilla bazel.
check "no-aspect: build → install hint then falls back to bazel" \
"$(hint build)
  Running it through vanilla bazel for now.
INVOKED:bazel
ARG:build
ARG:--keep_going
ARG://..." \
"$(run_no_aspect build --keep_going //...)"

# Aspect-only wrapped verb (lint): bazel can't run it, so hint and stop.
check "no-aspect: lint (aspect-only) → install hint, no bazel fallback" \
"$(hint lint)" \
"$(run_no_aspect lint 2>&1)"

# Custom/unknown verb: aspect-only, hint and stop.
check "no-aspect: custom verb → install hint, no bazel fallback" \
"$(hint mytask)" \
"$(run_no_aspect mytask //x 2>&1)"

# Plain bazel verb: never needs aspect, runs vanilla with no hint.
check "no-aspect: query (bazel verb) → vanilla bazel, no hint" \
"INVOKED:bazel
ARG:query
ARG://..." \
"$(run_no_aspect query //...)"

# =====================================================================
# Summary
# =====================================================================

echo
echo "$PASS passed, $FAIL failed"
if [[ $FAIL -gt 0 ]]; then
    echo "Failed tests:"
    printf '  %s\n' "${FAILED_NAMES[@]}"
fi
[[ $FAIL -eq 0 ]]
