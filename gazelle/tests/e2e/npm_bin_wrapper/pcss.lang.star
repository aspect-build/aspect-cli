aspect.register_rule_kind("postcss", {
    "From": "//:pcss.bzl",
    "NonEmptyAttrs": ["config", "srcs"],
    "MergeableAttrs": ["srcs"],
})

def prepare(_):
    return aspect.PrepareResult(
        sources = [
            aspect.SourceFiles("postcss.config.mjs"),
            aspect.SourceExtensions(".css"),
        ],
    )

def declare(ctx):
    # TODO(PR-7062): allow grouping sources to avoid this filtering loop
    config = [c for c in ctx.sources if c.path.endswith("postcss.config.mjs")]
    if not config:
        ctx.targets.remove("css", kind = "postcss")
        return

    # A rule of a custom kind that generates js provider symbols
    ctx.targets.add(
        name = "css",
        kind = "postcss",
        attrs = {
            "config": aspect.Label(name = "postcss_config"),
            "srcs": [s for s in ctx.sources if s.path.endswith(".css")],
        },
    )

aspect.register_configure_extension(
    id = "postcss",
    prepare = prepare,
    declare = declare,
)
