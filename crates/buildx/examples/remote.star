# Run from the fixture workspace with the fake REAPI server running:
#
#   cargo run -p buildx --example fake_remote -- --port 17890
#
# Genrules are routed to the remote executor: their inputs are uploaded to the
# server's CAS, `Execute` runs them there, and the outputs are downloaded into
# the execution root, where the build result's artifacts point at them.

def _route(spawn):
    if spawn.mnemonic == "Genrule":
        print("remote:", spawn.mnemonic, spawn.outputs[0].split("/")[-1], "action", spawn.action_digest[:16] + "...", "input root", spawn.action.input_root_digest.hash[:12], "entries", len(spawn.input_tree.root.files) + len(spawn.input_tree.root.directories))
        return spawn.executors["remote"]
    return spawn.executors["local"]

def _impl(ctx: Context):
    e = ctx.execution
    store = e.store("grpc://127.0.0.1:17890")
    remote = e.remote("grpc://127.0.0.1:17890", store = store, name = "fake")
    routed = e.route(_route, executors = {"remote": remote, "local": e.local()})
    for label, r in ctx.build(["//:sentence"], executor = routed).items():
        for a in r.artifacts():
            ctx.output.print(label, a.path.split("/")[-1], "size", a.size, "digest", a.digest, "is_local", a.is_local)
        for f in r.failures():
            ctx.output.print(label, "FAILED", f.message)

main = buildx_main(impl = _impl, cli_args = {})
