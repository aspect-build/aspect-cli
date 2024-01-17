load "../common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
}

@test 'aspect info bazel-bin should work' {
    run aspect info bazel-bin
    assert_output --partial "bazel-out"
}