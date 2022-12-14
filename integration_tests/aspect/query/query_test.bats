load "../common.bats"

setup() {
    touch WORKSPACE
}

teardown() {
    rm -rf foo
    rm -rf bar 
    rm -f "$HOME/.aspect/cli/config.yaml"
}

@test 'preset query' {
    mkdir -p foo
    cat > foo/BUILD << 'EOF'
genrule(
    name = "foo",
    outs = ["foo.txt"],
    cmd = "touch $@",
)
EOF
    cat > "$HOME/.aspect/cli/config.yaml" << 'EOF'
query:
  presets:
    foo:
      description: "List deps"
      query: "deps(?target)"
      verb: "query"
EOF
    run aspect query foo //foo
    assert_output --partial "//foo:foo"
}

@test 'passthrough query' {
    mkdir -p bar
    cat > bar/BUILD << 'EOF'
genrule(
    name = "bar",
    outs = ["bar.txt"],
    cmd = "touch $@",
)
EOF
    run aspect query 'deps(//bar)'
    assert_output --partial "//bar:bar"
}