aspect.register_rule_kind("templ_library", {
    "From": "@deps-test//my:rules.bzl",
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def declare(ctx):
    for file in ctx.sources:
        ctx.targets.add(
            name = file.path[:file.path.rindex(".")] + "_lib",
            kind = "templ_library",
            attrs = {
                "srcs": [file.path],
                # This ought to be aspect.Import's appearing in a deps attribute, but we haven't wired up the go language here
                # As a first step, just prove that we parsed the import statement properly and tree-sitter is working.
                "tags": ["imports-{}".format(i.captures["import_path"].strip("\"")) for i in file.query_results["imports"]],
            },
        )

aspect.register_configure_extension(
    id = "templ-test",
    prepare = lambda _: aspect.PrepareResult(
        # All source files to be processed
        sources = aspect.SourceExtensions(".templ"),
        queries = {
            # A query treated as an array of results
            "imports": aspect.AstQuery(
                grammar = "go",
                filter = "*.templ",
                query = "(import_spec path: (interpreted_string_literal) @import_path)",
            ),
        },
    ),
    declare = declare,
)
