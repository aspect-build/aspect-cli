aspect.register_rule_kind("x_lib", {
    "From": "@deps-test//my:rules.bzl",
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def prepare(_):
    return aspect.PrepareResult(
        sources = [
            aspect.SourceExtensions(".js"),
            aspect.SourceExtensions(".json"),
        ],
        queries = {
            "imports": aspect.AstQuery(
                grammar = "typescript",
                filter = "**/*.js",
                query = "(import_statement (string (string_fragment) @imp))",
            ),
        },
    )

def declare(ctx):
    for file in ctx.sources:
        ctx.targets.add(
            name = path.base(file.path[:file.path.rindex(".")]) + "_lib",
            kind = "x_lib",
            attrs = {
                "srcs": [file.path],
                "deps": [
                    aspect.Import(
                        id = i.captures["imp"],
                        provider = "x",
                        src = file.path,
                    )
                    for i in file.query_results["imports"]
                ],
            },
            symbols = [aspect.Symbol(
                id = "/".join([ctx.rel, file.path[:file.path.rindex(".")]]) if ctx.rel else file.path[:file.path.rindex(".")],
                provider = "x",
            )],
        )

aspect.register_configure_extension(
    id = "jsq-test",
    prepare = prepare,
    declare = declare,
)
