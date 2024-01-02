workspace(name = "build_aspect_cli")

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

http_archive(
    name = "aspect_bazel_lib",
    sha256 = "fc8bd670380eaba5314769abbe9fee21d641e3da06d9d26b8073a301f6d62332",
    strip_prefix = "bazel-lib-2.1.0",
    url = "https://github.com/aspect-build/bazel-lib/releases/download/v2.1.0/bazel-lib-v2.1.0.tar.gz",
)

http_archive(
    name = "aspect_rules_js",
    sha256 = "76a04ef2120ee00231d85d1ff012ede23963733339ad8db81f590791a031f643",
    strip_prefix = "rules_js-1.34.1",
    url = "https://github.com/aspect-build/rules_js/releases/download/v1.34.1/rules_js-v1.34.1.tar.gz",
)

http_archive(
    name = "aspect_rules_lint",
    sha256 = "6e32df708b40ea8c6d4482eeaf8fd6afaa8798d0028654ba25b667b5aee5707c",
    strip_prefix = "rules_lint-0.7.0",
    url = "https://github.com/aspect-build/rules_lint/releases/download/v0.7.0/rules_lint-v0.7.0.tar.gz",
)

http_archive(
    name = "bazel_skylib",
    sha256 = "cd55a062e763b9349921f0f5db8c3933288dc8ba4f76dd9416aac68acee3cb94",
    urls = ["https://github.com/bazelbuild/bazel-skylib/releases/download/1.5.0/bazel-skylib-1.5.0.tar.gz"],
)

http_archive(
    name = "com_grail_bazel_toolchain",
    patch_args = ["-p1"],
    patches = ["//patches:com_grail_bazel_toolchain.patch"],
    sha256 = "f6fd877e6f48092383e3dae80c28564450125e11fca31f35fba3ba599c9fd10d",
    strip_prefix = "bazel-toolchain-ceeedcc4464322e05fe5b8df3749cc02273ee083",
    urls = ["https://github.com/grailbio/bazel-toolchain/archive/ceeedcc4464322e05fe5b8df3749cc02273ee083.tar.gz"],
)

_SYSROOT_LINUX_BUILD_FILE = """
filegroup(
    name = "sysroot",
    srcs = glob(["*/**"]),
    visibility = ["//visibility:public"],
)
"""

_SYSROOT_DARWIN_BUILD_FILE = """
filegroup(
    name = "sysroot",
    srcs = glob(
        include = ["**"],
        exclude = ["**/*:*"],
    ),
    visibility = ["//visibility:public"],
)
"""

http_archive(
    name = "org_chromium_sysroot_linux_arm64",
    build_file_content = _SYSROOT_LINUX_BUILD_FILE,
    sha256 = "cf2fefded0449f06d3cf634bfa94ffed60dbe47f2a14d2900b00eb9bcfb104b8",
    urls = ["https://commondatastorage.googleapis.com/chrome-linux-sysroot/toolchain/80fc74e431f37f590d0c85f16a9d8709088929e8/debian_bullseye_arm64_sysroot.tar.xz"],
)

http_archive(
    name = "org_chromium_sysroot_linux_x86_64",
    build_file_content = _SYSROOT_LINUX_BUILD_FILE,
    sha256 = "04b94ba1098b71f8543cb0ba6c36a6ea2890d4d417b04a08b907d96b38a48574",
    urls = ["https://commondatastorage.googleapis.com/chrome-linux-sysroot/toolchain/f5f68713249b52b35db9e08f67184cac392369ab/debian_bullseye_amd64_sysroot.tar.xz"],
)

http_archive(
    name = "sysroot_darwin_universal",
    build_file_content = _SYSROOT_DARWIN_BUILD_FILE,
    # The ruby header has an infinite symlink that we need to remove.
    patch_cmds = ["rm System/Library/Frameworks/Ruby.framework/Versions/Current/Headers/ruby/ruby"],
    sha256 = "71ae00a90be7a8c382179014969cec30d50e6e627570af283fbe52132958daaf",
    strip_prefix = "MacOSX11.3.sdk",
    urls = ["https://s3.us-east-2.amazonaws.com/static.aspect.build/sysroots/MacOSX11.3.sdk.tar.xz"],
)

http_archive(
    name = "io_bazel_rules_go",
    patch_args = ["-p1"],
    patches = [
        "//patches:rules_go.patch",
        "//patches:rules_go.pr3617.patch",
    ],
    sha256 = "6dc2da7ab4cf5d7bfc7c949776b1b7c733f05e56edc4bcd9022bb249d2e2a996",
    urls = ["https://github.com/bazelbuild/rules_go/releases/download/v0.39.1/rules_go-v0.39.1.zip"],
)

http_archive(
    name = "rules_pkg",
    sha256 = "8f9ee2dc10c1ae514ee599a8b42ed99fa262b757058f65ad3c384289ff70c4b8",
    urls = ["https://github.com/bazelbuild/rules_pkg/releases/download/0.9.1/rules_pkg-0.9.1.tar.gz"],
)

http_archive(
    name = "buildifier_prebuilt",
    sha256 = "8ada9d88e51ebf5a1fdff37d75ed41d51f5e677cdbeafb0a22dda54747d6e07e",
    strip_prefix = "buildifier-prebuilt-6.4.0",
    urls = ["http://github.com/keith/buildifier-prebuilt/archive/6.4.0.tar.gz"],
)

load("@buildifier_prebuilt//:deps.bzl", "buildifier_prebuilt_deps")

buildifier_prebuilt_deps()

