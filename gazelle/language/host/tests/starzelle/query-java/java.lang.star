aspect.register_rule_kind("java_library", {
    "From": "@deps-test//my:rules.bzl",
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def declare(ctx):
    for file in ctx.sources:
        ctx.targets.add(
            name = file.path[:file.path.rindex(".")] + "_lib",
            kind = "java_library",
            attrs = {
                "srcs": [file.path],
                # This ought to be aspect.Import's appearing in a deps attribute, but we haven't wired up the maven language here
                # As a first step, just prove that we parsed the import statement properly and tree-sitter is working.
                # TODO(https://github.com/aspect-build/aspect-cli/issues/797) continue porting the real logic for this extension.
                "tags": ["imports-{}".format(i.captures["imp"]) for i in file.query_results["imports"]],
            },
        )

aspect.register_configure_extension(
    id = "java-test",
    prepare = lambda _: aspect.PrepareResult(
        # All source files to be processed
        sources = aspect.SourceExtensions(".java"),
        queries = {
            # A query treated as an array of results
            "imports": aspect.AstQuery(
                grammar = "java",
                filter = "*.java",
                query = "(import_declaration (scoped_identifier) @imp)",
            ),
        },
    ),
    declare = declare,
)
