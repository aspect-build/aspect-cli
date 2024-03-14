load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
}

teardown() {
    BAZELISK_BASE_URL=
    USE_BAZEL_VERSION=
    rm -f .bazeliskrc
}

@test 'should download and reenter aspect cli version specified in bazeliskrc' {
    cat >.bazeliskrc <<'EOF'
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/5.8.19
EOF

    run aspect version
    assert_success
    assert_output --partial "Aspect CLI version: 5.8.19"
    assert_output --partial "Build label: 6.4.0"
}

@test 'should download and reenter aspect cli version specified by USE_BAZEL_VERSION and BAZELISK_BASE_URL' {
    export BAZELISK_BASE_URL="https://github.com/aspect-build/aspect-cli/releases/download"
    export USE_BAZEL_VERSION="aspect/5.8.20"

    run aspect version
    assert_success
    assert_output --partial "Aspect CLI version: 5.8.20"
    assert_output --partial "Build label: 6.4.0"
}

@test 'exit code from reentrant aspect cli should be progated to parent' {
    cat >.bazeliskrc <<'EOF'
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/5.8.19
EOF

    run aspect --versio
    assert_failure
    assert_output --partial "unknown startup flag: --versio"
}

@test 'non-one exit code from reentrant aspect cli should be progated to parent' {
    cat >.bazeliskrc <<'EOF'
BAZELISK_BASE_URL=https://github.com/aspect-build/aspect-cli/releases/download
USE_BAZEL_VERSION=aspect/5.8.19
EOF

    run aspect configure --mode=diff
    assert_failure 112
    assert_output --partial "No languages enabled for BUILD file generation."
}
