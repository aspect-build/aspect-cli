bats_load_library 'bats-support'
bats_load_library 'bats-assert'

aspect() {
    # workflows creates system wide config at /etc/.bazelrc which contains configuration bits like --output_base. 
    # but the configured path is only writable by specific users which we can't run as within the sandbox. 
    # these system wide flag affect the output path of the bazel-in-bazel that is run inside the tests leading to errors.
    # NOTE: the output files left by bazel-in-bazel should be discarded as they are not part of the action. 
    "$TEST_SRCDIR/build_aspect_cli/cmd/aspect/aspect_/aspect" --nosystem_rc --nohome_rc $@
}

setup_file() {
    export USE_BAZEL_VERSION=$(realpath $BAZEL_BINARY)
    export HOME="$TEST_TMPDIR"
    mkdir -p "$HOME/.aspect/cli"
    touch "$HOME/.aspect/cli/config.yaml"
}

teardown_file() {
    aspect shutdown
    rm -f "$HOME/.aspect/cli/config.yaml"
}