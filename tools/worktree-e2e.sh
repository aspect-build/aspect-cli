#!/usr/bin/env bash
#
# End-to-end checks for `aspect worktree`: the command-level flows the AXL unit
# suites cannot reach, because they need several CLI processes, sessions that
# stop and resume, and clones that move.
#
# Everything runs in a scratch HOME against a local bare remote, so nothing on
# the machine is touched. Each call runs under `env -i` with an explicit Claude
# Code session (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_PID` pointing at
# a `sleep` this script owns), so identity is the same on a laptop, inside an
# agent, and on CI. Assertions are on exit status and the `--output=json`
# document, never on prose.
#
# Run: ./tools/worktree-e2e.sh [path-to-aspect-cli]
set -euo pipefail

CLI="${1:-aspect-cli}"
CLI="$(command -v "$CLI" || echo "$CLI")"
CLI="$(realpath "$CLI")"

ROOT="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/worktree-e2e.XXXXXX")"
ROOT="$(cd "$ROOT" && pwd -P)"
H="$ROOT/home"
SLEEPS=()
# The scratch tree is kept when the run fails, as the evidence of why.
cleanup() {
    local status=$?
    for pid in "${SLEEPS[@]}"; do kill "$pid" 2>/dev/null || true; done
    if [[ "$status" -eq 0 && "${FAILED:-0}" -eq 0 ]]; then
        rm -rf "$ROOT"
    else
        echo "kept $ROOT for inspection" >&2
    fi
}
trap cleanup EXIT
mkdir -p "$H"

FAILED=0
pass() { echo "  ok   $1"; }
fail() {
    echo "  FAIL $1" >&2
    FAILED=$((FAILED + 1))
}
check() { # label, actual, expected
    if [[ "$2" == "$3" ]]; then pass "$1"; else fail "$1: got '$2', want '$3'"; fi
}
section() { echo "--- $1"; }

git_env=(GIT_AUTHOR_NAME=e2e GIT_AUTHOR_EMAIL=e2e@example.test GIT_COMMITTER_NAME=e2e GIT_COMMITTER_EMAIL=e2e@example.test)
g() { env -i HOME="$H" PATH="$PATH" "${git_env[@]}" git "$@"; }

# A live process standing in for a session's harness, its pid in $SESSION_PID.
# Not a `$(...)`: the `sleep` would hold the substitution's pipe open, and its
# pid would never reach SLEEPS for the cleanup.
new_session() {
    sleep 3600 >/dev/null 2>&1 &
    SESSION_PID=$!
    SLEEPS+=("$SESSION_PID")
}

# Stop a session's process, as a crash or a closed terminal would.
end_session() {
    kill "$1" 2>/dev/null || true
    wait "$1" 2>/dev/null || true
}

# `aspect worktree <args>` as session $1 (process $2), in the current directory.
as() {
    local sid="$1" pid="$2"
    shift 2
    env -i HOME="$H" PATH="$PATH" DO_NOT_TRACK=1 "${git_env[@]}" \
        CLAUDECODE=1 CLAUDE_CODE_SESSION_ID="$sid" CLAUDE_PID="$pid" \
        "$CLI" worktree "$@"
}

# The `--output=json` document of a call, whatever its exit status.
json() {
    as "$@" --output=json 2>/dev/null || true
}

# The `error` token of a call, or "ok".
outcome() {
    json "$@" | jq -r '.error // "ok"'
}

# --- a repository --------------------------------------------------------
g init -q --bare -b main "$ROOT/remote.git"
g clone -q "$ROOT/remote.git" "$ROOT/repo" 2>/dev/null
cd "$ROOT/repo"
echo hi >README
printf '*.log\n' >.gitignore
g add -A
g commit -qm init
g push -q origin HEAD:main 2>/dev/null

A=sessA
new_session
A_PID="$SESSION_PID"
B=sessB
new_session
B_PID="$SESSION_PID"

