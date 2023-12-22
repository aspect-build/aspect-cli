load "common.bats"

setup() {
    touch WORKSPACE
}

@test 'should download and reenter aspect cli version specified in bazeliskrc' {
    cat > .bazeliskrc << 'EOF'
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/5.8.19
EOF
    run aspect --version
    assert_success
    assert_output --partial "aspect 5.8.19"

    rm .bazeliskrc
}

@test 'exit code from reentrant aspect cli should be progated to parent' {
    cat > .bazeliskrc << 'EOF'
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/5.8.19
EOF
    run aspect --versio
    assert_failure
    assert_output --partial "unknown startup flag: --versio"

    rm .bazeliskrc
}

@test 'non-one exit code from reentrant aspect cli should be progated to parent' {
    cat > .bazeliskrc << 'EOF'
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/5.8.19
EOF
    run aspect configure --mode=diff
    assert_failure 112
    assert_output --partial "No languages enabled for BUILD file generation."

    rm .bazeliskrc
}
