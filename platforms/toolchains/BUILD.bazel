load("@bazel_skylib//:bzl_library.bzl", "bzl_library")
load(":defs.bzl", "platforms")

[toolchain(
    name = "{}_{}_{}_{}_llvm".format(
        p.exe_os,
        p.exe_cpu,
        p.tgt_os,
        p.tgt_cpu,
    ),
    exec_compatible_with = [
        "@platforms//os:{}".format(p.exe_os),
        "@platforms//cpu:{}".format(p.exe_cpu),
    ],
    target_compatible_with = [
        "@platforms//os:{}".format(p.tgt_os),
        "@platforms//cpu:{}".format(p.tgt_cpu),
    ],
    toolchain = "@llvm_toolchain//:cc-clang-{}-{}".format(
        p.tgt_cpu,
        p.tgt_os if p.tgt_os != "macos" else "darwin",
    ),
    toolchain_type = "@bazel_tools//tools/cpp:toolchain_type",
    visibility = ["//visibility:public"],
) for p in platforms]

bzl_library(
    name = "defs",
    srcs = ["defs.bzl"],
    visibility = ["//visibility:public"],
)
