"macro for downloading bats dependencies"

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

CORE_VERSION = "1.8.2"
ASSERT_VERSION = "2.1.0"
SUPPORT_VERSION = "0.3.0"

def bats_dependencies():
    http_archive(
        name = "bats_core",
        sha256 = "0f2df311a536e625a72bff64c838e67c7b5032e6ea9edcdf32758303062b2f3b",
        urls = [
            "https://github.com/bats-core/bats-core/archive/v{}.tar.gz".format(CORE_VERSION),
        ],
        strip_prefix = "bats-core-{}".format(CORE_VERSION),
        build_file_content = """
sh_library(
    name = "bats_core",
    srcs = glob([
        "lib/**",
        "libexec/**"
    ]),
    visibility = ["//visibility:public"]
)
sh_library(
    name = "bin",
    srcs = ["bin/bats"],
    visibility = ["//visibility:public"]
)
        """,
    )

    http_archive(
        name = "bats_assert",
        urls = [
            "https://github.com/bats-core/bats-assert/archive/v{}.tar.gz".format(ASSERT_VERSION),
        ],
        sha256 = "98ca3b685f8b8993e48ec057565e6e2abcc541034ed5b0e81f191505682037fd",
        strip_prefix = "bats-assert-{}".format(ASSERT_VERSION),
        add_prefix = "bats-assert",
        build_file_content = """
sh_library(
    name = "bats_assert",
    srcs = glob([
        "bats-assert/src/**",
        "bats-assert/load.bash",
    ]),
    visibility = ["//visibility:public"]
)
sh_library(
    name = "dir",
    srcs = [
        "bats-assert",
    ],
    visibility = ["//visibility:public"]
)
        """,
    )

    http_archive(
        name = "bats_support",
        urls = [
            "https://github.com/bats-core/bats-support/archive/v{}.tar.gz".format(SUPPORT_VERSION),
        ],
        sha256 = "7815237aafeb42ddcc1b8c698fc5808026d33317d8701d5ec2396e9634e2918f",
        strip_prefix = "bats-support-{}".format(SUPPORT_VERSION),
        add_prefix = "bats-support",
        build_file_content = """
sh_library(
    name = "bats_support",
    srcs = glob([
        "bats-support/src/**",
        "bats-support/load.bash",
    ]),
    visibility = ["//visibility:public"]
)
sh_library(
    name = "dir",
    srcs = [
        "bats-support",
    ],
    visibility = ["//visibility:public"]
)""",
    )
