load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
}

teardown() {
    rm -rf foo
    rm -rf bar
    rm -f "$HOME/.aspect/cli/config.yaml"
}

@test 'preset query' {
    mkdir -p foo
    cat >foo/BUILD <<'EOF'
genrule(
    name = "foo",
    outs = ["foo.txt"],
    cmd = "touch $@",
)
EOF
    cat >"$HOME/.aspect/cli/config.yaml" <<'EOF'
query:
  presets:
    foo:
      description: "List deps"
      query: "deps(?target)"
      verb: "query"
EOF
    run aspect query foo //foo
    assert_output --partial "//foo:foo"

    # flags should work with preset queries
    run aspect query --output location foo //foo
    assert_output --partial "foo/BUILD:1:8: genrule rule //foo:foo"
    run aspect query --output=location foo //foo
    assert_output --partial "foo/BUILD:1:8: genrule rule //foo:foo"
    run aspect query foo //foo --output location
    assert_output --partial "foo/BUILD:1:8: genrule rule //foo:foo"
    run aspect query foo //foo --output=location
    assert_output --partial "foo/BUILD:1:8: genrule rule //foo:foo"
}

@test 'passthrough query' {
    mkdir -p bar
    cat >bar/BUILD <<'EOF'
genrule(
    name = "bar",
    outs = ["bar.txt"],
    cmd = "touch $@",
)
EOF
    run aspect query 'deps(//bar)'
    assert_output --partial "//bar:bar"

    # flags should work with passthrough queries
    run aspect query --output location 'deps(//bar)'
    assert_output --partial "bar/BUILD:1:8: genrule rule //bar:bar"
    run aspect query --output=location 'deps(//bar)'
    assert_output --partial "bar/BUILD:1:8: genrule rule //bar:bar"
    run aspect query 'deps(//bar)' --output location
    assert_output --partial "bar/BUILD:1:8: genrule rule //bar:bar"
    run aspect query 'deps(//bar)' --output=location
    assert_output --partial "bar/BUILD:1:8: genrule rule //bar:bar"
}

@test 'passthrough cquery' {
    mkdir -p bar
    cat >bar/BUILD <<'EOF'
genrule(
    name = "bar",
    outs = ["bar.txt"],
    cmd = "touch $@",
)
EOF
    run aspect cquery 'deps(//bar)'
    assert_output --partial "//bar:bar"

    # flags should work with passthrough queries
    run aspect cquery --output label_kind 'deps(//bar)'
    assert_output --partial "genrule rule //bar:bar"
    run aspect cquery --output=label_kind 'deps(//bar)'
    assert_output --partial "genrule rule //bar:bar"
    run aspect cquery 'deps(//bar)' --output label_kind
    assert_output --partial "genrule rule //bar:bar"
    run aspect cquery 'deps(//bar)' --output=label_kind
    assert_output --partial "genrule rule //bar:bar"
}

@test 'passthrough aquery' {
    mkdir -p bar
    cat >bar/BUILD <<'EOF'
genrule(
    name = "bar",
    outs = ["bar.txt"],
    cmd = "touch $@",
)
EOF
    run aspect aquery 'deps(//bar)'
    assert_output --partial "action 'Executing genrule //bar:bar'"

    # flags should work with passthrough queries
    run aspect aquery --output text 'deps(//bar)'
    assert_output --partial "action 'Executing genrule //bar:bar'"
    run aspect aquery --output=text 'deps(//bar)'
    assert_output --partial "action 'Executing genrule //bar:bar'"
    run aspect aquery 'deps(//bar)' --output text
    assert_output --partial "action 'Executing genrule //bar:bar'"
    run aspect aquery 'deps(//bar)' --output=text
    assert_output --partial "action 'Executing genrule //bar:bar'"
}
