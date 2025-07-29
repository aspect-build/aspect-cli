# https://aspect-build.slack.com/archives/C0549H229MH/p1738340956408799?thread_ts=1738192716.619419&cid=C0549H229MH

"""
Aspect CLI plugin for generating `vitest_test()` targets.

See: https://docs.aspect.build/cli/starlark
"""

VITEST_CONFIG_FILE = "vitest.config.mjs"

aspect.register_rule_kind("vitest_test", {
    "From": "//bazel:defs.bzl",
    "NonEmptyAttrs": ["config"],
    "ResolveAttrs": ["config", "data"],
})

def prepare(_ctx):
    return aspect.PrepareResult(
        sources = [
            # It would be cleaner to use a glob to match the corresponding `gazelle:js_test_files` directive,
            # but according to the Aspect CLI plugin docs globs are significantly slower
            aspect.SourceExtensions(".test.ts", ".test.tsx", ".test.mts", ".test.cts", ".test.js", ".test.jsx", ".test.mjs", ".test.cjs"),
            aspect.SourceExtensions(".spec.ts", ".spec.tsx", ".spec.mts", ".spec.cts", ".spec.js", ".spec.jsx", ".spec.mjs", ".spec.cjs"),
        ],
    )

def declare(ctx):
    test_srcs = []
    for src in ctx.sources:
        test_srcs.append(src)

    if len(test_srcs) == 0:
        ctx.targets.remove("vitest")
        return

    package_name = path.base(ctx.rel)
    ctx.targets.add(
        name = "vitest",
        kind = "vitest_test",
        attrs = {
            "config": aspect.Import(
                id = path.join(package_name, VITEST_CONFIG_FILE),
                provider = "js",
                optional = True,
            ),
            "data": [
                aspect.Import(
                    id = path.join(package_name, src.path.replace(".ts", "")),
                    provider = "js",
                )
                for src in test_srcs
            ],
        },
    )

aspect.register_configure_extension(
    id = "vitest_test",
    prepare = prepare,
    declare = declare,
)
