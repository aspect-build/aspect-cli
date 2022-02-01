#!/usr/bin/env bash

set -o pipefail -o errexit -o nounset
HOME="$TEST_TMPDIR"
touch "$HOME"/.aspect.yaml
ASPECT="$TEST_SRCDIR/build_aspect_cli/cmd/aspect/aspect_/aspect"
export HOME
touch WORKSPACE

mkdir foo
echo "genrule(" > foo/BUILD
echo "    name = \"foo\"," >> foo/BUILD
echo "    outs = [\"foo.txt\"]," >> foo/BUILD
echo "    cmd = \"touch \$@\"," >> foo/BUILD
echo ")" >> foo/BUILD

echo "query:" > .aspectconfig.yaml
echo "  presets:" >> .aspectconfig.yaml
echo "    foo:" >> .aspectconfig.yaml
echo "      description: \"List deps\"" >> .aspectconfig.yaml
echo "      query: \"deps(?target)\"" >> .aspectconfig.yaml
echo "      verb: \"query\"" >> .aspectconfig.yaml

# Only capture stdout, just like `bazel version` prints to stdout
query=$($ASPECT query foo //foo 2>/dev/null) || "$ASPECT" query foo //foo

# Should list the //foo:foo genrule that we have created
[[ "$query" =~ "//foo:foo" ]] || {
    echo >&2 "Expected 'aspect query foo //foo' stdout to contain '//foo:foo', but was"
    echo "$query"
    exit 1
}
