"go_proto_library wrapper macro"

load("@aspect_bazel_lib//lib:default_info_files.bzl", "make_default_info_files")
load("@aspect_bazel_lib//lib:write_source_files.bzl", "write_source_files")
load("@bazel_skylib//lib:paths.bzl", "paths")
load("@io_bazel_rules_go//proto:def.bzl", _go_proto_library = "go_proto_library")

def go_proto_library(name, importpath, proto_srcs = [], **kwargs):
    """Wrap go_proto_library with write_source_files.

    This causes the resulting .pb.go files to be checked into the source tree.
    Args:
        name: name of the go_proto_library rule produced
        importpath: passed to go_proto_library#importpath
        proto_srcs: the srcs of the proto_library target passed to go_proto_library#proto
            If unset, a glob() of all ".proto" files in the package is used.
        **kwargs: remaining arguments to go_proto_library
    """

    # Based on our knowledge of the rule implementation,
    # predict the output paths it writes.
    proto_out_path = "{0}/{1}_/{2}/%s.pb.go".format(
        native.package_name(),
        name,
        importpath,
    )

    gen_srcs_filegroup = "_{}.gensrcs".format(name)

    if len(proto_srcs) < 1:
        proto_srcs = native.glob(["*.proto"])

    _go_proto_library(
        name = name,
        importpath = importpath,
        **kwargs
    )

    native.filegroup(
        name = gen_srcs_filegroup,
        srcs = [name],
        output_group = "go_generated_srcs",
    )

    write_source_files(
        name = name + ".update_go_pb",
        files = {
            base + ".pb.go": make_default_info_files(base + "_pb_go", gen_srcs_filegroup, [proto_out_path % base])
            for base in [paths.replace_extension(p, "") for p in proto_srcs]
        },
        visibility = ["//:__pkg__"],
    )
