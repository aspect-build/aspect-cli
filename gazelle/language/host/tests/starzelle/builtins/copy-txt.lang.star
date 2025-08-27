def prepare(_):
    return aspect.PrepareResult(
        sources = aspect.SourceGlobs("**/*.txt"),
    )

def declare_targets(ctx):
    if len(ctx.sources) == 0:
        ctx.targets.remove("ctb")
        return

    ctx.targets.add(
        name = "ctb",
        kind = "copy_to_bin",
        attrs = {
            "srcs": [s.path for s in ctx.sources],
        },
    )

aspect.register_configure_extension(
    id = "copy-txt",
    prepare = prepare,
    declare = declare_targets,
)
