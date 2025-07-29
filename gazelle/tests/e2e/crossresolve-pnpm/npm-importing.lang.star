aspect.register_rule_kind("x_lib", {
    "From": "@deps-test//my:rules.bzl",
    "ResolveAttrs": ["deps"],
})

def declare(ctx):
    # A rule of a custom kind that generates js provider symbols
    ctx.targets.add(
        name = "a",
        kind = "x_lib",
        attrs = {
            "deps": [
                aspect.Import(
                    id = "jquery",
                    provider = "js",
                ),
            ],
        },
    )

aspect.register_configure_extension(
    id = "npm-imports-test",
    declare = declare,
)
