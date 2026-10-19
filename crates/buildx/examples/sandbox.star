# Run from a workspace with a MODULE.bazel, a `sibling.txt` source file and a
# BUILD.bazel holding:
#
#   genrule(name = "ok", outs = ["ok.txt"], cmd = "echo ok > $@")
#   genrule(name = "peek", outs = ["peek.txt"], cmd = "cat sibling.txt > $@")
#   genrule(name = "peek_no_sandbox", outs = ["peek_no_sandbox.txt"], cmd = "cat sibling.txt > $@", tags = ["no-sandbox"])
#
# `//:peek` reads a file it does not declare. Under `sandbox()`, and under the
# default executor (which is the sandbox), only declared inputs exist in the
# action's directory, so it fails as an ordinary action failure and the other
# targets still build. Under `local()` it succeeds, and so would a `cached`
# store's upload of it, under a key that omits `sibling.txt`. `//:peek_no_sandbox`
# runs unsandboxed under all three, as in Bazel (tags `no-sandbox`, `local`).
#
# Outputs are built once per output base, so run it in a fresh workspace for
# each executor to see each result.

def _build(ctx, name, executor):
    results = ctx.build(["//:ok", "//:peek", "//:peek_no_sandbox"], executor = executor)
    for label, r in results.items():
        failures = r.failures()
        ctx.output.print(name, label, "FAILED" if failures else "ok")

def _impl(ctx: Context):
    _build(ctx, "sandbox", ctx.execution.sandbox())

main = buildx_main(impl = _impl, cli_args = {})
