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
    "6.3.1": {
        "darwin-arm64": "b31a3dd7d5cfa7eb25856075fe85ef6180d0a25499f8183a7bb85e5b88d6158b",
        "darwin-x86_64": "49a6d5f96ce89a9cfb320378293de214df5a4ac22b002a978e1f8a23fb3ceb83",
        "linux-arm64": "ac70546fd207a98d500f118f621c6e15f918786cb5f0a5bb9ca709b433fb5a9b",
        "linux-x86_64": "81130d324e145dcf3192338b875669fe5f410fef26344985dd4cdcdb1c7cab5b",
    },
    "6.3.2": {
        "darwin-arm64": "c3e8a47b9926adc305cacf64e6d17964dfa08c570c139a734e00c381bf38ba49",
        "darwin-x86_64": "78f7417b4dd9193dba7b753d5a7069497185a020d87e9076a577871994b59ead",
        "linux-arm64": "9d88a0b206e22cceb4afe0060be7f294b423f5f49b18750fbbd7abd47cea4054",
        "linux-x86_64": "e78fc3394deae5408d6f49a15c7b1e615901969ecf6e50d55ef899996b0b8458",
    },
    "6.4.0": {
        "darwin-arm64": "574d54bfb7e84adf3fe29bc2a0d13214048c1b5e295f826fced3fb94fba282ee",
        "darwin-x86_64": "eef2661dabc3de09c9c8f839f7789b29763ea9987659e432b3c4e6246b3fe5df",
        "linux-arm64": "1df1147765ad4aa23d7f12045b8e8d21b47db40525de69c877ac49234bf2d22d",
        "linux-x86_64": "79e4f370efa6e31717b486af5d9efd95864d0ef13da138582224ac9b2a1bad86",
    },
}

def _bazel_binary_impl(rctx):
    # TODO: make this configurable if needed in the future.
    version = "6.4.0"
    version_without_rc = "6.4.0"
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
