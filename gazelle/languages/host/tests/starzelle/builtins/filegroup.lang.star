def prepare(_):
    return aspect.PrepareResult(
        sources = [
            aspect.SourceGlobs("**/*.*"),
        ],
    )

def declare_targets(ctx):
    if len(ctx.sources) == 0:
        ctx.targets.remove("all-files")
        return

    ctx.targets.add(
        name = "all-files",
        kind = "filegroup",
        attrs = {
            "srcs": [s.path for s in ctx.sources],
        },
    )

aspect.register_configure_extension(
    id = "fgs",
    prepare = prepare,
    declare = declare_targets,
)
