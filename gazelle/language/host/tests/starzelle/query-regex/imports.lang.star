aspect.register_rule_kind("x_lib", {
    "From": "@deps-test//my:rules.bzl",
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def prepare(_):
    return aspect.PrepareResult(
        # All source files to be processed
        sources = [
            aspect.SourceExtensions(".x"),
        ],
        queries = {
            # A query treated as an array of results
            "imports": aspect.RegexQuery(
                filter = "*.x",
                expression = """import\\s+"(?P<import>[^"]+)\"""",
            ),
            # A query treated as a singleton which may have 0 results
            "is_test": aspect.RegexQuery(
                filter = "*.x",
                expression = """testonly:\\s*(?P<test_flag>true|false)""",
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
                "testonly": file.query_results["is_test"][0].captures["test_flag"] == "true" if len(file.query_results["is_test"]) else None,
                "deps": [
                    aspect.Import(
                        id = i.captures["import"],
                        provider = "x",
                        src = file.path,
                    )
                    for i in file.query_results["imports"]
                ],
            },
            symbols = [aspect.Symbol(
                id = "/".join([ctx.rel, file.path.removesuffix(".x")]) if ctx.rel else file.path.removesuffix(".x"),
                provider = "x",
            )],
        )

aspect.register_configure_extension(
    id = "re-test",
    prepare = prepare,
    declare = declare,
)
