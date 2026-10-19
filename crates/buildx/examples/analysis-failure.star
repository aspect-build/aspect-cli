# Run from the fixture workspace: defs.bzl's `fail_analysis` and `needs` rules
# define `//:analysis_fail` (calls fail()), `//:missing_dep` (depends on a
# target that does not exist), `//:private_dep` (depends on a target `//lib`
# does not make visible) and `//:good`.
#
# A target that fails to analyze is data, not an exception: `analyze` maps its
# label to a `Failure`, and `build` gives it a `BuildResult` whose `failures()`
# is populated while the other targets build.

def _impl(ctx: Context):
    names = ["//:good", "//:analysis_fail", "//:missing_dep", "//:private_dep", "//:hello"]

    analyzed = ctx.analysis().analyze(names)
    for label, r in analyzed.items():
        if type(r) == "Failure":
            ctx.output.print("analyze", label, "FAILED:", r.message)
        else:
            ctx.output.print("analyze", label, "ok")

    for label, r in ctx.build(names).items():
        failures = r.failures()
        ctx.output.print("build", label, "FAILED: " + failures[0].message if failures else "built " + str([a.path.split("/")[-1] for a in r.artifacts()]))

main = buildx_main(impl = _impl, cli_args = {})
