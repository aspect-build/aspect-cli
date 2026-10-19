# Run from a workspace with a MODULE.bazel, a `tools/status.sh` that prints
# `STABLE_GIT_COMMIT <sha>` and `BUILD_SCM_STATUS <state>`, and targets
# `//:stamped` (a genrule with `stamp = 1` reading `bazel-out/stable-status.txt`
# and `volatile-status.txt`), `//:starlark_stamped` (a rule reading
# `ctx.info_file` and `ctx.version_file`) and `//:plain` (no stamping).
#
# The workspace status action is Bazel's own: it runs `tools/status.sh` through
# `sh -c` in the workspace on every build, writes the stable and volatile
# status files, and posts the `workspace_status` event. As for Bazel, the
# command is run by `sh -c` with the workspace as its directory and the path
# is normalized, so a script in the workspace root is spelled `tools/x.sh` or
# an absolute path (`./x.sh` becomes `x.sh`, a command looked up in PATH).

def _impl(ctx: Context):
    cfg = ctx.config(
        stamp = True,
        workspace_status_command = "tools/status.sh",
        embed_label = "release-1",
    )
    results = ctx.build(
        ["//:stamped", "//:starlark_stamped", "//:plain"],
        config = cfg,
        events = [ctx.events.file("stamp.bep")],
    )
    for label, r in results.items():
        for f in r.failures():
            ctx.output.print(label, "FAILED:", f.message)
        for a in r.artifacts():
            ctx.output.print(label, a.path, a.digest)

main = buildx_main(impl = _impl, cli_args = {})
