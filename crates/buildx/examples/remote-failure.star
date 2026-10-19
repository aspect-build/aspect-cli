# Run from the fixture workspace with the fake REAPI server running (see
# remote.star). A remote or cached executor that fails fails that action the way
# a local failure does, with Bazel's own exception: the build result shows the
# message, and the rest of the build continues.

def _impl(ctx: Context):
    e = ctx.execution
    store = e.store("grpc://127.0.0.1:17890")
    remote = e.remote("grpc://127.0.0.1:17890", store = store)
    cached_local = e.cached(store, e.local())
    dead = e.remote("grpc://127.0.0.1:1")

    cases = [
        ("non-zero exit on the remote", remote, ["//:broken", "//:hello"], "A"),
        ("non-zero exit under cached(local)", cached_local, ["//:broken", "//:hello"], "B"),
        ("transport error", dead, ["//:broken", "//:hello"], "C"),
    ]
    for name, executor, targets, round in cases:
        cfg = ctx.config(action_env = {"ROUND": round})
        ctx.output.print("==", name)
        for label, r in ctx.build(targets, config = cfg, executor = executor).items():
            ctx.output.print("  ", label, "built" if not r.failures() else "FAILED", [a.path.split("/")[-1] for a in r.artifacts()], [f.message.split("\n")[0][:170] for f in r.failures()])

main = buildx_main(impl = _impl, cli_args = {})
