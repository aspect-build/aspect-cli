# Run from the fixture workspace (a MODULE.bazel with a `//:hello` genrule and a
# failing `//:broken`):
#
#   buildx examples/bes-file.star:main
#
# The build's Build Event Protocol stream is written to `build.bep`, delimited
# `BuildEvent` messages as `--build_event_binary_file` writes them. Read it with
#
#   cargo run -p buildx --example bep_dump -- build.bep
#
# which prints the event kinds, whether every announced event arrived, and the
# exit code the stream states. `--events` lists ids and children, `--kind
# named_set` prints the payloads of one kind.

def _impl(ctx: Context):
    results = ctx.build(
        ["//:hello", "//:broken"],
        events = [ctx.events.file("build.bep")],
    )
    for label, r in results.items():
        ctx.output.print(label, "FAILED" if r.failures() else "ok")

main = buildx_main(impl = _impl, cli_args = {})
