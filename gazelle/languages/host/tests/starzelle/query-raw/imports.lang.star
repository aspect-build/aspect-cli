aspect.register_rule_kind("r_lib", {
    "From": "@deps-test//my:rules.bzl",
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def prepare(_):
    return aspect.PrepareResult(
        # All source files to be processed
        sources = [
            aspect.SourceExtensions(".r"),
        ],
        queries = {
            "imports": aspect.RawQuery(
                filter = "*.r",
            ),
        },
    )

def declare(ctx):
    for file in ctx.sources:
        ctx.targets.add(
            name = file.path[:file.path.rindex(".")] + "_lib",
            kind = "r_lib",
            attrs = {
                "srcs": [file.path],
                "deps": [
                    aspect.Import(
                        id = i,
                        provider = "r",
                        src = file.path,
                    )
                    for i in file.query_results["imports"].split("\n")
                    if i != ""
                ],
            },
            symbols = [aspect.Symbol(
                id = "/".join([ctx.rel, file.path.removesuffix(".r")]) if ctx.rel else file.path.removesuffix(".r"),
                provider = "r",
            )],
        )

aspect.register_configure_extension(
    id = "raw-test",
    prepare = prepare,
    declare = declare,
)
