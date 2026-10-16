# Run from this repository's root: queries into external repositories,
# including repositories created by a module extension (@crates) and Bazel's
# own (@bazel_tools), by apparent and canonical name.

QUERIES = [
    "@crates//:all",
    "@bazel_tools//tools/cpp:toolchain",
    "@@bazel_tools//tools/cpp:toolchain",
    "@bazel_tools//tools/build_defs/repo:all",
]

def _impl(ctx):
    for q in QUERIES:
        ctx.output.print(len(ctx.analysis().query(q)), q)

main = buildx_main(impl = _impl, cli_args = {})
