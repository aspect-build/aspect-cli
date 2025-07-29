def declare_main_js(ctx):
    if len(ctx.sources) == 0:
        ctx.targets.remove("main")
        return

    ctx.targets.add(
        name = "lib",
        kind = "js_binary",
        attrs = {
            "entry_point": ctx.sources[0],
        },
        symbols = [aspect.Symbol(
            id = ctx.rel,
            provider = "tool",
        )],
    )

aspect.register_configure_extension(
    id = "main_bin",
    prepare = lambda _: aspect.PrepareResult(
        sources = aspect.SourceGlobs("main.js"),
    ),
    declare = declare_main_js,
)

def run_main_js(ctx):
    if len(ctx.sources) == 0:
        ctx.targets.remove("bin")
        return

    ctx.targets.add(
        name = "bin",
        kind = "js_run_binary",
        attrs = {
            "srcs": ctx.sources,
            # The entry point file + data
            # TODO: this should be an Import that gets resolved!!!
            # "tool": aspect.Import(
            #     id = "tool" + ctx.sources[0].path.removesuffix(".txt").removeprefix("input"),
            #     provider = "tool",
            # ),
            "tool": aspect.Label(
                pkg = "tool" + ctx.sources[0].path.removesuffix(".txt").removeprefix("input"),
                name = "bin",
            ),
        },
    )

aspect.register_configure_extension(
    id = "main_js",
    prepare = lambda _: aspect.PrepareResult(
        sources = aspect.SourceGlobs("input*.txt"),
    ),
    declare = run_main_js,
)
