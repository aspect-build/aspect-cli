# Run from a workspace with a //lib package: the fixture used in the README (BUILD.bazel with //:all, //:hello and //lib:lib).
def _impl(ctx: Context):
    targets = ctx.analysis().query("//...")
    ctx.output.print(len(targets), "targets:", targets)
    for t in targets:
        kind = t.target_kind
        ctx.output.print(kind, t.label)

main = buildx_main(impl = _impl, cli_args = {})
