"""
Fetches and compiles tree-sitter grammars.
"""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

# Most tree-sitter languages have a simple source structure like this.
BUILD = """\
filegroup(
    name = "srcs",
    srcs = glob(["src/**/*.c", "src/**/*.h"]),
    visibility = ["//visibility:public"],
)
"""

# buildifier: disable=function-docstring
def fetch_grammars():
    http_archive(
        name = "tree-sitter-go",
        integrity = "sha256-M7w7RN4de4FfUv6fQuSXhsWDPZGB/Vn9aRDBnIJVkik=",
        urls = ["https://github.com/tree-sitter/tree-sitter-go/releases/download/v0.23.4/tree-sitter-go.tar.xz"],
        build_file_content = BUILD,
    )

    http_archive(
        name = "tree-sitter-java",
        sha256 = "ed766e1045be236e50a7f99295996f6705d7506628b79af80d1fd5efb63c86a7",
        urls = ["https://github.com/tree-sitter/tree-sitter-java/releases/download/v0.23.5/tree-sitter-java.tar.xz"],
        build_file_content = BUILD,
    )

    http_archive(
        name = "tree-sitter-json",
        sha256 = "acf6e8362457e819ed8b613f2ad9a0e1b621a77556c296f3abea58f7880a9213",
        strip_prefix = "tree-sitter-json-0.24.8",
        urls = ["https://github.com/tree-sitter/tree-sitter-json/archive/v0.24.8.tar.gz"],
        build_file_content = BUILD,
    )

    http_archive(
        name = "tree-sitter-kotlin",
        sha256 = "7dd60975786bf9cb4be6a5176f5ccb5fed505f9929a012da30762505b1015669",
        strip_prefix = "tree-sitter-kotlin-0.3.8",
        urls = ["https://github.com/fwcd/tree-sitter-kotlin/archive/0.3.8.tar.gz"],
        build_file_content = BUILD,
    )

    http_archive(
        name = "tree-sitter-starlark",
        sha256 = "31c58a540d738a17b366f2046da298b66dfa0695bcbfa207f61fa63cfe5c03ed",
        strip_prefix = "tree-sitter-starlark-1.3.0",
        urls = ["https://github.com/tree-sitter-grammars/tree-sitter-starlark/archive/v1.3.0.tar.gz"],
        build_file_content = BUILD,
    )

    http_archive(
        name = "tree-sitter-typescript",
        sha256 = "2c4ce711ae8d1218a3b2f899189298159d672870b5b34dff5d937bed2f3e8983",
        strip_prefix = "tree-sitter-typescript-0.23.2",
        urls = ["https://github.com/tree-sitter/tree-sitter-typescript/archive/v0.23.2.tar.gz"],
        build_file_content = """
filegroup(
    name = "typescript-srcs",
    srcs = glob(["common/**/*.h", "typescript/src/**/*.h", "typescript/src/**/*.c"]),
    visibility = ["//visibility:public"],
)
filegroup(
    name = "tsx-srcs",
    srcs = glob(["common/**/*.h", "tsx/src/**/*.h", "tsx/src/**/*.c"]),
    visibility = ["//visibility:public"],
)
""",
    )