load("@aspect_bazel_lib//lib:repositories.bzl", "aspect_bazel_lib_dependencies", "register_coreutils_toolchains", "register_expand_template_toolchains", "register_jq_toolchains", "register_yq_toolchains")

aspect_bazel_lib_dependencies()

register_coreutils_toolchains()

register_expand_template_toolchains()

register_jq_toolchains()

register_yq_toolchains(version = "4.24.5")

load("@io_bazel_rules_go//extras:embed_data_deps.bzl", "go_embed_data_dependencies")
load("@io_bazel_rules_go//go:deps.bzl", "go_register_toolchains", "go_rules_dependencies")

go_rules_dependencies()

go_embed_data_dependencies()

go_register_toolchains(version = "1.20.4")

http_archive(
    name = "bazel_gazelle",
    sha256 = "b7387f72efb59f876e4daae42f1d3912d0d45563eac7cb23d1de0b094ab588cf",
    # Ensure this version always matches the version of @com_github_bazelbuild_bazel_gazelle set in deps.bzl.
    #
    # :notice: Care should be taken when upgrading gazelle since we have vendored & modified parts of gazelle
    # in the CLI configure command (cli/core/pkg/aspect/configure).
    urls = ["https://github.com/bazelbuild/bazel-gazelle/releases/download/v0.34.0/bazel-gazelle-v0.34.0.tar.gz"],
)

http_archive(
    name = "rules_proto",
    sha256 = "dc3fb206a2cb3441b485eb1e423165b231235a1ea9b031b4433cf7bc1fa460dd",
    strip_prefix = "rules_proto-5.3.0-21.7",
    urls = ["https://github.com/bazelbuild/rules_proto/archive/refs/tags/5.3.0-21.7.tar.gz"],
)

load("@rules_proto//proto:repositories.bzl", "rules_proto_dependencies", "rules_proto_toolchains")

rules_proto_dependencies()

rules_proto_toolchains()

load("@com_google_protobuf//:protobuf_deps.bzl", "protobuf_deps")

protobuf_deps()

load("@bazel_gazelle//:deps.bzl", "gazelle_dependencies")
load("//:go.bzl", _go_repositories = "deps")

# gazelle:repository_macro go.bzl%deps
_go_repositories()

gazelle_dependencies()

http_archive(
    name = "bazel_gomock",
    sha256 = "82a5fb946d2eb0fed80d3d70c2556784ec6cb5c35cd65a1b5e93e46f99681650",
    strip_prefix = "bazel_gomock-1.3",
    urls = [
        "https://github.com/jmhodges/bazel_gomock/archive/refs/tags/v1.3.tar.gz",
    ],
)

load("@aspect_rules_js//js:repositories.bzl", "rules_js_dependencies")

rules_js_dependencies()

load("@rules_nodejs//nodejs:repositories.bzl", "nodejs_register_toolchains")

nodejs_register_toolchains(
    name = "nodejs",
    node_version = "17.9.1",
)

load("@aspect_rules_js//npm:npm_import.bzl", "npm_translate_lock")

npm_translate_lock(
    name = "npm",
    pnpm_lock = "//:pnpm-lock.yaml",
    verify_node_modules_ignored = "//:.bazelignore",
)

load("@npm//:repositories.bzl", "npm_repositories")

npm_repositories()

load("//integration_tests:bats_deps.bzl", "bats_dependencies")

bats_dependencies()

load("//integration_tests:bazel_binary.bzl", "bazel_binaries")

bazel_binaries()

load("@bazel_skylib//:workspace.bzl", "bazel_skylib_workspace")

bazel_skylib_workspace()

load("@com_grail_bazel_toolchain//toolchain:rules.bzl", "llvm_toolchain")

llvm_toolchain(
    name = "llvm_toolchain",
    llvm_version = "14.0.0",
    sha256 = {
        "darwin-aarch64": "1b8975db6b638b308c1ee437291f44cf8f67a2fb926eb2e6464efd180e843368",
        "linux-x86_64": "564fcbd79c991e93fdf75f262fa7ac6553ec1dd04622f5d7db2a764c5dc7fac6",
    },
    strip_prefix = {
        "darwin-aarch64": "clang+llvm-14.0.0-arm64-apple-darwin",
        "linux-x86_64": "clang+llvm-14.0.0-x86_64-linux-gnu",
    },
    sysroot = {
        "darwin-aarch64": "@sysroot_darwin_universal//:sysroot",
        "darwin-x86_64": "@sysroot_darwin_universal//:sysroot",
        "linux-aarch64": "@org_chromium_sysroot_linux_arm64//:sysroot",
        "linux-x86_64": "@org_chromium_sysroot_linux_x86_64//:sysroot",
    },
    urls = {
        "darwin-aarch64": ["https://github.com/aspect-forks/llvm-project/releases/download/aspect-release-14.0.0/clang+llvm-14.0.0-arm64-apple-darwin.tar.xz"],
        "linux-x86_64": ["https://github.com/aspect-forks/llvm-project/releases/download/aspect-release-14.0.0/clang+llvm-14.0.0-x86_64-linux-gnu.tar.xz"],
    },
)

load("//platforms/toolchains:defs.bzl", "register_llvm_toolchains")

register_llvm_toolchains()

load("@buildifier_prebuilt//:defs.bzl", "buildifier_prebuilt_register_toolchains")

buildifier_prebuilt_register_toolchains()

# Use whichever formatter binaries you need:
load(
    "@aspect_rules_lint//format:repositories.bzl",
    "fetch_shfmt",
    "fetch_terraform",
)

fetch_shfmt()

fetch_terraform()

load("//.aspect/workflows:deps.bzl", "fetch_workflows_deps")

fetch_workflows_deps()
