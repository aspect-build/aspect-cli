"Define linter aspects"

load("@aspect_rules_lint//lint:shellcheck.bzl", "lint_shellcheck_aspect")

shellcheck = lint_shellcheck_aspect(
    binary = Label("//:shellcheck"),
    config = Label("//:.shellcheckrc"),
)
