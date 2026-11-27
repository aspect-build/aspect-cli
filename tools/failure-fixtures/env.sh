# shellcheck shell=bash
# Sourced by record.sh and refresh_all.sh: where the fixtures and the scratch
# state live, and how Bazel is run for a recording.
#
# Bazel is Bazelisk, from $BAZELISK, else the first `bazelisk` (then `bazel`)
# on PATH. It runs under `env -i` with a throwaway HOME, so neither the user's
# rc files nor their environment (tokens included) reach a recording; what it
# gets is only: that HOME, USER and LOGNAME (scrubbed from the recordings),
# TERM, a PATH of Bazelisk's own directory and the system's, the Bazel version
# to use, and BAZELISK_HOME, Bazelisk's cache: $BAZELISK_HOME if set, else
# where Bazelisk keeps it by default (the user cache directory's `bazelisk`:
# ~/Library/Caches on macOS, $XDG_CACHE_HOME or ~/.cache elsewhere), so the
# throwaway HOME does not make it download every Bazel again.

FIXTURES=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../crates/aspect-cli/tests/testdata/failures" && pwd)
SCRATCH=${FIXTURE_SCRATCH:-${TMPDIR:-/tmp}/aspect-failure-fixtures}
FAKEHOME=$SCRATCH/home

BAZELISK=${BAZELISK:-$(command -v bazelisk || command -v bazel || true)}
if [ -z "$BAZELISK" ] || [ ! -x "$BAZELISK" ]; then
    echo "no Bazelisk: put bazelisk on PATH or set BAZELISK to it" >&2
    exit 2
fi
if [ -z "${BAZELISK_HOME:-}" ]; then
    case "$(uname -s)" in
    Darwin) BAZELISK_HOME=$HOME/Library/Caches/bazelisk ;;
    *) BAZELISK_HOME=${XDG_CACHE_HOME:-$HOME/.cache}/bazelisk ;;
    esac
fi

# The environment Bazel runs in for Bazel version $1, as `env` arguments.
bazel_env() {
    BAZEL_ENV=(env -i "HOME=$FAKEHOME" "USER=$USER" "LOGNAME=$USER" "TERM=xterm-256color"
        "PATH=$(dirname "$BAZELISK"):/usr/bin:/bin:/usr/sbin:/sbin"
        "BAZELISK_HOME=$BAZELISK_HOME" "USE_BAZEL_VERSION=$1")
}

# Run "$@" on a pty 120 columns wide, what the pty got written to file $1
# (shifted off): script(1)'s util-linux and BSD forms alike.
on_pty() {
    local out=$1
    shift
    local cmd
    cmd="stty cols 120 rows 40 2>/dev/null; $(printf '%q ' "$@")"
    if script --version >/dev/null 2>&1; then
        script -qec "bash -c $(printf '%q' "$cmd")" "$out"
    else
        script -q "$out" bash -c "$cmd"
    fi
}
