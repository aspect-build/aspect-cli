"macro for downloading bats dependencies"

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

CORE_VERSION = "1.8.2"
ASSERT_VERSION = "2.1.0"
SUPPORT_VERSION = "0.3.0"
DETIK_VERSION = "0d71702f9016a955fc8197d562bb1bb88ddf63a8"  # Aug 8, 2022

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

    http_archive(
        name = "bats_detik",
        urls = [
            "https://github.com/bats-core/bats-detik/archive/{}.tar.gz".format(DETIK_VERSION),
        ],
        sha256 = "1ebfbfc89e277c4455366efc37ee5e52036547be5ed640c44bd56499e6b0baf7",
        strip_prefix = "bats-detik-{}".format(DETIK_VERSION),
        add_prefix = "bats-detik",
        build_file_content = """
sh_library(
    name = "bats_detik",
    srcs = [
        "bats-detik/lib/detik.bash",
        "bats-detik/lib/linter.bash",
        "bats-detik/lib/utils.bash",
    ],
    visibility = ["//visibility:public"]
)
sh_library(
    name = "dir",
    srcs = ["bats-detik"],
    visibility = ["//visibility:public"]
)""",
    )
