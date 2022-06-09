#!/usr/bin/env bash

set -o pipefail -o errexit -o nounset
HOME="$TEST_TMPDIR"
ASPECT="$TEST_SRCDIR/build_aspect_cli/cmd/aspect/aspect_/aspect"
export HOME
touch WORKSPACE

mkdir foo
cat > foo/BUILD <<'EOF'
genrule(
    name = "foo",
    outs = ["foo.txt"],
    cmd = "touch $@",
)
EOF

cat > .aspect.yaml <<'EOF'
query:
  presets:
    foo:
      description: "List deps"
      query: "deps(?target)"
      verb: "query"
EOF

# Only capture stdout, just like `bazel version` prints to stdout
query=$($ASPECT query foo //foo 2>/dev/null) || "$ASPECT" query foo //foo

# Should list the //foo:foo genrule that we have created
[[ "$query" =~ "//foo:foo" ]] || {
    echo >&2 "Expected 'aspect query foo //foo' stdout to contain '//foo:foo', but was"
    echo "$query"
    exit 1
}
