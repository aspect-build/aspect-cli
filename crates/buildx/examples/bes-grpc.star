# Run from the fixture workspace with the fake Build Event Service running:
#
#   cargo run -p buildx --example fake_bes -- --save received.bep
#
# It prints `grpc://127.0.0.1:PORT stats=http://127.0.0.1:PORT2/`; put the first
# word in `ENDPOINT` below. `stats` serves counters as JSON (lifecycle events,
# streams, stream events, acks), and `received.bep` is the stream the server got,
# readable with `cargo run -p buildx --example bep_dump -- received.bep`.
#
# A real service takes `grpcs://host:port`. Its certificate is checked against the
# JVM's trust store, so a private CA goes in with
# `JAVA_TOOL_OPTIONS=-Djavax.net.ssl.trustStore=...`.
#
# The server can also refuse or drop streams (`--refuse`, `--drop-after N`): the
# build still completes and stderr says why the upload did not.

ENDPOINT = "grpc://127.0.0.1:17891"

def _impl(ctx: Context):
    bes = ctx.events.bes(
        ENDPOINT,
        headers = {"x-team": "core"},
        instance = "main",
        results_url = "https://results.example/invocation/",
        metadata = {"ci": "false"},
    )
    results = ctx.build(["//:hello", "//:broken"], events = [bes])
    for label, r in results.items():
        ctx.output.print(label, "FAILED" if r.failures() else "ok")

main = buildx_main(impl = _impl, cli_args = {})
