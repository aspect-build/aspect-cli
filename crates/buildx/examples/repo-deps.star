# Run from this repository's root: module extensions (rules_rs) and external
# repositories are resolved and fetched as `bazel query` would.

QUERIES = [
    "//...",
    "deps(//:all)",
    "deps(//crates/buildx:buildx)",
    "rdeps(//crates/..., //crates/blazon-rt:blazon-rt)",
    "allpaths(//crates/buildx:buildx, //crates/blazon-rt:blazon-rt)",
    "buildfiles(//crates/buildx:buildx)",
    "loadfiles(//crates/buildx:buildx)",
]

def _impl(ctx):
    for q in QUERIES:
        ctx.output.print(len(ctx.analysis().query(q)), q)

main = buildx_main(impl = _impl, cli_args = {})
