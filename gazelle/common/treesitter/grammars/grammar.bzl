"""
Fetches and compiles tree-sitter grammars.
"""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

def fetch_grammars():
    http_archive(
        name = "tree-sitter-typescript",
        sha256 = "fb95a7a78268b3c0aeca86cc376681f1f4f9a1ae97b9bd8167c633bfd41398c6",
        strip_prefix = "tree-sitter-typescript-0.20.6",
        urls = ["https://github.com/tree-sitter/tree-sitter-typescript/archive/v0.20.6.tar.gz"],
        build_file_content = """
cc_library(
    name = "typescript",
    srcs = glob(["**/*.c", "**/*.h"]),
    includes = ["tsx/src", "typescript/src"],
    visibility = ["//visibility:public"],
)
""",
    )

    http_archive(
        name = "tree-sitter-kotlin",
        sha256 = "f8d6f766ff2da1bd411e6d55f4394abbeab808163d5ea6df9daa75ad48eb0834",
        strip_prefix = "tree-sitter-kotlin-0.3.5",
        urls = ["https://github.com/fwcd/tree-sitter-kotlin/archive/0.3.5.tar.gz"],
        patch_args = ["-p1"],
        patches = [
            # Patch to fix the following cc_library compile error:
            # external/tree-sitter-kotlin/src/scanner.c:2:10: error: 'tree_sitter/parser.h' file not found with <angled> include; use "quotes" instead
            "//gazelle/common/treesitter/grammars/kotlin:include_angles.patch",
        ],
        build_file_content = """
cc_library(
    name = "kotlin",
    srcs = glob(["src/**/*.c", "src/**/*.h"]),
    visibility = ["//visibility:public"],
)
""",
    )
