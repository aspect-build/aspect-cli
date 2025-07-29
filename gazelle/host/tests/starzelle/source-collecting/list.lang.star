def prepare_list(_):
    return aspect.PrepareResult(
        sources = [
            aspect.SourceGlobs("**/*.*"),
        ],
    )

def declare_list(ctx):
    if not ctx.sources:
        return

    ctx.targets.add(
        name = "the-list",
        kind = "filegroup",
        attrs = {
            "srcs": ctx.sources,
        },
    )

aspect.register_configure_extension(
    id = "fg-list",
    prepare = prepare_list,
    declare = declare_list,
)
