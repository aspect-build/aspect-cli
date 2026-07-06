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
//! remote-cache gRPC requests. The request `uri`'s host selects which configured
//! deployment's credential to emit (so a self-hosted deployment's cache/BES gets
//! its own token); a host no configured deployment owns gets no header, so a
//! global `--credential_helper=aspect` never sends a token to a third party. It
//! deliberately bypasses workspace discovery and the rest of the CLI: a tool may
//! invoke it from anywhere and expects it to be fast, so `main` intercepts it
//! before the async runtime. `aspect` thus reserves `get` as a top-level command
//! name; a user task cannot shadow it.

use anyhow::Context;
use axl_runtime::engine::{profile_for_uri, resolve_access_token};

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
/// synchronous but its refresh / token-exchange paths internally call
/// `Handle::current().block_on(..)` on async HTTP, so it must run on a
/// `spawn_blocking` worker while the runtime itself drives the reactor via
/// `block_on` — the same arrangement `main` uses. (Merely `enter()`ing a
/// current-thread runtime registers a handle but never pumps the IO driver, so
/// the inner `block_on` on the refresh path would hang.)
pub fn run() -> anyhow::Result<()> {
    // Read the request `{"uri": "..."}`. A malformed/empty request yields an empty
    // URI, which owns no deployment and so gets no header (see below).
    let mut request = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut request)
        .context("reading credential-helper request from stdin")?;
    let uri = serde_json::from_str::<serde_json::Value>(&request)
        .ok()
        .and_then(|v| v.get("uri").and_then(|u| u.as_str()).map(str::to_owned))
        .unwrap_or_default();

    // The URI's host selects the configured deployment that owns the endpoint, so
    // a self-hosted deployment's cache/BES gets that deployment's own token.
    let resolved = profile_for_uri(&uri).context("resolving the deployment for the request URI")?;

    // Emit a credential only for a host a configured deployment owns; for any
    // other host return no headers so the helper self-scopes by the URI Bazel
    // passes (a global `--credential_helper=aspect` never sends a token to a
    // third-party host) and Bazel falls back to whatever it would otherwise use.
    let Some(deployment) = resolved.deployment else {
        println!("{}", no_credential_response());
        return Ok(());
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the credential-helper runtime")?;
    // A configured deployment stores its credential under its own name. Resolve on
    // a blocking worker so the runtime's `block_on` keeps driving the reactor while
    // `resolve_access_token`'s inner refresh `block_on` runs.
    let profile = deployment.clone();
    let token = runtime
        .block_on(async move { tokio::task::spawn_blocking(move || resolve_access_token(&profile)).await })
        .context("running the credential-helper token resolution")?
        .context("resolving the Aspect access token")?
        .with_context(|| {
            format!(
                "not logged in to the Aspect Workflows deployment '{deployment}'; run `aspect auth login --deployment {deployment}`"
            )
        })?;

    println!("{}", bearer_response(&token));
    Ok(())
}

/// The empty response Bazel reads as "no credential from this helper", so it
/// falls back to whatever it would otherwise use.
fn no_credential_response() -> serde_json::Value {
    serde_json::json!({ "headers": {} })
}

/// The response attaching `token` as a bearer `Authorization` header.
fn bearer_response(token: &str) -> serde_json::Value {
    serde_json::json!({ "headers": { "Authorization": [format!("Bearer {token}")] } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_credential_response_has_empty_headers() {
        // The security-critical shape for an unowned host: headers present but
        // empty, so no token ever reaches a third-party endpoint.
        assert_eq!(
            no_credential_response(),
            serde_json::json!({ "headers": {} })
        );
    }

    #[test]
    fn bearer_response_wraps_the_token() {
        assert_eq!(
            bearer_response("abc.def"),
            serde_json::json!({ "headers": { "Authorization": ["Bearer abc.def"] } })
        );
    }

    /// Guards the runtime arrangement `run()` relies on: the synchronous token
    /// resolver internally does `Handle::current().block_on(<async HTTP>)` on the
    /// refresh path, so it must run on a `spawn_blocking` worker while the runtime
    /// drives the reactor via `block_on`. The earlier `runtime.enter()`-only form
    /// registered a handle but never pumped the IO driver, so this same shape
    /// deadlocked as soon as a token needed refreshing. If someone reverts to
    /// `enter()`, this test hangs (and CI times out) rather than passing silently.
    #[test]
    fn resolver_arrangement_drives_inner_block_on() {
        use std::time::Duration;

        // Stand-in for resolve_access_token's refresh branch: a sync fn that
        // block_on's an awaiting (timer-driven) future via the ambient handle.
        fn sync_resolve_like() -> u32 {
            tokio::runtime::Handle::current().block_on(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                7
            })
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = runtime
            .block_on(async { tokio::task::spawn_blocking(sync_resolve_like).await })
            .unwrap();
        assert_eq!(out, 7);
    }
}
