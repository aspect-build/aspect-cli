# Run from the fixture workspace with both fake servers running:
#
#   cargo run -p buildx --example fake_remote -- --port 17890
#   cargo run -p buildx --example fake_bes -- --port 17891 --save received.bep
#
# By default every file the stream references is named by a local `file://` URI,
# with its digest and length, whether or not a remote cache is configured. With
# `artifacts = store` the Build Event Service stream names each file by a
# `bytestream://` URI instead, after uploading it to the store; `build.bep` (a
# file sink in the same build) keeps the `file://` URIs. Compare the named sets:
#
#   cargo run -p buildx --example bep_dump -- --kind named_set build.bep
#   cargo run -p buildx --example bep_dump -- --kind named_set received.bep
#
# A file that cannot be uploaded keeps its `file://` URI; the failure is shown in
# the stream's progress and on stderr and does not fail the build.

def _impl(ctx: Context):
    store = ctx.execution.store("grpc://127.0.0.1:17890", instance = "main")
    results = ctx.build(
        ["//:hello"],
        events = [
            ctx.events.file("build.bep"),
            ctx.events.bes("grpc://127.0.0.1:17891", artifacts = store),
        ],
    )
    for label, r in results.items():
        ctx.output.print(label, "FAILED" if r.failures() else "ok")

main = buildx_main(impl = _impl, cli_args = {})
