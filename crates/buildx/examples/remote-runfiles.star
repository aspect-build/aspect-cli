# Run from the fixture workspace with the fake REAPI server running (see
# remote.star). `//:used` runs `//:tool`, an executable whose runfiles hold
# `data.txt`, as a tool: the remote input root carries the runfiles in their
# `.runfiles` layout, so the tool finds its data next to itself on the server.

def _route(spawn):
    if spawn.mnemonic == "Action":
        root = spawn.input_tree
        print("remote: argv", spawn.arguments[-1].split("&&")[-1][-90:])
        files = [f.name for c in root.children for f in c.files]
        print("  input root directories:", len(root.root.directories), "child directories:", len(root.children), "files in children:", sorted(files))
        return spawn.executors["remote"]
    return spawn.executors["local"]

def _impl(ctx: Context):
    e = ctx.execution
    remote = e.remote("grpc://127.0.0.1:17890")
    routed = e.route(_route, executors = {"remote": remote, "local": e.local()})
    for label, r in ctx.build(["//:used"], executor = routed).items():
        for a in r.artifacts():
            ctx.output.print(label, a.path.split("/")[-1], "size", a.size, "digest", a.digest[:16])
        for f in r.failures():
            ctx.output.print(label, "FAILED", f.message)

main = buildx_main(impl = _impl, cli_args = {})
