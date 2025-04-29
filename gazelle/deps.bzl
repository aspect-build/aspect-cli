"""Utils for fetching aspect gazelle dependencies"""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")
load("//gazelle/common/treesitter/grammars:grammars.bzl", _fetch_grammars = "fetch_grammars")

def fetch_gazelle():
    http_archive(
        name = "bazel_gazelle",
        sha256 = "0adf07fcdc2d4d51a19a605df5c3fe89c4b0166f23f1a564f682a0ef2de1524b",
        strip_prefix = "bazel-gazelle-710f7cb6156a2c03f16307c607fdba7a7544cdf5",
        # Ensure this version always matches the go.mod version.
        #
        # :notice: Care should be taken when upgrading gazelle since we have vendored & modified parts of gazelle
        # in the CLI configure command (cli/core/pkg/aspect/configure).
        urls = ["https://github.com/bazel-contrib/bazel-gazelle/archive/710f7cb6156a2c03f16307c607fdba7a7544cdf5.tar.gz"],
        patch_args = ["-p1"],
        patches = [
            "//:patches/bazelbuild_bazel-gazelle_aspect-cli.patch",
            "//:patches/bazelbuild_bazel-gazelle_aspect-walk-subdir.patch",
            "//:patches/bazelbuild_bazel-gazelle_aspect-fs-direntry.patch",
            "//:patches/bazelbuild_bazel-gazelle_aspect-gitignore.patch",
        ],
    )
    # native.local_repository(
    #     name = "bazel_gazelle",
    #     path = "../bazel-gazelle",
    # )

fetch_grammars = _fetch_grammars

def fetch_deps():
    fetch_gazelle()
    _fetch_grammars()
