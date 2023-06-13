"""Rules for generating hash values"""

load("@bazel_skylib//lib:dicts.bzl", "dicts")
load("@bazel_tools//tools/build_defs/hash:hash.bzl", "tools", _sha256 = "sha256")

def _sha256_impl(ctx):
    out = _sha256(ctx, ctx.file.artifact)
    files = depset(direct = [out])
    runfiles = ctx.runfiles(files = [out])
    return [DefaultInfo(files = files, runfiles = runfiles)]

sha256 = rule(
    implementation = _sha256_impl,
    attrs = dicts.add({
        "artifact": attr.label(
            allow_single_file = True,
            mandatory = True,
            doc = "The artifact whose sha256 value should be calculated.",
        ),
    }, tools),
    doc = "Calculate the SHA256 hash value for a file.",
    provides = [DefaultInfo],
)
