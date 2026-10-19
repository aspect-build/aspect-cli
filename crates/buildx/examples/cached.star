# Run from the fixture workspace (its `//:stamp` genrule writes the current
# time in nanoseconds) with the fake REAPI server running:
#
#   cargo run -p buildx --example fake_remote -- --port 17890
#
# `cached(store, local())` looks each action up in the server's action cache by
# its REAPI Action digest. A miss runs the action with Bazel's local strategy and
# stores the outputs and the result; a hit downloads the outputs and does not run
# the action, so `stamp.txt` keeps the time of the run that stored it.
#
# The action environment is part of the Action digest: config A and config B are
# two different actions for `//:stamp`, and building A again after B is an action
# Bazel must re-run, which the cache answers.

def _build(ctx, label, cfg, executor):
    for l, r in ctx.build(["//:stamp"], config = cfg, executor = executor).items():
        for a in r.artifacts():
            ctx.output.print(label, "stamp.txt digest", a.digest[:16])
        for f in r.failures():
            ctx.output.print(label, "FAILED", f.message)

def _impl(ctx: Context):
    e = ctx.execution
    store = e.store("grpc://127.0.0.1:17890")
    cached = e.cached(store, e.local())
    a = ctx.config(action_env = {"VARIANT": "a"})
    b = ctx.config(action_env = {"VARIANT": "b"})
    _build(ctx, "A, first run (miss)  ", a, cached)
    _build(ctx, "B, first run (miss)  ", b, cached)
    _build(ctx, "A again (cache hit)  ", a, cached)

main = buildx_main(impl = _impl, cli_args = {})
