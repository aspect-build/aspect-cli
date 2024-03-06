load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
}

teardown() {
    rm -f .bazelrc
    rm -f .bazeliskrc
    rm -f BUILD.bazel
    rm -f version-config.yaml
    rm -rf "$HOME/.aspect/cli/config.yaml"
}

@test 'flags should work' {
    cat > BUILD.bazel << 'EOF'
genrule(
    name = "foo",
    outs = ["foo.txt"],
    cmd = "touch $@",
)
EOF

    run aspect build :foo --announce_rc
    assert_success
    assert_output --partial "INFO: Build completed successfully"
}

@test 'mixing flags with --aspect: should work' {
    cat > BUILD.bazel << 'EOF'
genrule(
    name = "foo",
    outs = ["foo.txt"],
    cmd = "touch $@",
)
EOF

    run aspect build :foo --announce_rc --keep_going --aspect:interactive=false --nocheck_up_to_date --experimental_use_sandboxfs=false --keep_going
    assert_success
    assert_output --partial "INFO: Build completed successfully"
}

@test 'lock_version flag should prevent downloading and running bazeliskrc version' {
    cat > .bazeliskrc << 'EOF'
BAZELISK_BASE_URL=https://static.aspect.build/aspect
USE_BAZEL_VERSION=aspect/1.2.3
EOF

    run aspect
    assert_failure
    assert_output --partial "could not download Bazel"

    run aspect --aspect:lock_version
    assert_success
    assert_output --partial "Aspect CLI is a better frontend for running bazel"
}

@test 'lock_version flag should prevent downloading and running config version' {
    cat > version-config.yaml << 'EOF'
version: 1.2.3
EOF

    run aspect --aspect:config="version-config.yaml"
    assert_failure
    assert_output --partial "could not download Bazel"

    run aspect --aspect:config="version-config.yaml" --aspect:lock_version
    assert_success
    assert_output --partial "Aspect CLI is a better frontend for running bazel"
}

@test '--[no]able flags should work' {
    cat > BUILD.bazel << 'EOF'
genrule(
    name = "foo",
    outs = ["foo.txt"],
    cmd = "touch $@",
)
EOF

    run aspect build :foo --noannounce_rc
    assert_success
    assert_output --partial "INFO: Build completed successfully"
}

@test 'unknown flags should fail' {
    run aspect info :foo --noannounce_rcc
    assert_failure
    assert_output --partial "ERROR: --noannounce_rcc :: Unrecognized option: --noannounce_rcc"

    run aspect build :foo --noannounce_rcc
    assert_failure
    assert_output --partial "ERROR: --noannounce_rcc :: Unrecognized option: --noannounce_rcc"

    run aspect test :foo --noannounce_rcc
    assert_failure
    assert_output --partial "ERROR: --noannounce_rcc :: Unrecognized option: --noannounce_rcc"

    run aspect query :foo --noannounce_rcc
    assert_failure
    assert_output --partial "ERROR: --noannounce_rcc :: Unrecognized option: --noannounce_rcc"

    run aspect cquery :foo --noannounce_rcc
    assert_failure
    assert_output --partial "ERROR: --noannounce_rcc :: Unrecognized option: --noannounce_rcc"

    run aspect aquery :foo --noannounce_rcc
    assert_failure
    assert_output --partial "ERROR: --noannounce_rcc :: Unrecognized option: --noannounce_rcc"
}

@test '--[no]able flags should be hidden in help' {
    run aspect help build
    assert_success
    refute_output --partial "--nokeep_going"
}

@test '--[no]able startup flags should work' {
    run aspect info
    assert_success

    run aspect --noidle_server_tasks info
    assert_success
    assert_output --partial "WARNING: Running Bazel server needs to be killed, because the startup options are different."
}

@test 'startup flags with value should work' {
    run aspect info
    assert_success
    refute_output --partial "BINDIR:" "COMPILATION_MODE:" "GENDIR:"
    echo "info --show_make_env" > .bazelrc

    # --startup_flag=<value>
    run aspect --bazelrc=.bazelrc info
    assert_success
    assert_output --partial "BINDIR:" "COMPILATION_MODE:" "GENDIR:"

    # --startup_flag <value>
    run aspect --bazelrc .bazelrc info
    assert_success
    assert_output --partial "BINDIR:" "COMPILATION_MODE:" "GENDIR:"
}

@test 'run command should not process args after --' {
    touch "$HOME/.aspect/cli/config.yaml"
    cat > test.sh << 'EOF'
echo $@
EOF
    chmod +x test.sh
    cat > BUILD.bazel << 'EOF'
sh_binary(
    name = "bin",
    srcs = ["test.sh"],
    args = [
        "-p", "from_starlark"
    ]
)
EOF

    run aspect run :bin -- -p from_commandline
    assert_success
    assert_output --partial "-p from_starlark -p from_commandline"

    run aspect run :bin -- --aspect:config=/devil/config.yaml
    assert_success
    assert_output --partial "-p from_starlark --aspect:config=/devil/config.yaml"

    run aspect run :bin --aspect:config="$HOME/.aspect/cli/config.yaml" -- --aspect:config=/devil/config.yaml
    assert_success
    assert_output --partial "-p from_starlark --aspect:config=/devil/config.yaml"

    run aspect run :bin --aspect:config="/absent-config.yaml" -- --aspect:config=/devil/config.yaml
    assert_failure
    assert_output --partial "Error: failed to load --aspect:config file \"/absent-config.yaml\": open /absent-config.yaml: no such file or directory"
}

@test 'should warn about unknown flags that start with --aspect:' {
    touch BUILD.bazel

    run aspect query --aspect:nounknownflag=1 --aspect:interactive=false //...
    assert_failure
    assert_output --partial "ERROR: --aspect:nounknownflag=1 :: Unrecognized option: --aspect:nounknownflag=1"

    run aspect query --aspect:unknownflag=2 --aspect:interactive=false //...
    assert_failure
    assert_output --partial "ERROR: --aspect:unknownflag=2 :: Unrecognized option: --aspect:unknownflag=2"

    run aspect query --aspect:interactive=false //...
    assert_success
}

@test 'startup flags in .bazelrc should not permanently kill bazel server' {
    run aspect info
    assert_success
    refute_output --partial "WARNING: Running Bazel server needs to be killed, because the startup options are different."

    echo "startup --noidle_server_tasks" > .bazelrc

    run aspect info
    assert_success
    assert_output --partial "WARNING: Running Bazel server needs to be killed, because the startup options are different."

    run aspect info
    assert_success
    refute_output --partial "WARNING: Running Bazel server needs to be killed, because the startup options are different."
}
