//! Bazel `--credential_helper` implementation.
//!
//! Bazel invokes the helper configured by `--credential_helper`
//! (<https://bazel.build/reference/command-line-reference#flag--credential_helper>)
//! as a short-lived subprocess with the command as its first argument, a request
//! JSON on stdin, and a response JSON on stdout. Bazel calls it as `<helper> get`,
//! so the command is `argv[1]`. The protocol's one command is `get`: read
//! `{"uri": "..."}` and return `{"headers": {"Authorization": ["Bearer <jwt>"]}}`.
//!
//! This helper emits the Aspect login JWT so Bazel attaches it to BES and
//! remote-cache gRPC requests. It deliberately bypasses workspace discovery and
//! the rest of the CLI: a tool may invoke it from anywhere and expects it to be
//! fast, so `main` intercepts it before the async runtime. `aspect` thus
//! reserves `get` as a top-level command name; a user task cannot shadow it.

use anyhow::Context;
use axl_runtime::engine::{resolve_access_token, resolve_profile};

/// The credential-helper command, as the spec passes it (`argv[1]`). Also the
/// reserved top-level command name the CLI guards in `cmd`.
pub const GET_COMMAND: &str = "get";

/// Whether argv designates the credential helper (`aspect get`).
pub fn is_invocation() -> bool {
    std::env::args().nth(1).as_deref() == Some(GET_COMMAND)
}

/// Run the `get` command, writing the response JSON to stdout: resolve the
/// Aspect login JWT and return it as a bearer `Authorization` header. An
/// unresolved token is an error (non-zero exit) so a tool never attaches a
/// credential a server will reject.
///
/// Runs before the CLI enters its main async runtime. `resolve_access_token` is
/// synchronous but its token-exchange and refresh paths internally call
/// `Handle::current().block_on(..)`, which needs a runtime registered on this
/// thread. We `enter()` a runtime to register the handle rather than driving an
/// outer future with `block_on`, since a nested `block_on` would panic.
pub fn run() -> anyhow::Result<()> {
    // The request `uri` is not needed (the same token is attached to every
    // Aspect endpoint), but stdin must be drained so the tool's write side does
    // not block.
    std::io::copy(&mut std::io::stdin(), &mut std::io::sink())
        .context("draining credential-helper request from stdin")?;

    // No flag is available on the helper path, so the profile comes from the
    // shared ASPECT_AUTH_PROFILE env var (or the default).
    let profile = resolve_profile(None);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the credential-helper runtime")?;
    let _guard = runtime.enter();
    let token = resolve_access_token(&profile)
        .context("resolving the Aspect access token")?
        .context("no Aspect credentials found; run `aspect auth login`")?;

    let response = serde_json::json!({
        "headers": { "Authorization": [format!("Bearer {token}")] }
    });
    println!("{response}");
    Ok(())
}
