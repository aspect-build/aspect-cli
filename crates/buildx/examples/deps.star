# Run from a workspace with a //lib package: the fixture used in the README (BUILD.bazel with //:all, //:hello and //lib:lib).
def _impl(ctx: Context):
    first = ctx.analysis().query("deps(//:all)")
    second = ctx.analysis().query("kind(rule, //...)")

    for l in first:
        print(l.label)

    for l in second:
        print(l.label)



main = buildx_main(impl = _impl, cli_args = {})
