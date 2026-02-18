"""Rule for generating integrity files

Default output is a .sha256 file but .sha1 and .md5 files are also available
via output groups.

Based on https://github.com/bazelbuild/examples/blob/main/rules/implicit_output/hash.bzl
"""

_COREUTILS_TOOLCHAIN = "@aspect_bazel_lib//lib:coreutils_toolchain_type"

def _hash_action(ctx, coreutils, algorithm, src, out):
    ctx.actions.run_shell(
        outputs = [out],
        inputs = [src],
        tools = [coreutils.bin],
        command = "{coreutils} hashsum --{algorithm} {src} > {out}".format(
            coreutils = coreutils.bin.path,
            algorithm = algorithm,
            src = src.path,
            out = out.path,
        ),
        toolchain = _COREUTILS_TOOLCHAIN,
    )

def _impl(ctx):
    # Create actions to generate the three output files.
    # Actions are run only when the corresponding file is requested.

    if ctx.file.src.is_directory:
        fail("src expected to be a file but got a directory")

    coreutils = ctx.toolchains[_COREUTILS_TOOLCHAIN].coreutils_info

    md5out = ctx.actions.declare_file("{}.md5".format(ctx.file.src.basename))
    _hash_action(ctx, coreutils, "md5", ctx.file.src, md5out)

    sha1out = ctx.actions.declare_file("{}.sha1".format(ctx.file.src.basename))
    _hash_action(ctx, coreutils, "sha1", ctx.file.src, sha1out)

    sha256out = ctx.actions.declare_file("{}.sha256".format(ctx.file.src.basename))
    _hash_action(ctx, coreutils, "sha256", ctx.file.src, sha256out)

    # By default (if you run `bazel build` on this target, or if you use it as a
    # source of another target), only the sha256 is computed.
    return [
        DefaultInfo(
            files = depset([sha256out]),
        ),
        OutputGroupInfo(
            md5 = depset([md5out]),
            sha1 = depset([sha1out]),
            sha256 = depset([sha256out]),
        ),
    ]

_hashes = rule(
    implementation = _impl,
    attrs = {
        "src": attr.label(
            allow_single_file = True,
            mandatory = True,
        ),
    },
    toolchains = [_COREUTILS_TOOLCHAIN],
)

def hashes(name, src, **kwargs):
    _hashes(
        name = name,
        src = src,
        **kwargs
    )
