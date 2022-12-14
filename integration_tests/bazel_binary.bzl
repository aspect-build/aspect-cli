"repository rules for downloading bazel"

load("@aspect_bazel_lib//lib:repo_utils.bzl", "repo_utils")

BAZEL_VERSIONS = {
    "6.0.0rc4": {
        "darwin-arm64": "31bf36c1379bd27c9b3cfbd6d35870018883b1132593436bfb1dd1ac08556671",
        "darwin-x86_64": "76fb4e652303b1923abd3315a6b9cc65e14fbb9adad434613dffb927e245bd69",
        "linux-arm64": "00bd6e2e40d14625729e71b595e711f880b9d968342b2bb2f7cafe352ad81e64",
        "linux-x86_64": "388695ac574d9f67bab5ef3aa0ecf7d38de89709fc1a80c1be954fb00e5d6c09",
    },
}

def _bazel_binary_impl(rctx):
    # TODO: make this configurable if needed in the future.
    version = "6.0.0rc4"
    version_without_rc = "6.0.0"
    release_type = "rc4"
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
    bazel_binary(name = "bazel_6_0_0rc4")