section "a worktree's life: add, work, release, reuse"
doc="$(json "$A" "$A_PID" add feat/a --create=origin/main)"
slot="$(jq -r .slot <<<"$doc")"
path="$(jq -r .path <<<"$doc")"
check "add --create gives a slot" "$([[ -d "$path" ]] && echo yes)" "yes"
g -C "$path" commit -q --allow-empty -m work
g -C "$path" push -q -u origin feat/a 2>/dev/null
check "release" "$(outcome "$A" "$A_PID" release feat/a)" "ok"
check "the slot is free" "$(json "$A" "$A_PID" list | jq -r --arg s "$slot" '[.pools[].slots[] | select(.slot == $s)][0].state')" "free"
doc="$(json "$A" "$A_PID" add feat/a)"
check "the branch comes back to its slot" "$(jq -r .slot <<<"$doc")" "$slot"
check "reused_from names it" "$(jq -r .reused_from <<<"$doc")" "feat/a"

section "inspect: the slot named, or the one you are in"
doc="$(json "$A" "$A_PID" inspect feat/a)"
check "named from the clone" "$(jq -r '.slot.slot + " " + (.in_worktree | tostring)' <<<"$doc")" "$slot false"
doc="$(cd "$path" && json "$A" "$A_PID" inspect)"
check "unnamed from inside it" "$(jq -r '.branch + " " + (.in_worktree | tostring)' <<<"$doc")" "feat/a true"
check "another session may read it" "$(json "$B" "$B_PID" inspect feat/a | jq -r .held_by_this_session)" "false"
check "unnamed in the clone is an answer" "$(json "$A" "$A_PID" inspect | jq -r .location)" "clone"
check "a name nothing answers to" "$(outcome "$A" "$A_PID" inspect no/such)" "no_such_worktree"

section "release from inside the slot, naming nothing"
check "outside any slot, there is nothing to release" "$(outcome "$A" "$A_PID" release)" "no_such_worktree"
mkdir -p "$path/sub/dir"
doc="$(cd "$path/sub/dir" && json "$A" "$A_PID" release)"
check "from a subdirectory, the slot it is in" "$(jq -r '.released + " " + (.cwd_removed | tostring)' <<<"$doc")" "feat/a true"
json "$A" "$A_PID" add feat/a >/dev/null

section "work is protected; ignored files are not work"
echo wip >"$path/wip.txt"
check "uncommitted work refuses release" "$(outcome "$A" "$A_PID" release feat/a)" "worktree_dirty"
rm "$path/wip.txt"
echo log >"$path/build.log"
check "an ignored file does not" "$(outcome "$A" "$A_PID" release feat/a)" "ok"
check "a free slot inspects, with no checkout" "$(json "$A" "$A_PID" inspect "$slot" | jq -r '.slot.state + " [" + .head + "]"')" "free []"

section "commits a detached HEAD left behind"
doc="$(json "$A" "$A_PID" add main --detach)"
dslot="$(jq -r .slot <<<"$doc")"
dpath="$(jq -r .path <<<"$doc")"
g -C "$dpath" commit -q --allow-empty -m lost
tip="$(g -C "$dpath" rev-parse HEAD)"
g -C "$dpath" checkout -q --detach HEAD~1
doc="$(json "$A" "$A_PID" release "$dslot")"
check "release refuses" "$(jq -r .error <<<"$doc")" "unreferenced_commits"
check "and names the tip" "$(jq -r '.tips[0]' <<<"$doc")" "$tip"
g branch kept "$tip"
check "kept on a branch, it releases" "$(outcome "$A" "$A_PID" release "$dslot")" "ok"

section "a session and its subagents"
json "$A" "$A_PID" add t1 --create=origin/main --agent-id=sessA-t1 >/dev/null
json "$A" "$A_PID" add t2 --create=origin/main --agent-id=sessA-t2 >/dev/null
doc="$(json "$A" "$A_PID" release t1 --agent-id=sessA-t2 --force=all)"
check "a sibling cannot release, whatever the flag" "$(jq -r .error <<<"$doc")" "held_by_another_session"
check "and is told it is the same session" "$(jq -r .same_session <<<"$doc")" "true"
doc="$(json "$A" "$A_PID" path t1 --agent-id=sessA-t2)"
check "a sibling is not handed the path" "$(jq -r '.error + " " + (has("path") | tostring)' <<<"$doc")" "held_by_another_session false"
check "the session itself inspect lists both" "$(json "$A" "$A_PID" inspect | jq '[.held[] | select(.agent_id != "")] | length')" "2"
check "the session itself releases its subagent's slot" "$(outcome "$A" "$A_PID" release t1)" "ok"
check "another session cannot" "$(outcome "$B" "$B_PID" release t2)" "held_by_another_session"

