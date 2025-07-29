BZL_LIBRARY = "bzl_library"

LANG_NAME = "starlark"
BZL_EXT = ".bzl"

aspect.register_rule_kind(BZL_LIBRARY, {
    "From": "@bazel_skylib//:bzl_library.bzl",
    "NonEmptyAttrs": ["srcs"],
    "MergeableAttrs": ["srcs"],
    "ResolveAttrs": ["deps"],
})

def prepare(_):
    return aspect.PrepareResult(
        sources = aspect.SourceExtensions(".bzl"),
        queries = {
            "loads": aspect.AstQuery(
                grammar = "starlark",
                query = """(module
                    (expression_statement
                        (call 
                            function: (identifier) @id
                            arguments: (argument_list
                                (string) @path
                                (string)
                            )
                        )
                        (#eq? @id "load")
                    )
                )""",
            ),
        },
    )

def declare_targets(ctx):
    # TODO
    # Loop through the existing bzl_library targets in this package and
    # delete any that are no longer needed.
    for file in ctx.sources:
        label = file.path.removesuffix(".bzl").replace("/", "_")
        file_pkg = path.dirname(file.path)

        loads = [ld.captures["path"].strip("\"") for ld in file.query_results["loads"]]
        loads = [ld.removeprefix("//").replace(":", "/") if ld.startswith("//") else path.join(file_pkg, ld.removeprefix(":")) for ld in loads]
        loads = [ld.strip("/") for ld in loads]

        ctx.targets.add(
            kind = BZL_LIBRARY,
            name = label,
            attrs = {
                "srcs": [file.path],
                "visibility": [checkInternalVisibility(ctx.rel, "//visibility:public")],
                "deps": [
                    aspect.Import(
                        id = ld,
                        src = file.path,
                        provider = LANG_NAME,
                    )
                    for ld in loads
                ] if len(loads) > 0 else None,
            },
            # TODO
            # load("@bazel_tools//tools/build_defs/repo:http.bzl")
            # Note that the Go extension has a special case for it:
            # if impLabel.Repo == "bazel_tools" {
            # // The @bazel_tools repo is tricky because it is a part of the "shipped
            # // with bazel" core library for interacting with the outside world.
            symbols = [
                aspect.Symbol(
                    id = path.join(ctx.rel, file.path),
                    provider = LANG_NAME,
                ),
            ],
        )
    return {}

aspect.register_configure_extension(
    id = LANG_NAME,
    properties = {},
    prepare = prepare,
    declare = declare_targets,
)

# See https://github.com/bazelbuild/bazel-skylib/blob/1.7.1/gazelle/bzl/gazelle.go#L340
def checkInternalVisibility(rel, visibility):
    i = rel.find("internal")
    if i > 0:
        return "//%s:__subpackages__" % rel[:i - 1]
    elif i == 0:
        return "//:__subpackages__"

    i = rel.find("private")
    if i > 0:
        return "//%s:__subpackages__" % rel[:i - 1]
    elif i == 0:
        return "//:__subpackages__"

    return visibility
