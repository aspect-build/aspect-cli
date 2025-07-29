load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1

    mkdir test
}

teardown() {
    rm -rf test

    aspect clean
}

@test "should not swallow stdin in non --watch mode" {
    echo "cat" >test/stdin.sh
    chmod +x test/stdin.sh
    echo 'sh_binary(name = "stdin", srcs = ["stdin.sh"])' >test/BUILD.bazel

    run aspect run //test:stdin <<<"testing"
    assert_success 0
    assert_output -p "testing"
}

@test "should not swallow the succesful exit code" {
    echo "exit 0" >test/success.sh
    chmod +x test/success.sh
    echo 'sh_binary(name = "test", srcs = ["success.sh"])' >test/BUILD.bazel

    run aspect run //test
    assert_success 0
}

@test "should not swallow the exit code when runnable command is failing" {
    echo "exit 127" >test/fail.sh
    chmod +x test/fail.sh
    echo 'sh_binary(name = "test", srcs = ["fail.sh"])' >test/BUILD.bazel

    run aspect run //test
    assert_failure 127
}

@test "should not swallow the exit code when build failing" {
    echo "exit 0" >test/fail.sh
    chmod +x test/fail.sh
    echo 'sh_binary(name = "test, srcs = ["fail.sh"])' >test/BUILD.bazel

    run aspect run //test
    assert_failure 1
}

@test "should ignore --watch if it comes after -- in the command" {
    echo "env" >test/success.sh
    chmod +x test/success.sh
    echo 'sh_binary(name = "test", srcs = ["success.sh"])' >test/BUILD.bazel

    run aspect run //test -- --watch
    assert_success 0
    refute_output 'Watching feature is experimental and may have breaking changes in the future'
}

@test "should preserve the current working directory" {
    echo "echo 'workdir:'\$BUILD_WORKING_DIRECTORY" >test/success.sh
    echo "echo 'workspacedir:'\$BUILD_WORKSPACE_DIRECTORY" >>test/success.sh
    chmod +x test/success.sh
    echo 'sh_binary(name = "test", srcs = ["success.sh"])' >test/BUILD.bazel
    current_dir="$(pwd)"
    pushd test
    run aspect run :test
    popd
    assert_success 0
    assert_output --partial "workdir:$current_dir/test"
    assert_output --partial "workspacedir:$current_dir"
}

@test "should call the wrapper bazel if it is present" {
    echo "echo 'real binary'" >test/success.sh
    chmod +x test/success.sh
    echo 'sh_binary(name = "test", srcs = ["success.sh"])' >test/BUILD.bazel
    mkdir tools
    echo "#!/bin/bash" >tools/bazel
    echo "echo 'it called the wrapper'" >>tools/bazel
    echo "\$BAZEL_REAL \"\$@\"" >>tools/bazel
    chmod +x tools/bazel
    run aspect run //test

    assert_success 0
    assert_output --partial "it called the wrapper"
    assert_output --partial "real binary"
}

@test 'startup flags should work' {
    echo "echo 'real binary'" >test/success.sh
    chmod +x test/success.sh
    echo 'sh_binary(name = "test", srcs = ["success.sh"])' >test/BUILD.bazel
    run aspect --noidle_server_tasks run //test
    assert_success
    assert_output --partial "WARNING: Running Bazel server needs to be killed, because the startup options are different."
}
