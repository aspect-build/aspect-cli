load("@rules_shell//shell:sh_library.bzl", "sh_library")

config_setting(
    name = "macos_aarch64",
    constraint_values = [
        "@platforms//os:macos",
        "@platforms//cpu:aarch64",
    ],
)

alias(
    name = "shellcheck",
    actual = select({
        ":macos_aarch64": "@@+http_archive+shellcheck_darwin_aarch64//:shellcheck",
        "//conditions:default": "@@+http_archive+shellcheck_linux_x86_64//:shellcheck",
    }),
    visibility = ["//visibility:public"],
)

exports_files(
    [
        ".shellcheckrc",
    ],
    visibility = ["//:__subpackages__"],
)

sh_library(
    name = "hello",
    srcs = ["hello.sh"],
)
