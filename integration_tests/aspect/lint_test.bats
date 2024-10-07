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
    assert_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out"
    assert_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out.exit_code"
    assert_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.patch"

    run aspect lint //:all --@aspect_rules_lint//lint:fail_on_violation
    assert_failure
    assert_output --partial "SC2068"
    # if bazel exits non-zero then outputs are not reported
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out.exit_code"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.patch"
}

@test 'aspect lint should not output patch files if --nofixes flag is set' {
    run aspect lint //:all --nofixes
    assert_failure
    assert_output --partial "SC2068"
    assert_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out"
    assert_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out.exit_code"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.patch"

    run aspect lint //:all --nofixes --@aspect_rules_lint//lint:fail_on_violation
    assert_failure
    assert_output --partial "SC2068"
    # if bazel exits non-zero then outputs are not reported
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out.exit_code"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.patch"
}

@test 'aspect lint should not output report files if --noreport flag is set' {
    run aspect lint //:all --noreport
    assert_success
    refute_output --partial "SC2068"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out.exit_code"
    assert_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.patch"

    run aspect lint //:all --noreport --@aspect_rules_lint//lint:fail_on_violation
    assert_success
    refute_output --partial "SC2068"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out.exit_code"
    assert_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.patch"
}

@test 'aspect lint should always pass if --nofixes and --noreport flags are both set' {
    run aspect lint //:all --nofixes --noreport
    assert_success
    refute_output --partial "SC2068"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out.exit_code"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.patch"

    run aspect lint //:all --nofixes --noreport --@aspect_rules_lint//lint:fail_on_violation
    assert_success
    refute_output --partial "SC2068"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.out.exit_code"
    refute_output --partial "bazel-bin/shell.AspectRulesLintShellCheck.patch"
}
