"""Shell binary targets and their lint tests."""

load("@rules_shell//shell:defs.bzl", "sh_binary")
load("//tools/lint:linters.bzl", "shellcheck_test")

sh_binary(
    name = "hello",
    srcs = ["hello.sh"],
)

# Run with: bazel test //src:hello_shellcheck_test
shellcheck_test(
    name = "hello_shellcheck_test",
    size = "small",
    srcs = ["hello.sh"],
)
