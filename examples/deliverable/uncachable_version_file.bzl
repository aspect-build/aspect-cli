"""Mirrors rules_python's uncachable_version_file pattern.

The action consumes `ctx.version_file` (the volatile workspace status file)
and tags itself `no-cache` so neither the local action cache nor the remote
cache retains the result. Each build re-executes the action and emits a
fresh stamped output. Used by delivery as a fixture to exercise the
INTENTIONAL bucket in phase-2 diagnostics.
"""

def _impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.run_shell(
        outputs = [out],
        inputs = [ctx.version_file],
        command = "cat $1 > $2",
        arguments = [ctx.version_file.path, out.path],
        mnemonic = "UncachableVersionFile",
        execution_requirements = {"no-cache": ""},
    )
    return [DefaultInfo(files = depset([out]))]

uncachable_version_file = rule(
    implementation = _impl,
)
