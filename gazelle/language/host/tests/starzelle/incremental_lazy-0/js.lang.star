aspect.register_rule_kind("js_library", {
    "From": "@aspect_rules_js//js:defs.bzl",
    "NonEmptyAttrs": ["srcs"],
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def prepare(_):
    return aspect.PrepareResult(
        # All source files to be processed
        sources = [
            aspect.SourceExtensions(".js"),
        ],
        queries = {
            # A query treated as an array of results
            "imports": aspect.RegexQuery(
                filter = "*.js",
                expression = """import\\s+'(?P<import>[^']+)'""",
            ),
        },
    )

def declare(ctx):
    for file in ctx.sources:
        ctx.targets.add(
            name = file.path[:file.path.rindex(".")] + "_lib",
            kind = "js_library",
            attrs = {
                "srcs": [file.path],
                "deps": [
                    aspect.Import(
                        id = i.captures["import"],
                        provider = "js",
                        src = file.path,
                    )
                    for i in file.query_results["imports"]
                ],
            },
            symbols = [aspect.Symbol(
                id = "/".join([ctx.rel, file.path]) if ctx.rel else file.path,
                provider = "js",
            )],
        )

aspect.register_configure_extension(
    id = "dummy-js",
    prepare = prepare,
    declare = declare,
)
