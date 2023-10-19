"macro for downloading bats dependencies"

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

CORE_VERSION = "1.8.2"
ASSERT_VERSION = "2.1.0"
SUPPORT_VERSION = "0.3.0"
FILES_VERSION = "c0f822aceac6a70614c5a7c92fd9c5ddd97c7f83"  # May 25, 2023
MOCK_VERSION = "48fce74482a4d2bb879b904ccab31b6bc98e3224"  # May 3, 2021
DETIK_VERSION = "0d71702f9016a955fc8197d562bb1bb88ddf63a8"  # Aug 8, 2022

def bats_dependencies():
    """Fetches the required dependencies to run bats_test"""

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
        name = "bats_file",
        urls = [
            "https://github.com/bats-core/bats-file/archive/{}.tar.gz".format(FILES_VERSION),
        ],
        sha256 = "76a19ead26c7cf666b9fbe659874b947392443de638f76802f057739f04a8d33",
        strip_prefix = "bats-file-{}".format(FILES_VERSION),
        add_prefix = "bats-file",
        build_file_content = """
sh_library(
    name = "bats_file",
    srcs = glob([
        "bats-file/src/**",
        "bats-file/load.bash",
    ]),
    visibility = ["//visibility:public"]
)
sh_library(
    name = "dir",
    srcs = [
        "bats-file",
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
        name = "bats_mock",
        urls = [
            "https://github.com/grayhemp/bats-mock/archive/{}.tar.gz".format(MOCK_VERSION),
        ],
        sha256 = "1559fd3e6b63818f6ddf8652250dbbc9de47230f0c46b91dc84a73b5704cd3cd",
        strip_prefix = "bats-mock-{}".format(MOCK_VERSION),
        add_prefix = "bats-mock",
        build_file_content = """
sh_library(
    name = "bats_mock",
    srcs = [
        "bats-mock/load.bash",
    ],
    visibility = ["//visibility:public"]
)
sh_library(
    name = "dir",
    srcs = ["bats-mock"],
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
