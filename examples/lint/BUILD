"""Targets in the repository root"""

load("@gazelle//:def.bzl", "gazelle")

exports_files(
    [
        ".shellcheckrc",
    ],
    visibility = ["//:__subpackages__"],
)

# We prefer BUILD instead of BUILD.bazel
# gazelle:build_file_name BUILD
# gazelle:exclude githooks/*

gazelle(
    name = "gazelle",
    env = {
        "ENABLE_LANGUAGES": ",".join([
            "starlark",
        ]),
    },
    gazelle = "@multitool//tools/gazelle",
)
