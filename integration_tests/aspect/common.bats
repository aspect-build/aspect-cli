bats_load_library 'bats-support'
bats_load_library 'bats-assert'

aspect() {
    # workflows creates system wide config at /etc/.bazelrc which contains configuration bits like --output_base.
    # but the configured path is only writable by specific users which we can't run as within the sandbox.
    # these system wide flag affect the output path of the bazel-in-bazel that is run inside the tests leading to errors.
    # NOTE: the output files left by bazel-in-bazel should be discarded as they are not part of the action.
    "$TEST_SRCDIR/$TEST_WORKSPACE/cmd/aspect/aspect_/aspect" --aspect:nosystem_config --nosystem_rc --nohome_rc "$@"
}

aspect_vanilla() {
    # for use with --version, -v and --bazel-version
    "$TEST_SRCDIR/$TEST_WORKSPACE/cmd/aspect/aspect_/aspect" "$@"
}

setup_file() {
    BAZEL_BINARY=$(realpath "$BAZEL_BINARY")

    export TEST_REPO="${TEST_TMPDIR}/mock-repo"
    mkdir "$TEST_REPO"
    echo "$BAZEL_BINARY" >"$TEST_REPO/.bazelversion"
    touch "$TEST_REPO/WORKSPACE"
    touch "$TEST_REPO/MODULE.bazel"
    echo "common --enable_bzlmod" >>"$TEST_REPO/.bazelrc"

    export HOME="${TEST_TMPDIR}/mock-home"
    mkdir "$HOME"
    mkdir -p "$HOME/.aspect/cli"
}

teardown_file() {
    aspect shutdown

    rm -rf "${TEST_TMPDIR}/mock-repo"
    rm -rf "${TEST_TMPDIR}/mock-home"
}
