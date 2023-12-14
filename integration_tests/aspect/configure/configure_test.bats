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
    [ "$status" -eq 112 ]
    assert_output --partial "No languages enabled for BUILD"
}

@test 'aspect configure bad cli params' {
    run aspect configure --baadd
    [ "$status" -eq 1 ]
    assert_output --partial "Error: unknown flag: --baadd"
}

@test 'aspect configure js' {
    cat > "$HOME/.aspect/cli/config.yaml" << 'EOF'
configure:
  languages:
    javascript: true
EOF
    run aspect configure
    [ "$status" -eq 110 ]
    assert_output --partial "1 BUILD file updated"
    run cat js/BUILD.bazel
    assert_output --partial "ts_project("

    # Nothing to update now
    run aspect configure
    [ "$status" -eq 0 ]
    assert_output --partial "0 BUILD files updated"
}

@test 'aspect configure js --mode=diff' {
    cat > "$HOME/.aspect/cli/config.yaml" << 'EOF'
configure:
  languages:
    javascript: true
EOF
    run aspect configure --mode=diff
    echo $status
    [ "$status" -eq 111 ]
    assert_output --partial "+ts_project("

    # Still has a diff
    run aspect configure --mode=diff
    [ "$status" -eq 111 ]
}

@test 'aspect configure enable go' {
    cat > "$HOME/.aspect/cli/config.yaml" << 'EOF'
configure:
  languages:
    go: true
EOF
    run aspect configure
    [ "$status" -eq 110 ]
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
    [ "$status" -eq 110 ]
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
    [ "$status" -eq 110 ]
    assert_output --partial "1 BUILD file updated"
    run cat proto/BUILD.bazel
    assert_output --partial "proto_library("
}