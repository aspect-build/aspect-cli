#!/usr/bin/env bash
# Regenerate one failure fixture: record.sh <case> <bazel-version>
#
# Copies the fixture's src/ (its Bazel files stored with a `.in` suffix, so neither this
# repository's Bazel nor its formatters take them for its own) into a fixed scratch workspace
# (one Bazel server serves every case of a version), runs `bazel <cmd>` with its stderr on a
# pty and colour forced, then scrubs machine paths and copies the stderr/stdout/test logs the
# BEP names into the fixture (layout and placeholders: the fixtures' README.md). How Bazel is
# found and the environment it runs in: env.sh. Scratch state lives under $FIXTURE_SCRATCH
# (default ${TMPDIR:-/tmp}/aspect-failure-fixtures).
# Variants listed in fixtures/<case>/variants: noall (rerun without
# --build_event_publish_all_actions), nocolor (rerun with --color=no --curses=no),
# merged (rerun with Bazel's stdout on the pty too, as a terminal shares it with stderr),
# console_only (no BEP; 90s cap, then the server is killed - for the OOM case).
# The server is left running for the next case; `refresh_all.sh` shuts it down per version.
set -u
case_=$1
ver=$2
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=env.sh
. "$HERE/env.sh"
FX=$FIXTURES/$case_
RUN=$SCRATCH/runws
RAW=$SCRATCH/raw/$case_.$ver
[ -d "$FX/src" ] || {
    echo "no fixture $FX" >&2
    exit 2
}
mkdir -p "$RUN" "$FAKEHOME"
bazel_env "$ver"
STARTUP="--nosystem_rc"
find "$RUN" -mindepth 1 -maxdepth 1 ! -name 'bazel-*' -exec rm -rf {} +
cp -R "$FX/src/." "$RUN/"
find "$RUN" -name '*.in' ! -path '*/bazel-*' -exec sh -c 'mv "$1" "${1%.in}"' sh {} \;
cd "$RUN" || exit 2
variants=$(cat "$FX/variants" 2>/dev/null)
console_only=0
case " $variants " in *" console_only "*) console_only=1 ;; esac
OB=$("${BAZEL_ENV[@]}" "$BAZELISK" $STARTUP info output_base 2>/dev/null)
IB=$("${BAZEL_ENV[@]}" "$BAZELISK" $STARTUP info install_base 2>/dev/null)
one() { # <prefix> <extra flags> <strip publish_all: 0|1> [merged]
    local prefix=$1 flags=$2 strip=$3 merged=${4:-} args cap=300 bep tostdout
    tostdout='>"$out/stdout"'
    [ -n "$merged" ] && tostdout=
    args=$(cat "$FX/cmd")
    [ "$strip" = 1 ] && args=${args//--build_event_publish_all_actions/}
    rm -rf "$RAW" "$FX/$prefix".{bep.json,console,stdout,exit,files}
    mkdir -p "$RAW"
    rm -rf /tmp/bepfx-flaky.marker /tmp/bepfx-rpt.lock
    bep="--build_event_json_file=$RAW/bep.json"
    if [ $console_only = 1 ]; then
        bep=""
        cap=90
    else
        # Every recording runs its actions again, the successful ones' output
        # included: Bazel's action cache would replay none of it (an
        # `--action_env` reaches only actions that take the default shell
        # environment). Then a warm-up: module resolution and repo fetches
        # would otherwise show only in the first case.
        perl -e 'alarm 300; exec @ARGV' "${BAZEL_ENV[@]}" "$BAZELISK" $STARTUP clean >/dev/null 2>&1 </dev/null
        # shellcheck disable=SC2086
        perl -e 'alarm 300; exec @ARGV' "${BAZEL_ENV[@]}" "$BAZELISK" $STARTUP $args --disk_cache= --lockfile_mode=off --nobuild >/dev/null 2>&1 </dev/null
    fi
    # Bazel's stdout and exit code to their own files (merged: its stdout on
    # the pty with its stderr), its stderr on the pty.
    # shellcheck disable=SC2086
    on_pty "$RAW/console" perl -e "alarm $cap; exec @ARGV" "${BAZEL_ENV[@]}" sh -c 'out=$1; shift; "$@" '"$tostdout"'; echo $? >"$out/exit"' \
        sh "$RAW" "$BAZELISK" $STARTUP $args --disk_cache= --lockfile_mode=off $flags $bep </dev/null >/dev/null 2>&1
    if [ $console_only = 1 ]; then
        kill -9 "$(cat "$OB/server/server.pid.txt" 2>/dev/null)" 2>/dev/null
        [ -s "$RAW/exit" ] || echo "timeout" >"$RAW/exit"
    fi
    # It refuses (and so does this) a recording that still names this
    # machine's host, user or home.
    python3 -I "$HERE/fxpost.py" "$FX" "$prefix" "$RAW" "$OB" "$IB" "$RUN" "$FAKEHOME" || {
        echo "record.sh: $case_ $prefix not scrubbed; not kept" >&2
        rm -rf "$FX/$prefix".{bep.json,console,stdout,exit,files}
        exit 3
    }
    echo "$case_ $ver $prefix exit=$(cat "$FX/$prefix.exit" 2>/dev/null)"
}
one "$ver" "--color=yes --curses=yes" 0
case " $variants " in *" noall "*) one "$ver.noall" "--color=yes --curses=yes" 1 ;; esac
case " $variants " in *" nocolor "*) one "$ver.nocolor" "--color=no --curses=no" 0 ;; esac
case " $variants " in *" merged "*) one "$ver.merged" "--color=yes --curses=yes" 0 merged ;; esac
rm -rf "$RAW"
