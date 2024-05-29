"""
Fetches and compiles tree-sitter grammars.
"""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

def fetch_grammars():
    http_archive(
        name = "tree-sitter-json",
        sha256 = "83d89d297c475dceb1865a9eacd8da1008e1aecffb6137161361107665cbdf79",
        strip_prefix = "tree-sitter-json-0.21.0",
        urls = ["https://github.com/tree-sitter/tree-sitter-json/archive/v0.21.0.tar.gz"],
        build_file_content = """
filegroup(
    name = "srcs",
    srcs = glob(["src/**/*.c", "src/**/*.h"]),
    visibility = ["//visibility:public"],
)
""",
    )

    http_archive(
        name = "tree-sitter-kotlin",
        sha256 = "f8d6f766ff2da1bd411e6d55f4394abbeab808163d5ea6df9daa75ad48eb0834",
        strip_prefix = "tree-sitter-kotlin-0.3.5",
        urls = ["https://github.com/fwcd/tree-sitter-kotlin/archive/0.3.5.tar.gz"],
        build_file_content = """
filegroup(
    name = "srcs",
    srcs = glob(["src/**/*.c", "src/**/*.h"]),
    visibility = ["//visibility:public"],
)
""",
    )

    http_archive(
        name = "tree-sitter-starlark",
        integrity = "sha256-obJ54UOlqTfd1EInZNkV8ju0CFNh3gmULTvVdJ7uPKU=",
        strip_prefix = "tree-sitter-starlark-1.0.0",
        urls = ["https://github.com/tree-sitter-grammars/tree-sitter-starlark/archive/v1.0.0.tar.gz"],
        build_file_content = """
filegroup(
    name = "srcs",
    srcs = glob(["src/**/*.c", "src/**/*.h"]),
    visibility = ["//visibility:public"],
)
""",
    )

    http_archive(
        name = "tree-sitter-typescript",
        sha256 = "fb95a7a78268b3c0aeca86cc376681f1f4f9a1ae97b9bd8167c633bfd41398c6",
        strip_prefix = "tree-sitter-typescript-0.20.6",
        urls = ["https://github.com/tree-sitter/tree-sitter-typescript/archive/v0.20.6.tar.gz"],
        build_file_content = """
filegroup(
    name = "typescript-srcs",
    srcs = glob(["**/*.c", "**/*.h"], exclude=["tsx/**"]),
    visibility = ["//visibility:public"],
)
filegroup(
    name = "tsx-srcs",
    srcs = glob(["**/*.c", "**/*.h"], exclude=["typescript/**"]),
    visibility = ["//visibility:public"],
)
""",
    )
