# Run from the fixture workspace; `//:legacy` is a `legacy` rule (defs.bzl) that
# reads `dep.files`, which the Starlark-semantics setting
# `incompatible_disable_target_default_provider_fields` turns into an error.
#
# Two configs differing in a Starlark-semantics setting give different results
# in one process: the setting reaches `sync`, Skyframe re-loads and re-analyzes
# what depended on it, and a call with an equal config reuses everything.

def _impl(ctx: Context):
    default = ctx.config()
    legacy_off = ctx.config(incompatible_disable_target_default_provider_fields = True)
    for name, cfg in [("default", default), ("fields disabled", legacy_off), ("default again", default), ("equal to disabled", ctx.config(incompatible_disable_target_default_provider_fields = True))]:
        r = ctx.analysis().analyze("//:legacy", config = cfg)
        for label, t in r.items():
            if type(t) == "Failure":
                ctx.output.print(name, "-> FAILED:", [l for l in t.message.split("\n") if l.startswith("Error")][0])
            else:
                ctx.output.print(name, "-> analyzed")

main = buildx_main(impl = _impl, cli_args = {})
