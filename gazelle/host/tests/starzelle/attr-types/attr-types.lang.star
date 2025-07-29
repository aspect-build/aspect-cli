aspect.register_rule_kind("x_lib", {
    "From": "@deps-test//my:rules.bzl",
    "ResolveAttrs": ["deps"],
})

def declare(ctx):
    ctx.targets.add(
        name = "a",
        kind = "x_lib",
        attrs = {
            # Basic key/value labels
            "str": "bar",
            "strs": ["bar", "baz"],
            "i": 42,
            "is": [42],
            "n": None,

            # Labels
            "l": aspect.Label(
                repo = "r",
                pkg = "p",
                name = "l",
            ),
            "l2": aspect.Label(
                pkg = ctx.rel,
                name = "local",
            ),

            # Various label types within a resolved attribute
            "deps": [
                # Imports to be resolved
                aspect.Import(
                    id = "b",
                    provider = "x",
                ),

                # Labels
                aspect.Label(
                    repo = "r",
                    pkg = "p",
                    name = "l",
                ),
                aspect.Label(
                    pkg = ctx.rel,
                    name = "rel",
                ),
                aspect.Label(
                    pkg = "default",
                    name = "default",
                ),

                # Strings (that happen to look like labels)
                ":value",
                "//%s:value" % ctx.rel,
            ],
            "single_dep": aspect.Import(id = "b", provider = "x"),
            "single_optional_dep": aspect.Import(id = "not-found", provider = "x", optional = True),

            # Various label types within a plain attribute
            "types2": [
                aspect.Label(
                    pkg = "p2",
                    name = "l2",
                ),
                aspect.Label(
                    pkg = ctx.rel,
                    name = "rel",
                ),
                aspect.Label(
                    pkg = "default",
                    name = "default",
                ),
                aspect.Label(
                    repo = "foo",
                    pkg = "repo-default",
                    name = "repo-default",
                ),

                # Strings (that happen to look like labels)
                ":value",
                "//%s:value" % ctx.rel,
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
            id = ctx.rel + "b",
            provider = "x",
        )],
    )

aspect.register_configure_extension(
    id = "attribute-types-test",
    declare = declare,
)
