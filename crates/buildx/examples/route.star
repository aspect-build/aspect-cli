# Run from the fixture workspace: `//:sentence` (two genrules), `//:dir` and
# `//:written` (a run_shell action and a file write).
#
# Every spawn is routed by a function: it receives a `Spawn` and returns one of
# the executors given to `route`. Routing functions run on Bazel's threads, one
# evaluator per call, so they must be top-level functions of the script; they
# can `print`, and a failing one fails that action with its message.

def _route(spawn):
    print("saw", spawn.mnemonic, "argv0 =", spawn.arguments[0], "outputs =", [o.split("/")[-1] for o in spawn.outputs], "tags =", spawn.execution_info, "env =", sorted(spawn.environment.keys()))
    if spawn.mnemonic == "Genrule":
        return spawn.executors["genrules"]
    return spawn.executors["everything_else"]

def _refuse_genrules(spawn):
    if spawn.mnemonic == "Genrule":
        fail("genrules are not allowed here: " + spawn.outputs[0].split("/")[-1])
    return spawn.executors["local"]

def _inspect(spawn):
    # The REAPI messages, built on first use.
    action = spawn.action
    print(spawn.mnemonic, "action", action.command_digest.hash[:12], "input root", action.input_root_digest.hash[:12], "files", len(spawn.input_tree.root.files), "dirs", len(spawn.input_tree.root.directories), "output_paths", spawn.command.output_paths[0].split("/")[-1])
    return spawn.executors["local"]

def _impl(ctx: Context):
    e = ctx.execution
    routed = e.route(_route, executors = {"genrules": e.local(name = "genrules"), "everything_else": e.local(name = "everything_else")})
    for label, r in ctx.build(["//:sentence", "//:dir"], executor = routed).items():
        ctx.output.print("routed", label, [a.path.split("/")[-1] for a in r.artifacts()], [f.message for f in r.failures()])

    # A different action environment makes the actions run again, so they are routed again.
    second = ctx.config(action_env = {"ROUND": "2"})
    refuse = e.route(_refuse_genrules, executors = {"local": e.local()})
    for label, r in ctx.build(["//:sentence", "//:hello", "//:written"], executor = refuse, config = second).items():
        ctx.output.print("refusing genrules", label, [a.path.split("/")[-1] for a in r.artifacts()], [[l for l in f.message.split("\n") if "not allowed" in l or "routing" in l][:1] for f in r.failures()])

    third = ctx.config(action_env = {"ROUND": "3"})
    for label, r in ctx.build(["//:sentence"], executor = e.route(_inspect, executors = {"local": e.local()}), config = third).items():
        ctx.output.print("inspected", label, [a.path.split("/")[-1] for a in r.artifacts()])

main = buildx_main(impl = _impl, cli_args = {})
