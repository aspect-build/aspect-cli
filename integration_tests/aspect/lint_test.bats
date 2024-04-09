load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
    mkdir -p ".aspect/cli"

    cat >hello.sh <<'EOF'
#!/bin/sh
exec ./some-program $@
EOF

    cat >>BUILD.bazel <<'EOF'
exports_files([".shellcheckrc"])
sh_library(name = "shell", srcs = ["hello.sh"])
EOF

    cat >>MODULE.bazel <<'EOF'
bazel_dep(name = "aspect_rules_lint", version = "0.18.0")
EOF

    cat >lint.bzl <<'EOF'
load("@aspect_rules_lint//lint:shellcheck.bzl", "lint_shellcheck_aspect")
shellcheck = lint_shellcheck_aspect(
    binary = "@multitool//tools/shellcheck",
    config = "@@//:.shellcheckrc",
)
EOF

    touch .shellcheckrc

    cat >".aspect/cli/config.yaml" <<'EOF'
lint:
    aspects:
      - //:lint.bzl%shellcheck
EOF

}

@test 'aspect lint should work' {
    run aspect lint //:all
    # Should report a lint violation
    assert_output --partial "SC2068"
}
