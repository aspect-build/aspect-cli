load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
}

@test 'aspect version' {
    run aspect version
    # Should print our own version
    assert_output --partial "Aspect CLI OSS version: unknown [not built with --stamp]"
    # Should also call through to `bazel version`
    assert_output --partial "Build label: 6.4.0"
}

@test '--version flag should work' {
    run aspect_vanilla --version
    assert_output --partial "aspect oss unknown [not built with --stamp]"
}

@test '-v flag should work' {
    run aspect_vanilla -v
    assert_output --partial "aspect oss unknown [not built with --stamp]"
}

@test '--bazel-version flag should work' {
    run aspect_vanilla --bazel-version
    assert_output --partial "bazel 6.4.0"
}
