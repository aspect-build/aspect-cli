"""Utils for fetching aspect gazelle dependencies"""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")
load("//gazelle/common/treesitter/grammars:grammars.bzl", _fetch_grammars = "fetch_grammars")

def fetch_gazelle():
    http_archive(
        name = "bazel_gazelle",
        sha256 = "a80893292ae1d78eaeedd50d1cab98f242a17e3d5741b1b9fb58b5fd9d2d57bc",
        # Ensure this version always matches the go.mod version.
        #
        # :notice: Care should be taken when upgrading gazelle since we have vendored & modified parts of gazelle
        # in the CLI configure command (cli/core/pkg/aspect/configure).
        urls = ["https://github.com/bazel-contrib/bazel-gazelle/releases/download/v0.40.0/bazel-gazelle-v0.40.0.tar.gz"],
        patch_args = ["-p1"],
        patches = [
            "//cli/core:patches/bazelbuild_bazel-gazelle_aspect-cli.patch",
            "//cli/core:patches/bazelbuild_bazel-gazelle_aspect-walk-subdir.patch",
            "//cli/core:patches/bazelbuild_bazel-gazelle_aspect-gitignore.patch",
            "//cli/core:patches/bazelbuild_bazel-gazelle_aspect-fs-direntry.patch",
        ],
    )

fetch_grammars = _fetch_grammars

def fetch_deps():
    fetch_gazelle()
    _fetch_grammars()
