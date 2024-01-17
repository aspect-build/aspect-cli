load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
}

@test '--help flag should work' {
    run aspect --help
    assert_output --partial "Common Bazel Commands:" "Commands only in Aspect CLI:"
}

@test 'help command should work' {
    run aspect help build
    assert_output --partial "Performs a build on the specified targets, producing their default outputs." "Documentation: <https://bazel.build/run/build#bazel-build>"
    assert_output --partial "aspect build <target patterns> [flags]"
}

@test 'aspect with no args should display root help' {
    run aspect
    assert_output --partial "Aspect CLI is a better frontend for running bazel" "Common Bazel Commands" "Commands only in Aspect CLI" "Other Bazel Built-in Commands" "Additional Commands" "Additional help topics"
}

@test 'aspect help with no args should display root help' {
    run aspect help
    assert_output --partial "Aspect CLI is a better frontend for running bazel" "Common Bazel Commands" "Commands only in Aspect CLI" "Other Bazel Built-in Commands" "Additional Commands" "Additional help topics"
}

@test 'aspect help target-syntax' {
    run aspect help target-syntax
    assert_output --partial "Target pattern syntax"
}

@test 'aspect help info-keys' {
    run aspect help info-keys 
    assert_output --partial "bazel-bin"
}

@test 'aspect help tags' {
    run aspect help tags
    assert_output --partial "exclusive"
}

@test 'aspect help flags-as-proto' {
    run --separate-stderr aspect help flags-as-proto
    aspect_output=$output
    run --separate-stderr "$BAZEL_BINARY" --nosystem_rc --nohome_rc help flags-as-proto
    assert_equal "$output" "$aspect_output"
}