"""Utils for fetching aspect gazelle dependencies"""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")
load("//gazelle/common/treesitter/grammars:grammars.bzl", _fetch_grammars = "fetch_grammars")

def fetch_gazelle():
    http_archive(
        name = "bazel_gazelle",
        sha256 = "fa1a981ae546684dbb7e7f428bafe0180530af09eace265094e03f4383fc0de4",
        strip_prefix = "bazel-gazelle-186298911d38850b47b198e8d933a93125ce7043",        # Ensure this version always matches the go.mod version.
        #
        # :notice: Care should be taken when upgrading gazelle since we have vendored & modified parts of gazelle
        # in the CLI configure command (/pkg/aspect/configure).
        urls = ["https://github.com/bazel-contrib/bazel-gazelle/archive/186298911d38850b47b198e8d933a93125ce7043.tar.gz"],
        patch_args = ["-p1"],
        patches = [
            "//:patches/bazelbuild_bazel-gazelle_aspect-cli.patch",
            "//:patches/bazelbuild_bazel-gazelle_aspect-walk-subdir.patch",
            "//:patches/bazelbuild_bazel-gazelle_aspect-gitignore.patch",
            "//:patches/bazelbuild_bazel-gazelle_aspect-fs-direntry.patch",
        ],
    )

fetch_grammars = _fetch_grammars

def fetch_deps():
    fetch_gazelle()
    _fetch_grammars()
