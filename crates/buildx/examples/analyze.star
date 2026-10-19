# Run from the fixture workspace: a MODULE.bazel with `//:all` and `//lib:lib`
# filegroups, a `//:hello` genrule, and a `defs.bzl` whose `write_file`,
# `run_env` (DefaultInfo executable and RunEnvironmentInfo) and `tree` rules
# define `//:written`, `//:runner` and `//:dir`.

def _impl(ctx: Context):
    analysis = ctx.analysis()
    p = analysis.providers
    opt = ctx.config(compilation_mode = "opt")

    targets = analysis.analyze(["//:hello", "//:written", "//:runner", "//:dir"], config = opt)
    for label, t in targets.items():  # dict[Label, target or Failure]
        info = t[p.DefaultInfo]
        ctx.output.print(t.label, "files:", [f.short_path for f in info.files.to_list()])
        ctx.output.print("  files_to_run.executable:", info.files_to_run.executable)
        ctx.output.print("  output groups:", sorted(t.output_groups))
        if p.RunEnvironmentInfo in t:
            env = t[p.RunEnvironmentInfo]
            ctx.output.print("  run env:", env.environment, env.inherited_environment)

    # A whole query result, in the default configuration.
    all = analysis.analyze(analysis.query("//lib:all"))
    ctx.output.print("analyzed", len(all), "from a query result:", all.keys())

main = buildx_main(impl = _impl, cli_args = {})
