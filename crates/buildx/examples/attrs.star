# Run from a workspace with a //lib package: the fixture used in the README (BUILD.bazel with //:all, //:hello and //lib:lib).
def _impl(ctx: Context):
    for t in ctx.analysis().query("//:all"):
        srcs = t.getAttr("srcs")
        ctx.output.print(t.label, len(srcs), srcs)
        ctx.output.print("first:", srcs[0], srcs[0].name, srcs[0].package)
        ctx.output.print("labels:", [str(s) for s in srcs])
        ctx.output.print("contains:", srcs[0] in srcs, srcs[0] == srcs[0], srcs[0] != srcs[1])
        ctx.output.print("visibility:", t.visibility, "tags:", t.getAttr("tags"))

main = buildx_main(impl = _impl, cli_args = {})
