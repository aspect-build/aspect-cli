workspace(name = "build_aspect_cli")

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

http_archive(
    name = "aspect_bazel_lib",
    sha256 = "f1c181b910f821072f38ee45bb87db6b56275458c7f832c54c54ba6334119eca",
    strip_prefix = "bazel-lib-1.32.0",
    url = "https://github.com/aspect-build/bazel-lib/releases/download/v1.32.0/bazel-lib-v1.32.0.tar.gz",
)

http_archive(
    name = "aspect_rules_js",
    sha256 = "08061ba5e5e7f4b1074538323576dac819f9337a0c7d75aee43afc8ae7cb6e18",
    strip_prefix = "rules_js-1.26.1",
    url = "https://github.com/aspect-build/rules_js/releases/download/v1.26.1/rules_js-v1.26.1.tar.gz",
)

http_archive(
    name = "bazel_skylib",
    sha256 = "de9d2cedea7103d20c93a5cc7763099728206bd5088342d0009315913a592cc0",
    strip_prefix = "bazel-skylib-1.4.2",
    urls = ["https://github.com/bazelbuild/bazel-skylib/archive/refs/tags/1.4.2.tar.gz"],
)

http_archive(
    name = "aspect_gcc_toolchain",
    sha256 = "3341394b1376fb96a87ac3ca01c582f7f18e7dc5e16e8cf40880a31dd7ac0e1e",
    strip_prefix = "gcc-toolchain-0.4.2",
    urls = ["https://github.com/aspect-build/gcc-toolchain/archive/refs/tags/0.4.2.tar.gz"],
)

http_archive(
    name = "com_grail_bazel_toolchain",
    patch_args = ["-p1"],
    patches = ["//patches:com_grail_bazel_toolchain.patch"],
    sha256 = "f6fd877e6f48092383e3dae80c28564450125e11fca31f35fba3ba599c9fd10d",
    strip_prefix = "bazel-toolchain-ceeedcc4464322e05fe5b8df3749cc02273ee083",
    urls = ["https://github.com/grailbio/bazel-toolchain/archive/ceeedcc4464322e05fe5b8df3749cc02273ee083.tar.gz"],
)

_SYSROOT_BUILD_FILE = """
filegroup(
    name = "sysroot",
    srcs = glob(["*/**"]),
    visibility = ["//visibility:public"],
)
"""

http_archive(
    name = "org_chromium_sysroot_linux_arm64",
    build_file_content = _SYSROOT_BUILD_FILE,
    sha256 = "e39b700d8858d18868544c8c84922f6adfa8419f3f42471b92024ba38eff7aca",
    urls = ["https://commondatastorage.googleapis.com/chrome-linux-sysroot/toolchain/2202c161310ffde63729f29d27fe7bb24a0bc540/debian_stretch_arm64_sysroot.tar.xz"],
)

http_archive(
    name = "org_chromium_sysroot_linux_x86_64",
    build_file_content = _SYSROOT_BUILD_FILE,
    sha256 = "84656a6df544ecef62169cfe3ab6e41bb4346a62d3ba2a045dc5a0a2ecea94a3",
    urls = ["https://commondatastorage.googleapis.com/chrome-linux-sysroot/toolchain/2202c161310ffde63729f29d27fe7bb24a0bc540/debian_stretch_amd64_sysroot.tar.xz"],
)

http_archive(
    name = "io_bazel_rules_go",
    patch_args = ["-p1"],
    patches = ["//patches:rules_go.patch"],
    sha256 = "19ef30b21eae581177e0028f6f4b1f54c66467017be33d211ab6fc81da01ea4d",
    urls = ["https://github.com/bazelbuild/rules_go/releases/download/v0.38.0/rules_go-v0.38.0.zip"],
)

http_archive(
    name = "rules_pkg",
    sha256 = "8c20f74bca25d2d442b327ae26768c02cf3c99e93fad0381f32be9aab1967675",
    urls = ["https://github.com/bazelbuild/rules_pkg/releases/download/0.8.1/rules_pkg-0.8.1.tar.gz"],
)

http_archive(
    name = "buildifier_prebuilt",
    sha256 = "95387c9dded7f8e3bdd4c598bc2ca4fbb6366cb214fa52e7d7b689eb2f421e01",
    strip_prefix = "buildifier-prebuilt-6.0.0",
    urls = ["https://github.com/keith/buildifier-prebuilt/archive/6.0.0.tar.gz"],
)

load("@buildifier_prebuilt//:deps.bzl", "buildifier_prebuilt_deps")

buildifier_prebuilt_deps()

load("@aspect_bazel_lib//lib:repositories.bzl", "aspect_bazel_lib_dependencies")

aspect_bazel_lib_dependencies()

load("@io_bazel_rules_go//extras:embed_data_deps.bzl", "go_embed_data_dependencies")
load("@io_bazel_rules_go//go:deps.bzl", "go_register_toolchains", "go_rules_dependencies")

go_rules_dependencies()

go_embed_data_dependencies()

go_register_toolchains(version = "1.20.4")

http_archive(
    name = "bazel_gazelle",
    sha256 = "29d5dafc2a5582995488c6735115d1d366fcd6a0fc2e2a153f02988706349825",
    # Ensure this version always matches the version of @com_github_bazelbuild_bazel_gazelle set in deps.bzl.
    # :notice: Care should be taken when upgrading gazelle since we have vendored & modified parts of gazelle
    # in the CLI configure command (cli/core/pkg/aspect/configure).
    urls = ["https://github.com/bazelbuild/bazel-gazelle/releases/download/v0.31.0/bazel-gazelle-v0.31.0.tar.gz"],
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
    llvm_version = "15.0.7",
    sha256 = {
        "darwin-arm64": "867c6afd41158c132ef05a8f1ddaecf476a26b91c85def8e124414f9a9ba188d",
        "darwin-x86_64": "d16b6d536364c5bec6583d12dd7e6cf841b9f508c4430d9ee886726bd9983f1c",
    },
    strip_prefix = {
        "darwin-arm64": "clang+llvm-15.0.7-arm64-apple-darwin22.0",
        "darwin-x86_64": "clang+llvm-15.0.7-x86_64-apple-darwin21.0",
    },
    sysroot = {
        "linux-aarch64": "@org_chromium_sysroot_linux_arm64//:sysroot",
        "linux-x86_64": "@org_chromium_sysroot_linux_x86_64//:sysroot",
    },
    urls = {
        "darwin-arm64": ["https://github.com/llvm/llvm-project/releases/download/llvmorg-15.0.7/clang+llvm-15.0.7-arm64-apple-darwin22.0.tar.xz"],
        "darwin-x86_64": ["https://github.com/llvm/llvm-project/releases/download/llvmorg-15.0.7/clang+llvm-15.0.7-x86_64-apple-darwin21.0.tar.xz"],
    },
)

register_toolchains("//platforms/toolchains:llvm")

load("@aspect_gcc_toolchain//toolchain:repositories.bzl", "gcc_toolchain_dependencies")

gcc_toolchain_dependencies()

load("@aspect_gcc_toolchain//toolchain:defs.bzl", "ARCHS", "gcc_register_toolchain")

gcc_register_toolchain(
    name = "gcc_toolchain_aarch64",
    target_arch = ARCHS.aarch64,
)

gcc_register_toolchain(
    name = "gcc_toolchain_x86_64",
    target_arch = ARCHS.x86_64,
)

load("@buildifier_prebuilt//:defs.bzl", "buildifier_prebuilt_register_toolchains")

buildifier_prebuilt_register_toolchains()
