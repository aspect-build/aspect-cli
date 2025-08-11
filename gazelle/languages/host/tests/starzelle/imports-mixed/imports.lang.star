aspect.register_rule_kind("x_lib", {
    "From": "@deps-test//my:rules.bzl",
    "ResolveAttrs": ["deps"],
})

def declare(ctx):
    ctx.targets.add(
        name = "a",
        kind = "x_lib",
        attrs = {
            "foo": "bar",
            "deps": [
                aspect.Import(
                    id = "b",
                    provider = "x",
                ),
                "@predefined//:value",
            ],
        },
    )
    ctx.targets.add(
        name = "b",
        kind = "x_lib",
        attrs = {
            "foo": "baz",
        },
        symbols = [aspect.Symbol(
            id = "b",
            provider = "x",
        )],
    )

aspect.register_configure_extension(
    id = "mixed-import-and-values-test",
    declare = declare,
)
