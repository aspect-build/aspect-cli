aspect.register_rule_kind("props_test", {
    "From": "@test//:x.bzl",
    "NonEmptyAttrs": ["s"],
    "MergeableAttrs": ["s", "sa", "b", "n"],
})

aspect.register_configure_extension(
    id = "property-types",
    properties = {
        "s": aspect.Property(
            type = "string",
        ),
        "s_defaults": aspect.Property(
            type = "string",
            default = "default-value",
        ),
        "sa": aspect.Property(
            type = "[]string",
        ),
        "sa_defaults": aspect.Property(
            type = "[]string",
            default = "1,2,3",
        ),
        "b": aspect.Property(
            type = "bool",
        ),
        "b_defaults": aspect.Property(
            type = "bool",
            default = True,
        ),
        "n": aspect.Property(
            type = "number",
        ),
        "n_defaults": aspect.Property(
            type = "number",
            default = 123,
        ),
    },
    prepare = lambda c: prep(c),
    declare = lambda c: decl(c),
)

def prep(ctx):
    return aspect.PrepareResult(sources = [])

def decl(ctx):
    ctx.targets.add(
        name = "no_defaults",
        kind = "props_test",
        attrs = {
            "s": ctx.properties["s"],
            "sa": ctx.properties["sa"],
            "b": ctx.properties["b"],
            "n": ctx.properties["n"],
        },
    )

    ctx.targets.add(
        name = "with_defaults",
        kind = "props_test",
        attrs = {
            "s": ctx.properties["s_defaults"],
            "sa": ctx.properties["sa_defaults"],
            "b": ctx.properties["b_defaults"],
            "n": ctx.properties["n_defaults"],
        },
    )
