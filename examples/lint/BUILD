load("@rules_shell//shell:sh_library.bzl", "sh_library")

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
