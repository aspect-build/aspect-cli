"Define linter aspects"

load("@aspect_rules_lint//lint:shellcheck.bzl", "lint_shellcheck_aspect")

shellcheck = lint_shellcheck_aspect(
    binary = Label("//tools/lint:shellcheck"),
    config = Label("//:.shellcheckrc"),
)