section "a session that stops, resumes, or does not"
mkdir -p .aspect
printf 'load("@aspect//traits.axl", "Worktrees")\ndef config(ctx):\n    ctx.traits[Worktrees].abandoned_grace_hours = 0\n' >.aspect/config.axl
end_session "$A_PID"
new_session
A_PID="$SESSION_PID"
json "$A" "$A_PID" inspect >/dev/null
doc="$(json "$B" "$B_PID" add t2)"
check "a resumed session keeps its subagent's slot" "$(jq -r '.error + " " + .process' <<<"$doc")" "branch_in_use running"
end_session "$A_PID"
check "once it has stopped, the clean slot is taken back" "$(outcome "$B" "$B_PID" add t2)" "ok"
new_session
A_PID="$SESSION_PID"
check "and the session is told its lease ended" "$(json "$A" "$A_PID" inspect | jq -r '[.ended[].branch] | index("t2") != null')" "true"
check "release the taken slot" "$(outcome "$B" "$B_PID" release t2)" "ok"
rm -rf .aspect

section "a stopped session's lease, within its grace"
check "the session takes a slot" "$(outcome "$A" "$A_PID" add g1 --create=origin/main)" "ok"
end_session "$A_PID"
doc="$(json "$B" "$B_PID" release g1)"
check "another session cannot release it" "$(jq -r '.error + " " + (.grace_left_ms > 0 | tostring)' <<<"$doc")" "held_by_another_session true"
doc="$(json "$B" "$B_PID" release g1 --force=all)"
check "--force=all can, and says so" "$(jq -r '.released + " " + ([.warnings[].code] | index("overrode_another_session") != null | tostring)' <<<"$doc")" "g1 true"
new_session
A_PID="$SESSION_PID"

section "a clone moved away, and another cloned in its place"
doc="$(json "$A" "$A_PID" add mv1 --create=origin/main)"
mpath="$(jq -r .path <<<"$doc")"
echo keep >"$mpath/keep.txt"
cd "$ROOT"
mv repo repo-moved
g clone -q "$ROOT/remote.git" "$ROOT/repo" 2>/dev/null
cd "$ROOT/repo"
check "the new clone refuses the moved clone's slot" "$(outcome "$A" "$A_PID" release mv1 --force=all)" "slot_stranded"
check "and its work is untouched" "$(cat "$mpath/keep.txt")" "keep"
cd "$ROOT/repo-moved"
check "the moved clone takes it back" "$(json "$A" "$A_PID" path mv1 | jq -r .path)" "$mpath"

section "a slot whose directory is deleted"
g -C "$mpath" checkout -q --detach
g -C "$mpath" commit -q --allow-empty -m gone
g -C "$mpath" checkout -q mv1
rm -rf "$mpath"
doc="$(json "$A" "$A_PID" release mv1)"
check "release refuses what git still keeps" "$(jq -r '.error + " " + .uncommitted[0].status' <<<"$doc")" "worktree_dirty VG"
check "--force discards it" "$(outcome "$A" "$A_PID" release mv1 --force)" "ok"
check "and frees the branch in git" "$(g worktree list | grep -c "$mpath" || true)" "0"

section "a worktree nested in a slot"
doc="$(json "$A" "$A_PID" add n1 --create=origin/main)"
npath="$(jq -r .path <<<"$doc")"
printf '.claude/\n' >>"$npath/.gitignore"
g -C "$npath" commit -qam ignore
g -C "$npath" worktree add -q --detach "$npath/.claude/worktrees/nested"
check "release refuses it" "$(json "$A" "$A_PID" release n1 | jq -r '[.uncommitted[].status] | index("NW") != null')" "true"
g -C "$npath" worktree remove --force "$npath/.claude/worktrees/nested"
check "removed by hand, release goes" "$(outcome "$A" "$A_PID" release n1)" "ok"

