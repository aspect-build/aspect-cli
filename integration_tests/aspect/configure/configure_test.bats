load "../common.bats"

setup() {
    touch BUILD.bazel WORKSPACE

    # js
    mkdir js
    touch js/BUILD.bazel js/a.ts

    # go
    mkdir go
    touch go/a.go

    # kotlin
    mkdir kotlin
    touch kotlin/a.kt

    # proto
    mkdir proto
    touch proto/a.proto
}

teardown() {
    rm -rf BUILD.bazel WORKSPACE js/ go/ kotlin/ proto/
    rm -f "$HOME/.aspect/cli/config.yaml"
}

@test 'aspect configure all disabled by default' {
    run aspect configure
    assert_output --partial "No languages enabled for BUILD"
}

@test 'aspect configure js' {
    cat > "$HOME/.aspect/cli/config.yaml" << 'EOF'
configure:
  languages:
    javascript: true
EOF
    run aspect configure
    assert_output --partial "1 BUILD file updated"
    run cat js/BUILD.bazel
    assert_output --partial "ts_project("
}

@test 'aspect configure enable go' {
    cat > "$HOME/.aspect/cli/config.yaml" << 'EOF'
configure:
  languages:
    go: true
EOF
    run aspect configure
    assert_output --partial "1 BUILD file updated"
    run cat go/BUILD.bazel
    assert_output --partial "go_library("
}

@test 'aspect configure enable kotlin' {
    cat > "$HOME/.aspect/cli/config.yaml" << 'EOF'
configure:
  languages:
    kotlin: true
EOF
    run aspect configure
    assert_output --partial "1 BUILD file updated"
    run cat kotlin/BUILD.bazel
    assert_output --partial "kt_jvm_library("
}

@test 'aspect configure enable protobuf' {
    cat > "$HOME/.aspect/cli/config.yaml" << 'EOF'
configure:
  languages:
    protobuf: true
EOF
    run aspect configure
    assert_output --partial "1 BUILD file updated"
    run cat proto/BUILD.bazel
    assert_output --partial "proto_library("
}