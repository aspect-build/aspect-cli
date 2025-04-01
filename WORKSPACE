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
    name = "aspect_rules_swc",
    sha256 = "e5ac926ebe1bbef1f38d245a65626d86f114eb1f3c68362e8a33472351d83608",
    strip_prefix = "rules_swc-2.0.1",
    url = "https://github.com/aspect-build/rules_swc/releases/download/v2.0.1/rules_swc-v2.0.1.tar.gz",
)

http_archive(
    name = "aspect_rules_js",
    sha256 = "75c25a0f15a9e4592bbda45b57aa089e4bf17f9176fd735351e8c6444df87b52",
    strip_prefix = "rules_js-2.1.0",
    url = "https://github.com/aspect-build/rules_js/releases/download/v2.1.0/rules_js-v2.1.0.tar.gz",
)

http_archive(
    name = "aspect_rules_ts",
    sha256 = "8bbac753f4b61adbfc1d9878b87b9cd0f64c9e8e6d8fafc8a1bbfa9625bab162",
    strip_prefix = "rules_ts-3.2.1",
    url = "https://github.com/aspect-build/rules_ts/releases/download/v3.2.1/rules_ts-v3.2.1.tar.gz",
)

http_archive(
    name = "aspect_rules_lint",
    sha256 = "7d5feef9ad85f0ba78cc5757a9478f8fa99c58a8cabc1660d610b291dc242e9b",
    strip_prefix = "rules_lint-1.0.2",
    url = "https://github.com/aspect-build/rules_lint/releases/download/v1.0.2/rules_lint-v1.0.2.tar.gz",
)

http_archive(
    name = "bazel_skylib",
    sha256 = "bc283cdfcd526a52c3201279cda4bc298652efa898b10b4db0837dc51652756f",
    urls = [
        "https://mirror.bazel.build/github.com/bazelbuild/bazel-skylib/releases/download/1.7.1/bazel-skylib-1.7.1.tar.gz",
        "https://github.com/bazelbuild/bazel-skylib/releases/download/1.7.1/bazel-skylib-1.7.1.tar.gz",
    ],
)

# Ensure this version always matches the go.mod version.
http_archive(
    name = "io_bazel_rules_go",
    sha256 = "90fe8fb402dee957a375f3eb8511455bd738c7ed562695f4dd117ac7d2d833b1",
    urls = [
        "https://mirror.bazel.build/github.com/bazelbuild/rules_go/releases/download/v0.52.0/rules_go-v0.52.0.zip",
        "https://github.com/bazelbuild/rules_go/releases/download/v0.52.0/rules_go-v0.52.0.zip",
    ],
)

http_archive(
    name = "rules_pkg",
    sha256 = "8f9ee2dc10c1ae514ee599a8b42ed99fa262b757058f65ad3c384289ff70c4b8",
    urls = ["https://github.com/bazelbuild/rules_pkg/releases/download/0.9.1/rules_pkg-0.9.1.tar.gz"],
)

http_archive(
    name = "buildifier_prebuilt",
    sha256 = "bf9101bd5d657046674167986a18d44c5612e417194dc55aff8ca174344de031",
    strip_prefix = "buildifier-prebuilt-8.0.3",
    urls = [
        "http://github.com/keith/buildifier-prebuilt/archive/8.0.3.tar.gz",
    ],
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

go_register_toolchains(version = "1.24.1")

load("//gazelle:deps.bzl", fetch_gazelle_deps = "fetch_deps")

fetch_gazelle_deps()

load("@bazel_gazelle//:deps.bzl", "gazelle_dependencies")
load("//:go.bzl", _go_repositories = "deps")

# gazelle:repository_macro go.bzl%deps
_go_repositories()

gazelle_dependencies()

load("//gazelle/common/treesitter/grammars:grammars.bzl", "fetch_grammars")

fetch_grammars()

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
