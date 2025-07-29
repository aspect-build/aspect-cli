aspect.register_rule_kind("x_lib", {
    "From": "@deps-test//my:rules.bzl",
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def prepare(_):
    return aspect.PrepareResult(
        # All source files to be processed
        sources = aspect.SourceExtensions(".yaml"),
        queries = {
            # A query treated as an array of results
            "imports": aspect.YamlQuery(
                filter = "*.yaml",
                query = ".imports[]?",
            ),
            # A query treated as a singleton which may have 0 results
            "is_test": aspect.YamlQuery(
                filter = "*.yaml",
                query = """.testonly?""",
            ),
        },
    )

def declare(ctx):
    for file in ctx.sources:
        ctx.targets.add(
            name = file.path[:file.path.rindex(".")] + "_lib",
            kind = "x_lib",
            attrs = {
                "srcs": [file.path],
                "testonly": file.query_results["is_test"][0] if len(file.query_results["is_test"]) else None,
                "deps": [
                    aspect.Import(
                        id = i,
                        provider = "x",
                        src = file.path,
                    )
                    for i in file.query_results["imports"]
                    if i
                ],
            },
            symbols = [aspect.Symbol(
                id = "/".join([ctx.rel, file.path.removesuffix(".yaml")]) if ctx.rel else file.path.removesuffix(".yaml"),
                provider = "x",
            )],
        )

aspect.register_configure_extension(
    id = "yamlq-test",
    prepare = prepare,
    declare = declare,
)
