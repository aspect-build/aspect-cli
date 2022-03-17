workspace(name = "build_aspect_cli")

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_archive")

http_archive(
    name = "com_google_protobuf",
    sha256 = "d0f5f605d0d656007ce6c8b5a82df3037e1d8fe8b121ed42e536f569dec16113",
    strip_prefix = "protobuf-3.14.0",
    urls = [
        "https://mirror.bazel.build/github.com/protocolbuffers/protobuf/archive/v3.14.0.tar.gz",
        "https://github.com/protocolbuffers/protobuf/archive/v3.14.0.tar.gz",
    ],
)

load("@com_google_protobuf//:protobuf_deps.bzl", "protobuf_deps")

protobuf_deps()

http_archive(
    name = "aspect_bazel_lib",
    sha256 = "4d709266337d8e147fb647d6dfa3bbcfb6983ab79bece9cf661e3dc13ae3c3bc",
    strip_prefix = "bazel-lib-0.6.1",
    url = "https://github.com/aspect-build/bazel-lib/archive/refs/tags/v0.6.1.tar.gz",
)

http_archive(
    name = "bazel_skylib",
    sha256 = "58f558d04a936cade1d4744d12661317e51f6a21e3dd7c50b96dc14f3fa3b87d",
    strip_prefix = "bazel-skylib-df3c9e2735f02a7fe8cd80db4db00fec8e13d25f",
    urls = [
        "https://github.com/bazelbuild/bazel-skylib/archive/df3c9e2735f02a7fe8cd80db4db00fec8e13d25f.tar.gz",
    ],
)

http_archive(
    name = "io_bazel_rules_go",
    sha256 = "d10309af5f19c08cb489760d547414135dc64c46e38071225b834588e4ef1a87",
    strip_prefix = "rules_go-134efd77b2a1d81a0ab86661ccdc5328d43e47dc",
    # From March 15, 2022
    urls = ["https://github.com/aspect-build/rules_go/archive/134efd77b2a1d81a0ab86661ccdc5328d43e47dc.zip"],
)

load("@io_bazel_rules_go//extras:embed_data_deps.bzl", "go_embed_data_dependencies")
load("@io_bazel_rules_go//go:deps.bzl", "go_register_toolchains", "go_rules_dependencies")

go_rules_dependencies()

go_embed_data_dependencies()

go_register_toolchains(
    nogo = "@//:nogo",
    version = "1.18",
)

http_archive(
    name = "bazel_gazelle",
    sha256 = "de69a09dc70417580aabf20a28619bb3ef60d038470c7cf8442fafcf627c21cb",
    urls = [
        "https://mirror.bazel.build/github.com/bazelbuild/bazel-gazelle/releases/download/v0.24.0/bazel-gazelle-v0.24.0.tar.gz",
        "https://github.com/bazelbuild/bazel-gazelle/releases/download/v0.24.0/bazel-gazelle-v0.24.0.tar.gz",
    ],
)

http_archive(
    name = "rules_proto",
    sha256 = "9fc210a34f0f9e7cc31598d109b5d069ef44911a82f507d5a88716db171615a8",
    strip_prefix = "rules_proto-f7a30f6f80006b591fa7c437fe5a951eb10bcbcf",
    urls = ["https://github.com/bazelbuild/rules_proto/archive/f7a30f6f80006b591fa7c437fe5a951eb10bcbcf.tar.gz"],
)

load("@rules_proto//proto:repositories.bzl", "rules_proto_dependencies", "rules_proto_toolchains")

rules_proto_dependencies()

rules_proto_toolchains()

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
