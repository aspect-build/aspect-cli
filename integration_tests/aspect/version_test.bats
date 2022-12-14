load "common.bats"

setup() {
    touch WORKSPACE
}

@test 'aspect version' {
    run aspect version
    # Should print our own version
    assert_output --partial "Aspect CLI version:"
    # Should also call through to `bazel version`
    assert_output --partial "Build label:"
    assert_output --partial "Aspect CLI version: unknown [not built with --stamp]"
}

@test '--version flag should work' {
    run aspect --version
    assert_output --partial "aspect unknown [not built with --stamp]"
}
