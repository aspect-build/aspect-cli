load "common.bats"

setup() {
    touch WORKSPACE
}

teardown() {
    rm -f .bazelrc
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
    assert_output --partial "INFO: Build completed successfully"
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
    assert_output --partial "INFO: Build completed successfully"
}


@test 'uknown flags should fail' {
    run aspect build :foo --noannounce_rcc
    assert_output --partial "ERROR: --noannounce_rcc :: Unrecognized option: --noannounce_rcc"
}

@test '--[no]able flags should be hidden in help' {
    run aspect help build
    refute_output --partial "--nokeep_going"
}

@test '--[no]able startup flags should work' {
    run aspect info
    run aspect --noidle_server_tasks info
    assert_output --partial "WARNING: Running Bazel server needs to be killed, because the startup options are different."
}

@test 'startup flags with value should work' {
    run aspect info
    refute_output --partial "BINDIR:" "COMPILATION_MODE:" "GENDIR:"
    echo "info --show_make_env" > .bazelrc

    # --startup_flag=<value>
    run aspect --bazelrc=.bazelrc info
    assert_output --partial "BINDIR:" "COMPILATION_MODE:" "GENDIR:"

    # --startup_flag <value>
    run aspect --bazelrc .bazelrc info
    assert_output --partial "BINDIR:" "COMPILATION_MODE:" "GENDIR:"
}

@test 'run command should not process args after --' {
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
    assert_output --partial "-p from_starlark -p from_commandline"

    run aspect run :bin -- --aspect:config=/devil/config.yaml
    assert_output --partial "-p from_starlark --aspect:config=/devil/config.yaml"

    run aspect run :bin --aspect:config="$HOME/.aspect/cli/config.yaml" -- --aspect:config=/devil/config.yaml
    assert_output --partial "-p from_starlark --aspect:config=/devil/config.yaml"

    run aspect run :bin --aspect:config="/absent-config.yaml" -- --aspect:config=/devil/config.yaml
    assert_output --partial "Error: Failed to load Aspect CLI config file '/absent-config.yaml' specified with --aspect:config flag: open /absent-config.yaml: no such file or directory"
}

@test 'should warn about unknown flags that start with --aspect:' {
    run aspect query --aspect:nounknownflag=1 --aspect:interactive=false
    assert_output --partial "Error: unknown flag: --aspect:nounknownflag"

    run aspect query --aspect:unknownflag=2 --aspect:interactive=false
    assert_output --partial "Error: unknown flag: --aspect:unknownflag"
}