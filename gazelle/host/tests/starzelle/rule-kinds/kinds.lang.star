aspect.register_rule_kind("my_build", {
    "From": "@builder//:a.bzl",
    "NonEmptyAttrs": ["x"],
})

aspect.register_rule_kind("my_test", {
    "From": "@tester//:b.bzl",
    "NonEmptyAttrs": ["y"],
})

aspect.register_rule_kind("should never be used", {
    "From": "@--invalid --label",
})

def declare(ctx):
    ctx.targets.add(
        name = "b",
        kind = "my_build",
        attrs = {
            "x": False,
        },
    )
    ctx.targets.add(
        name = "t",
        kind = "my_test",
        attrs = {
            "y": True,
        },
    )

aspect.register_configure_extension(
    id = "gen",
    declare = declare,
)
