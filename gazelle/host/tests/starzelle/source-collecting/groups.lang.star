def prepare_list(_):
    return aspect.PrepareResult(
        sources = {
            "js": [aspect.SourceGlobs("**/*.js")],
            "txt": [aspect.SourceGlobs("**/*.txt")],
        },
    )

def declare_list(ctx):
    if ctx.sources.js:
        ctx.targets.add(
            name = "the-dict-js",
            kind = "filegroup",
            attrs = {
                "srcs": ctx.sources.js,
            },
        )

    if ctx.sources.txt:
        ctx.targets.add(
            name = "the-dict-txt",
            kind = "filegroup",
            attrs = {
                "srcs": ctx.sources.txt,
            },
        )

    if ctx.sources:
        ctx.targets.add(
            name = "the-dict-all",
            kind = "filegroup",
            attrs = {
                "srcs": [s.path for s in ctx.sources],
            },
        )

aspect.register_configure_extension(
    id = "fg-groups",
    prepare = prepare_list,
    declare = declare_list,
)
