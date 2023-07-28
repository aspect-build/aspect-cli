"repository rules for downloading bazel"

load("@aspect_bazel_lib//lib:repo_utils.bzl", "repo_utils")

# Visit https://github.com/bazelbuild/bazel/releases/tag/x.x.x and download the corresponding
# .sha256 files to get the SHAs for new versions
BAZEL_VERSIONS = {
    "6.2.0": {
        "darwin-arm64": "482957a15c34eb43b1d1ae5e7623444e4783a04d4c618d7c518fe7b3dbf75512",
        "darwin-x86_64": "d2356012843ce3a2fbba89f88191673a6ad2f7716cc46ad43ec1bcee78d36b44",
        "linux-arm64": "16e41fe8fb791ffb9835643435e4828384a1890b0f916fd84b750fa01f783807",
        "linux-x86_64": "3d11c26fb9ba12c833844450bb90165b176e8a19cb5cf5923f3cec855837f17c",
    },
    "6.2.1": {
        "darwin-arm64": "0e4409d3243bf04bb709d3f1cc8a32ec0c36475c6d2aeda8475a213c40470793",
        "darwin-x86_64": "dd69512405d7a07c14ee2b33c8e1cb434b2eac203b3d46e17e7acb797608db22",
        "linux-arm64": "98d17ba59885957fe0dda423a52cfc3edf91176d4a7b3bdc5b573975a3785e1e",
        "linux-x86_64": "cdf349dc938b1f11db5a7172269d66e91ce18c0ca134b38bb3997a3e3be782b8",
    },
    "6.3.0": {
        "darwin-arm64": "94f797719cad71ee0c8f710797be334c9b94e9ad9ae86c3a9e45c3986113643e",
        "darwin-x86_64": "16f86ca1536fa9e1c7bb584de5425f935d391ae8ec6bb34f4c4f176a66efb21f",
        "linux-arm64": "647ccd5269c12ba724aa041b10e3dad8d7a0cfeeae4b9eac3ebcaa0774e8fcac",
        "linux-x86_64": "d64606c17e6b6a7fc119150420b4c109315982319ff3229587e200c47bf36946",
    },
}

def _bazel_binary_impl(rctx):
    # TODO: make this configurable if needed in the future.
    version = "6.3.0"
    version_without_rc = "6.3.0"
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
    bazel_binary(name = "bazel_6_2_1")
