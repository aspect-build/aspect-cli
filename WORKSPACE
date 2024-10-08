workspace(name = "build_aspect_cli")

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

HERMETIC_CC_TOOLCHAIN_VERSION = "v3.1.1"

http_archive(
    name = "hermetic_cc_toolchain",
    sha256 = "907745bf91555f77e8234c0b953371e6cac5ba715d1cf12ff641496dd1bce9d1",
    urls = [
        "https://mirror.bazel.build/github.com/uber/hermetic_cc_toolchain/releases/download/{0}/hermetic_cc_toolchain-{0}.tar.gz".format(HERMETIC_CC_TOOLCHAIN_VERSION),
        "https://github.com/uber/hermetic_cc_toolchain/releases/download/{0}/hermetic_cc_toolchain-{0}.tar.gz".format(HERMETIC_CC_TOOLCHAIN_VERSION),
    ],
)

load("@hermetic_cc_toolchain//toolchain:defs.bzl", zig_toolchains = "toolchains")

zig_toolchains()

register_toolchains(
    "@zig_sdk//toolchain:windows_amd64",
    "@zig_sdk//toolchain:windows_arm64",
)

http_archive(
    name = "bazel_features",
    sha256 = "2cd9e57d4c38675d321731d65c15258f3a66438ad531ae09cb8bb14217dc8572",
    strip_prefix = "bazel_features-1.11.0",
    url = "https://github.com/bazel-contrib/bazel_features/releases/download/v1.11.0/bazel_features-v1.11.0.tar.gz",
)

http_archive(
    name = "aspect_bazel_lib",
    sha256 = "6d758a8f646ecee7a3e294fbe4386daafbe0e5966723009c290d493f227c390b",
    strip_prefix = "bazel-lib-2.7.7",
    url = "https://github.com/aspect-build/bazel-lib/releases/download/v2.7.7/bazel-lib-v2.7.7.tar.gz",
)

http_archive(
    name = "aspect_rules_swc",
    sha256 = "0c2e8912725a1d97a37bb751777c9846783758f5a0a8e996f1b9d21cad42e839",
    strip_prefix = "rules_swc-2.0.0-rc1",
    url = "https://github.com/aspect-build/rules_swc/releases/download/v2.0.0-rc1/rules_swc-v2.0.0-rc1.tar.gz",
)

http_archive(
    name = "aspect_rules_js",
    sha256 = "dfd2c5494b43704ab33574ae701b31b68ca27333e5da1a76b5e39374cdd8dda4",
    strip_prefix = "rules_js-2.0.0-rc7",
    url = "https://github.com/aspect-build/rules_js/releases/download/v2.0.0-rc7/rules_js-v2.0.0-rc7.tar.gz",
)

http_archive(
    name = "aspect_rules_ts",
    sha256 = "1d745fd7a5ffdb5bb7c0b77b36b91409a5933c0cbe25af32b05d90e26b7d14a7",
    strip_prefix = "rules_ts-3.0.0-rc2",
    url = "https://github.com/aspect-build/rules_ts/releases/download/v3.0.0-rc2/rules_ts-v3.0.0-rc2.tar.gz",
)

http_archive(
    name = "aspect_rules_lint",
    sha256 = "bd5a82b350cf20a662c45d6baa0f301a6a1a81833122e1d68a91a120e33a14dd",
    strip_prefix = "rules_lint-37d0160469035e4ea0f1824135cb198cbdcc59e0",
    url = "https://github.com/aspect-build/rules_lint/archive/37d0160469035e4ea0f1824135cb198cbdcc59e0.zip",
)

http_archive(
    name = "bazel_skylib",
    sha256 = "cd55a062e763b9349921f0f5db8c3933288dc8ba4f76dd9416aac68acee3cb94",
    urls = ["https://github.com/bazelbuild/bazel-skylib/releases/download/1.5.0/bazel-skylib-1.5.0.tar.gz"],
)

