# Run from this repository's root: the targets of the buildx crate itself.

def _impl(ctx: Context):
    targets = ctx.analysis().query("//crates/buildx:*")
    for t in targets:
        kind = t.target_kind
        ctx.output.print(kind, t.label)
    ctx.output.print(len(targets), "targets")

main = buildx_main(impl = _impl, cli_args = {})
