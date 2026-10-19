# Run from the fixture workspace: a MODULE.bazel with a `//:hello` genrule, a
# `//:sentence` genrule over `//:written` and `//:hello`, `//:dir` (a tree
# artifact), `//:runner`, `//:all`, and `//:broken` (an action that exits 3);
# defs.bzl defines `write_file`, `tree`, `run_env` and `fail_action`.

def _impl(ctx: Context):
    local = ctx.execution.local()
    results = ctx.build(["//:sentence", "//:dir", "//:all", "//:broken"], executor = local)
    for label, r in results.items():
        failures = r.failures()
        if failures:
            ctx.output.print(label, "FAILED:", [f.message for f in failures])
            continue
        for a in r.artifacts():
            ctx.output.print(label, a.path, "size", a.size, "digest", a.digest, "local", a.is_local, "tree", a.is_tree)

    for t in ctx.analysis().query("//:dir"):
        ctx.output.print("ensure //:dir:", [p.split("/")[-1] for p in ctx.outputs.ensure(results[t.label].artifacts())])
    ctx.output.print("ensure (all results):", len(ctx.outputs.ensure(results)), "files")

    # A query after a build in the same process.
    ctx.output.print("query after build:", len(ctx.analysis().query("//...")))

main = buildx_main(impl = _impl, cli_args = {})
