aspect.register_rule_kind("rust_library", {
    "From": "@deps-test//my:rules.bzl",
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def declare(ctx):
    for file in ctx.sources:
        ctx.targets.add(
            name = file.path[:file.path.rindex(".")] + "_lib",
            kind = "rust_library",
            attrs = {
                "srcs": [file.path],
            },
        )

aspect.register_configure_extension(
    id = "rust-test",
    prepare = lambda _: aspect.PrepareResult(
        # All source files to be processed
        sources = aspect.SourceExtensions(".rs"),
        queries = {
            # A query treated as an array of results
            "imports": aspect.AstQuery(
                grammar = "rust",
                filter = "*.rs",
                query = """
                    (use_declaration
                        (scoped_identifier
                        (identifier) @root_crate
                        (identifier) @module)
                    )
                """,
            ),
        },
    ),
    declare = declare,
)
