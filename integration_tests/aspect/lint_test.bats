load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
    mkdir -p ".aspect/cli"

    cat >hello.sh <<'EOF'
#!/bin/sh
exec ./some-program $@
EOF

    cat >BUILD.bazel <<'EOF'
exports_files([".shellcheckrc"])
sh_library(name = "shell", srcs = ["hello.sh"])
EOF

    cat >MODULE.bazel <<'EOF'
bazel_dep(name = "aspect_rules_lint", version = "1.0.0-rc10")
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

@test 'aspect lint should report lint violations and exit non-zero' {
    run aspect lint //:all
    assert_failure
    assert_output --partial "SC2068"

    run aspect lint //:all --@aspect_rules_lint//lint:fail_on_violation
    assert_failure
    assert_output --partial "SC2068"
}
