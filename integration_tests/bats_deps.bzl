"macro for downloading bats dependencies"

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

CORE_VERSION = "1.11.0"  # March 26th 2024
ASSERT_VERSION = "2.1.0"  # October 22 2022
SUPPORT_VERSION = "9bf10e876dd6b624fe44423f0b35e064225f7556"  # August 26, 2023
FILES_VERSION = "048aa4c595d4a103d6ec3518ead9e071efc019e2"  # August 25, 2023
MOCK_VERSION = "48fce74482a4d2bb879b904ccab31b6bc98e3224"  # May 3, 2021
DETIK_VERSION = "85ce4ba67d2ccec6e248202f3e994a86abc6e0a4"  # February 26, 2023

def bats_dependencies():
    """Fetches the required dependencies to run bats_test"""

    http_archive(
        name = "bats_core",
        integrity = "sha256-rv8J/ciwyIswh8md4Az1STVtei9qaeP87F4Ohh0vkGM=",
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
        integrity = "sha256-mMo7aF+LiZPkjsBXVl5uKrzFQQNO1bDoHxkVBWggN/0=",
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
        integrity = "sha256-hd3iOyI0EllkQnVrH/gupo4Z1Hy3YoV/y5g4dblD5aU=",
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
            "https://github.com/bats-core/bats-support/archive/{}.tar.gz".format(SUPPORT_VERSION),
        ],
        integrity = "sha256-tXdsryy8AHowm1Ooi7isP6oAkQguQUr0PY7XPOXSaLI=",
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
        integrity = "sha256-FVn9PmtjgY9t34ZSJQ27yd5HIw8MRrkdyEpztXBM080=",
        strip_prefix = "bats-mock-{}".format(MOCK_VERSION),
        add_prefix = "bats-mock",
        build_file_content = """
sh_library(
    name = "bats_mock",
    srcs = glob([
        "bats-support/src/**",
        "bats-support/load.bash",
    ], allow_empty = True),
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
        integrity = "sha256-16bor7Kmxj5xHgS0hkEHy3vpJo2X/hDTVRzmUvLR+Q8=",
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
