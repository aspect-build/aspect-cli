"""
Fetches and compiles tree-sitter grammars.
"""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

def fetch_grammars():
    http_archive(
        name = "tree-sitter-json",
        sha256 = "ecde752640fb6eedd25b63647f016f92b3b63096d08f60742cbf19395c5c6036",
        strip_prefix = "tree-sitter-json-0.23.0",
        urls = ["https://github.com/tree-sitter/tree-sitter-json/archive/v0.23.0.tar.gz"],
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
        sha256 = "7dd60975786bf9cb4be6a5176f5ccb5fed505f9929a012da30762505b1015669",
        strip_prefix = "tree-sitter-kotlin-0.3.8",
        urls = ["https://github.com/fwcd/tree-sitter-kotlin/archive/0.3.8.tar.gz"],
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
        integrity = "sha256-STb+4buXAstpVLGTDqwTPCzxzEDz3n1EpqPXdtI7IWw=",
        strip_prefix = "tree-sitter-starlark-1.2.0",
        urls = ["https://github.com/tree-sitter-grammars/tree-sitter-starlark/archive/v1.2.0.tar.gz"],
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
        sha256 = "af500e16060b0221db8fb0743a37ca677340f8024127b54f6b6fc1ebfde496f4",
        strip_prefix = "tree-sitter-typescript-0.23.0",
        urls = ["https://github.com/tree-sitter/tree-sitter-typescript/archive/v0.23.0.tar.gz"],
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
