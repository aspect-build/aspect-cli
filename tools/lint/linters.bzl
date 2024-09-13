"Create linter aspects, see https://github.com/aspect-build/rules_lint/blob/main/docs/linting.md#installation"

load("@aspect_rules_lint//lint:shellcheck.bzl", "lint_shellcheck_aspect")

shellcheck = lint_shellcheck_aspect(
    binary = "@multitool//tools/shellcheck",
    config = "@@//:.shellcheckrc",
)
