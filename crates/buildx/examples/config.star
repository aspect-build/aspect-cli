# Run from the fixture workspace (any workspace with a MODULE.bazel works;
# the `starlark` setting needs a build setting target, here the fixture's
# `//:mode` string_flag from defs.bzl).

def _impl(ctx: Context):
    default = ctx.config()
    opt = ctx.config(
        compilation_mode = "opt",        # enum
        stamp = True,                    # bool
        copt = ["-O3", "-fPIC"],         # repeated list
        action_env = {"A": "b"},         # dict
        platforms = ["//:linux"],        # list of labels
        cpu = "k8",                      # opaque: Bazel's own converter parses it
        check_visibility = False,        # bool, spelled positively
    )
    same = ctx.config(
        check_visibility = False,
        cpu = "k8",
        platforms = ["//:linux"],
        action_env = {"A": "b"},
        copt = ["-O3", "-fPIC"],
        stamp = True,
        compilation_mode = "opt",
    )

    # The engine half (Starlark semantics, loading, repositories) is validated and
    # carried on the Config; it is not applied beyond the defaults yet.
    semantics = ctx.config(incompatible_disallow_empty_glob = True, lockfile_mode = "error")
    flagged = ctx.config(starlark = {"//:mode": "fast"})

    ctx.output.print("type:", type(opt))
    ctx.output.print("opt:", opt)
    ctx.output.print("opt.settings:", opt.settings)
    ctx.output.print("opt == same (order differs):", opt == same)
    ctx.output.print("opt == default:", opt == default)
    ctx.output.print("same checksum:", opt.checksum == same.checksum)
    ctx.output.print("engine half: == default is", semantics == default, "; carried:", semantics)
    ctx.output.print("starlark setting:", flagged, flagged == default)
    ctx.output.print("dict key:", {opt: 1}[same])

    d = default.describe("compilation_mode")
    ctx.output.print("describe:", d.name, d.kind, "default", d.default, d.effect_tags)
    ctx.output.print("  ", d.help)

main = buildx_main(impl = _impl, cli_args = {})
