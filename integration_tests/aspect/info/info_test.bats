load "../common.bats"

setup() {
    touch WORKSPACE
}

@test 'aspect info bazel-bin should work' {
    run aspect info bazel-bin
    assert_output --partial "bazel-out"
}