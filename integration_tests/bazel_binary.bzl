"repository rules for downloading bazel"

load("@aspect_bazel_lib//lib:repo_utils.bzl", "repo_utils")

BAZEL_VERSIONS = {
    "6.0.0": {
        "darwin-arm64": "8b00a2ea4010614742b2c20efd390b247b67217ef906c20712cdce7a1c16e027",
        "darwin-x86_64": "8e543c5c9f1c8c91df945cd2fb4c3b43587929a43044a0ed87d13da0d19f96e8",
        "linux-arm64": "408c33a0edb8f31374da47e011eef88c360264f268e9a4e3d9e699fbd5e57ad3",
        "linux-x86_64": "f03d44ecaac3878e3d19489e37caa4ca1dc57427b686a78a85065ea3c27ebe68",
    },
}

def _bazel_binary_impl(rctx):
    # TODO: make this configurable if needed in the future.
    version = "6.0.0"
    version_without_rc = "6.0.0"
    release_type = "release"
    platform = repo_utils.platform(rctx).replace("_", "-").replace("amd64", "x86_64")

    filename = "bazel-{version}-{platform}".format(
        version = version,
        platform = platform,
    )
    url = "https://releases.bazel.build/{version}/{release_type}/{filename}".format(
        filename = filename,
        version = version_without_rc,
        release_type = release_type,
    )

    rctx.download(
        url = [url],
        output = "bazel",
        executable = True,
        sha256 = BAZEL_VERSIONS[version][platform],
    )

    rctx.file(
        "BUILD.bazel",
        """
exports_files(["bazel"])
""",
    )

bazel_binary = repository_rule(
    implementation = _bazel_binary_impl,
)

def bazel_binaries():
    bazel_binary(name = "bazel_6_0_0")
