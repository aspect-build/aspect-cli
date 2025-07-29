load("//gazelle/host/tests/starzelle/starlark_load/utils:identity.star", "identity")

def declare_targets(ctx):
    if len(ctx.sources) == 0:
        ctx.targets.remove("all-files")
        return

    ctx.targets.add(
        name = "all-files",
        kind = "filegroup",
        attrs = {
            "srcs": identity([s.path for s in ctx.sources]),
        },
    )