http_archive(
    name = "com_grail_bazel_toolchain",
    patch_args = ["-p1"],
    # Note: these commits are on the silo branch of aspect-forks/bazel-toolchain
    patches = [
        "//patches:com_grail_bazel_toolchain.patch",
        "//patches:com_grail_bazel_toolchain.001.patch",
    ],
    sha256 = "a9fc7cf01d0ea0a935bd9e3674dd3103766db77dfc6aafcb447a7ddd6ca24a78",
    strip_prefix = "toolchains_llvm-c65ef7a45907016a754e5bf5bfabac76eb702fd3",
    urls = ["https://github.com/bazel-contrib/toolchains_llvm/archive/c65ef7a45907016a754e5bf5bfabac76eb702fd3.tar.gz"],
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
    sha256 = "67b4d1f517ba73e0a92eb2f57d821f2ddc21f5bc2bd7a231573f11bd8758192e",
    urls = [
        "https://mirror.bazel.build/github.com/bazelbuild/rules_go/releases/download/v0.50.0/rules_go-v0.50.0.zip",
        "https://github.com/bazelbuild/rules_go/releases/download/v0.50.0/rules_go-v0.50.0.zip",
    ],
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

load("@bazel_features//:deps.bzl", "bazel_features_deps")

bazel_features_deps()

load("@aspect_bazel_lib//lib:repositories.bzl", "aspect_bazel_lib_dependencies", "register_copy_directory_toolchains", "register_copy_to_directory_toolchains", "register_coreutils_toolchains", "register_expand_template_toolchains", "register_jq_toolchains", "register_tar_toolchains", "register_yq_toolchains")

aspect_bazel_lib_dependencies()

register_copy_directory_toolchains()

register_copy_to_directory_toolchains()

register_coreutils_toolchains()

register_expand_template_toolchains()

register_tar_toolchains()

register_jq_toolchains()

register_yq_toolchains(version = "4.24.5")

load("@io_bazel_rules_go//go:deps.bzl", "go_register_toolchains", "go_rules_dependencies")

go_rules_dependencies()

go_register_toolchains(version = "1.23.0")

http_archive(
    name = "bazel_gazelle",
    patch_args = ["-p1"],
    patches = [
        "//:patches/bazelbuild_bazel-gazelle_aspect-cli.patch",
        "//:patches/bazelbuild_bazel-gazelle_aspect-walk-subdir.patch",
        "//:patches/bazelbuild_bazel-gazelle_aspect-gitignore.patch",
    ],
    sha256 = "872f1532567cdc53dc8e9f4681cd45021cd6787e2bde8a022bcec24a5867ce4c",
    # Ensure this version always matches the go.mod version.
    #
    # :notice: Care should be taken when upgrading gazelle since we have vendored & modified parts of gazelle
    # in the CLI configure command (pkg/aspect/configure).
    strip_prefix = "bazel-gazelle-571d953b2bb9534c145242ead08eb35b3b096a5e",
    urls = ["https://github.com/bazelbuild/bazel-gazelle/archive/571d953b2bb9534c145242ead08eb35b3b096a5e.tar.gz"],
)

http_archive(
    name = "rules_proto",
    sha256 = "303e86e722a520f6f326a50b41cfc16b98fe6d1955ce46642a5b7a67c11c0f5d",
    strip_prefix = "rules_proto-6.0.0",
    url = "https://github.com/bazelbuild/rules_proto/releases/download/6.0.0/rules_proto-6.0.0.tar.gz",
)

load("@rules_proto//proto:repositories.bzl", "rules_proto_dependencies")

rules_proto_dependencies()

http_archive(
    name = "toolchains_protoc",
    sha256 = "1f3cd768bbb92164952301228bac5e5079743843488598f2b17fecd41163cadb",
    strip_prefix = "toolchains_protoc-0.2.4",
    url = "https://github.com/aspect-build/toolchains_protoc/releases/download/v0.2.4/toolchains_protoc-v0.2.4.tar.gz",
)

load("@toolchains_protoc//protoc:toolchain.bzl", "protoc_toolchains")

protoc_toolchains(
    name = "protoc_toolchains",
    google_protobuf = "com_google_protobuf",
    version = "v21.7",
)

http_archive(
    name = "rules_python",
    integrity = "sha256-bERKXOYmJB6fdw/0TFPBLWen8f+81eG2p0EAneLkBAo=",
    strip_prefix = "rules_python-49cdf7d3fe000076d6432a34238e5d25f5b598d0",
    # NB: version matches go.mod where we fetch the rules_python/gazelle Go package.
    url = "https://github.com/bazelbuild/rules_python/archive/49cdf7d3fe000076d6432a34238e5d25f5b598d0.tar.gz",
)

load("@rules_python//python:repositories.bzl", "py_repositories")

py_repositories()

load("@rules_python//gazelle:deps.bzl", "python_stdlib_list_deps")

python_stdlib_list_deps()

load("@bazel_gazelle//:deps.bzl", "gazelle_dependencies")
load("//:go.bzl", _go_repositories = "deps")

# gazelle:repository_macro go.bzl%deps
_go_repositories()

gazelle_dependencies()

load("//gazelle/common/treesitter/grammars:grammars.bzl", "fetch_grammars")

fetch_grammars()

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

load("@aspect_rules_js//npm:repositories.bzl", "npm_translate_lock")

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

load("@buildifier_prebuilt//:defs.bzl", "buildifier_prebuilt_register_toolchains")

buildifier_prebuilt_register_toolchains()

load("@aspect_rules_lint//format:repositories.bzl", "rules_lint_dependencies")

rules_lint_dependencies()

load("@rules_multitool//multitool:multitool.bzl", "multitool")

multitool(
    name = "multitool",
    lockfile = "@aspect_rules_lint//format:multitool.lock.json",
)

load("//.aspect/workflows:deps.bzl", "fetch_workflows_deps")

fetch_workflows_deps()
