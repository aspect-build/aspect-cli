# Run from a workspace with a //lib package: the fixture used in the README (BUILD.bazel with //:all, //:hello and //lib:lib).
def _impl(ctx: Context):
    ctx.analysis().query("//...")
    runtime = ctx.java.runtime
    ctx.output.print("product:", runtime.product_name)
    ctx.output.print("dir:", dir(runtime)[:10])
    pkg = ctx.analysis().package("//lib")
    ctx.output.print("package:", pkg)
    for name in pkg.targets:
        target = pkg.targets[name]
        ctx.output.print(" ", name, target)
        if hasattr(target, "getAttr"):
            srcs = target.getAttr("srcs")
            ctx.output.print("    srcs:", srcs, srcs[0].name)

main = buildx_main(impl = _impl, cli_args = {})
