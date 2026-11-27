#!/usr/bin/env bash
# Regenerate one failure fixture: record.sh <case> <bazel-version>
#
# Copies the fixture's src/ (its Bazel files stored with a `.in` suffix, so neither this
# repository's Bazel nor its formatters take them for its own) into a fixed scratch workspace
# (one Bazel server serves every case of a version), runs `bazel <cmd>` with its stderr on a
# pty and colour forced, then scrubs machine paths and copies the stderr/stdout/test logs the
# BEP names into the fixture (layout and placeholders: the fixtures' README.md). Bazel runs
# under `env -i` with a throwaway HOME so neither the user's rc files nor their environment
# (tokens included) reach the recorded command line. Scratch state lives under
# $FIXTURE_SCRATCH (default ${TMPDIR:-/tmp}/aspect-failure-fixtures).
# Variants listed in fixtures/<case>/variants: noall (rerun without
# --build_event_publish_all_actions), nocolor (rerun with --color=no --curses=no),
# console_only (no BEP; 90s cap, then the server is killed - for the OOM case).
# The server is left running for the next case; `refresh_all.sh` shuts it down per version.
set -u
case_=$1
ver=$2
HERE=$(cd "$(dirname "$0")" && pwd)
FIXTURES=$(cd "$HERE/../../crates/aspect-cli/tests/testdata/failures" && pwd)
SCRATCH=${FIXTURE_SCRATCH:-${TMPDIR:-/tmp}/aspect-failure-fixtures}
FX=$FIXTURES/$case_
RUN=$SCRATCH/runws
RAW=$SCRATCH/raw/$case_.$ver
FAKEHOME=$SCRATCH/home
[ -d "$FX/src" ] || {
    echo "no fixture $FX" >&2
    exit 2
}
mkdir -p "$RUN" "$FAKEHOME"
ENV="env -i HOME=$FAKEHOME USER=$USER LOGNAME=$USER TERM=xterm-256color BAZELISK_HOME=$HOME/Library/Caches/bazelisk PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin USE_BAZEL_VERSION=$ver"
STARTUP="--nosystem_rc"
find "$RUN" -mindepth 1 -maxdepth 1 ! -name 'bazel-*' -exec rm -rf {} +
cp -R "$FX/src/." "$RUN/"
find "$RUN" -name '*.in' ! -path '*/bazel-*' -exec sh -c 'mv "$1" "${1%.in}"' sh {} \;
cd "$RUN"
variants=$(cat "$FX/variants" 2>/dev/null)
console_only=0
case " $variants " in *" console_only "*) console_only=1 ;; esac
OB=$($ENV bazel $STARTUP info output_base 2>/dev/null)
IB=$($ENV bazel $STARTUP info install_base 2>/dev/null)
one() { # <prefix> <extra flags> <strip publish_all: 0|1>
    local prefix=$1 flags=$2 strip=$3 args cap=300 bep
    args=$(cat "$FX/cmd")
    [ "$strip" = 1 ] && args=${args//--build_event_publish_all_actions/}
    rm -rf "$RAW" "$FX/$prefix".*
    mkdir -p "$RAW"
    rm -rf /tmp/bepfx-flaky.marker /tmp/bepfx-rpt.lock
    bep="--build_event_json_file=$RAW/bep.json"
    if [ $console_only = 1 ]; then
        bep=""
        cap=90
    else
        # Warm-up: module resolution and repo fetches would otherwise show only in the first case.
        # shellcheck disable=SC2086
        perl -e 'alarm 300; exec @ARGV' $ENV bazel $STARTUP $args --disk_cache= --lockfile_mode=off --nobuild >/dev/null 2>&1 </dev/null
    fi
    # shellcheck disable=SC2086
    perl -e "alarm $cap; exec @ARGV" $ENV script -q "$RAW/console" sh -c \
        'stty cols 120 rows 40 2>/dev/null; bazel "$@" >"'"$RAW"'/stdout"; echo $? >"'"$RAW"'/exit"' \
        sh $STARTUP $args --disk_cache= --lockfile_mode=off $flags $bep </dev/null >/dev/null 2>&1
    if [ $console_only = 1 ]; then
        kill -9 "$(cat "$OB/server/server.pid.txt" 2>/dev/null)" 2>/dev/null
        [ -s "$RAW/exit" ] || echo "timeout" >"$RAW/exit"
    fi
    python3 -I "$HERE/fxpost.py" "$FX" "$prefix" "$RAW" "$OB" "$IB" "$RUN" "$FAKEHOME"
    echo "$case_ $ver $prefix exit=$(cat "$FX/$prefix.exit" 2>/dev/null)"
}
one "$ver" "--color=yes --curses=yes" 0
case " $variants " in *" noall "*) one "$ver.noall" "--color=yes --curses=yes" 1 ;; esac
case " $variants " in *" nocolor "*) one "$ver.nocolor" "--color=no --curses=no" 0 ;; esac
rm -rf "$RAW"