section "something other than a checkout at a free slot's path"
nslot="$(json "$A" "$A_PID" list | jq -r '[.pools[].slots[] | select(.state == "free" and .owned_here)][0].slot')"
check "there is a free slot to use" "$([[ "$nslot" != null && -n "$nslot" ]] && echo yes)" "yes"
npath="$(json "$A" "$A_PID" list | jq -r --arg s "$nslot" '[.pools[].slots[] | select(.slot == $s)][0].path')"
[[ "$npath" == "$H"/* ]] || {
    echo "unexpected slot path '$npath'" >&2
    exit 1
}
rm -rf "$npath"
echo precious >"$npath"
check "prune refuses a file there" "$(outcome "$A" "$A_PID" prune "$nslot" --force)" "worktree_dirty"
check "and it is still there" "$(cat "$npath")" "precious"
rm -f "$npath"

section "another repository's slot, named from elsewhere"
g init -q --bare -b main "$ROOT/other.git"
g clone -q "$ROOT/other.git" "$ROOT/other" 2>/dev/null
cd "$ROOT/other"
echo other >README
g add -A
g commit -qm init
g push -q origin HEAD:main 2>/dev/null
oslot="$(json "$A" "$A_PID" add o1 --create=origin/main | jq -r .slot)"
oslot2="$(json "$A" "$A_PID" add o2 --create=origin/main | jq -r .slot)"
cd "$ROOT/repo"
check "release it from this repository's clone" "$(json "$A" "$A_PID" release "$oslot" | jq -r .released)" "o1"
check "prune it from here too" "$(outcome "$A" "$A_PID" prune "$oslot" --force)" "ok"
check "and it is gone" "$(json "$A" "$A_PID" list --all | jq --arg s "$oslot" '[.pools[].slots[] | select(.slot == $s)] | length')" "0"
cd "$ROOT"
check "release one from outside any clone" "$(json "$A" "$A_PID" release "$oslot2" | jq -r .released)" "o2"
cd "$ROOT/repo"

# A branch of this clone that looks like the start of another repository's
# slot id is still this clone's branch.
hexname="${oslot2:0:6}"
mine="$(json "$A" "$A_PID" add "$hexname" --create=origin/main | jq -r .slot)"
json "$A" "$A_PID" release "$hexname" >/dev/null
check "a hex-looking branch name means this clone's slot" "$(json "$A" "$A_PID" prune "$hexname" --dry-run | jq -r '.candidates[0].slot')" "$mine"

# Another clone of the same repository.
g clone -q "$ROOT/remote.git" "$ROOT/repo2" 2>/dev/null
cd "$ROOT/repo2"
cslot="$(json "$A" "$A_PID" add c1 --create=origin/main | jq -r .slot)"
cd "$ROOT/repo"
check "release a sibling clone's slot by id" "$(json "$A" "$A_PID" release "$cslot" | jq -r .released)" "c1"

section "machine output"
bidi="$(printf 'bi\xe2\x80\xaedi')"
out="$(as "$A" "$A_PID" add "$bidi" --create=origin/main --output=path 2>"$ROOT/stderr")"
check "--output=path is one line" "$(wc -l <<<"$out" | tr -d ' ')" "1"
check "and that line is the slot" "$([[ -d "$out" ]] && echo yes)" "yes"
check "the report goes to stderr" "$(grep -c 'worktree ready' "$ROOT/stderr")" "1"
doc="$(json "$B" "$B_PID" add "$bidi")"
check "another session is refused it" "$(jq -r .error <<<"$doc")" "branch_in_use"
check "and the message escapes what a terminal would act on" "$(jq -r .message <<<"$doc" | grep -c $'\xe2\x80\xae' || true)" "0"

echo
if [[ "$FAILED" -ne 0 ]]; then
    echo "$FAILED check(s) failed" >&2
    exit 1
fi
echo "all worktree end-to-end checks passed"
