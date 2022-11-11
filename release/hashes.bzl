"""Rule for generating integrity files

Default output is a .sha256 file but .sha1 and .md5 files are also available
via output groups.

Based on https://github.com/bazelbuild/examples/blob/main/rules/implicit_output/hash.bzl
"""

def _impl(ctx):
    # Create actions to generate the three output files.
    # Actions are run only when the corresponding file is requested.

    md5out = ctx.actions.declare_file("{}.md5".format(ctx.file.src.basename))
    ctx.actions.run_shell(
        outputs = [md5out],
        inputs = [ctx.file.src],
        command = "ROOT=$PWD && cd {} && md5sum {} > $ROOT/{}".format(ctx.file.src.dirname, ctx.file.src.basename, md5out.path),
    )

    sha1out = ctx.actions.declare_file("{}.sha1".format(ctx.file.src.basename))
    ctx.actions.run_shell(
        outputs = [sha1out],
        inputs = [ctx.file.src],
        command = "ROOT=$PWD && cd {} && sha1sum {} > $ROOT/{}".format(ctx.file.src.dirname, ctx.file.src.basename, sha1out.path),
    )

    sha256out = ctx.actions.declare_file("{}.sha256".format(ctx.file.src.basename))
    ctx.actions.run_shell(
        outputs = [sha256out],
        inputs = [ctx.file.src],
        command = "ROOT=$PWD && cd {} && sha256sum {} > $ROOT/{}".format(ctx.file.src.dirname, ctx.file.src.basename, sha256out.path),
    )

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
        "src": attr.label(mandatory = True, allow_single_file = True),
    },
)

def hashes(name, src, **kwargs):
    _hashes(
        name = name,
        src = src,
        **kwargs
    )
