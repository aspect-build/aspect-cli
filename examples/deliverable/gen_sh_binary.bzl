"""A custom rule that emits a .sh script as its executable without a runfiles tree.

Used to verify that delivery handles targets that have no .runfiles_manifest in
BES output — a valid pattern for scripts that don't use rlocation.
"""

def _gen_sh_binary_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(out, ctx.attr.content, is_executable = True)
    return [DefaultInfo(executable = out)]

gen_sh_binary = rule(
    implementation = _gen_sh_binary_impl,
    executable = True,
    attrs = {"content": attr.string()},
)
