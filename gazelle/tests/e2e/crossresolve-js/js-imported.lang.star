aspect.register_rule_kind("x_lib", {
    "From": "@deps-test//my:rules.bzl",
})

def declare(ctx):
    # A rule of a custom kind that generates js provider symbols
    ctx.targets.add(
        name = "a",
        kind = "x_lib",
        symbols = [
            aspect.Symbol(
                id = path.join(ctx.rel, "generated"),
                provider = "js",
            ),
        ],
    )

    # A rule type known by the js gazelle plugin which may find the js provider symbols.
    # The file may not exist on disk, maybe its generated elsewhere, but the js gazelle plugin
    # should understand that it is a js_library target with files that can be imported.
    ctx.targets.add(
        name = "b",
        kind = "js_library",
        attrs = {
            "srcs": [
                "file-served-by-js_library.js",
            ],
        },
    )

aspect.register_configure_extension(
    id = "js-imports-test",
    declare = declare,
)
