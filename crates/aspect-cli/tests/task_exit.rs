//! A refusal raised through `TaskExit` ends the task without a traceback.
//!
//! `aspect auth use <unknown>` is the motivating case: the `unknown deployment`
//! message is complete on its own, and the `Traceback (most recent call last)`
//! block the evaluator used to print around it was noise. The path only reads
//! the user's deployment config, so the developer's own `~/.aspect/config.json`
//! is left alone; the credential store is redirected regardless so the test can
//! never reach a keyring.

use std::process::Command;

/// Locate the CLI under test: `ASPECT_CLI_BIN` under Bazel, cargo's
/// `CARGO_BIN_EXE_*` otherwise (`option_env!` because the cargo variable does
/// not exist in a Bazel build).
fn aspect_cli() -> String {
    match std::env::var("ASPECT_CLI_BIN") {
        Ok(p) => std::fs::canonicalize(&p)
            .unwrap_or_else(|e| panic!("ASPECT_CLI_BIN={p:?} not found: {e}"))
            .to_string_lossy()
            .into_owned(),
        Err(_) => option_env!("CARGO_BIN_EXE_aspect-cli")
            .expect("set ASPECT_CLI_BIN or run under cargo")
            .to_string(),
    }
}

#[test]
fn unknown_deployment_is_reported_without_a_traceback() {
    let dir = tempfile::tempdir().expect("temp dir");
    let name = format!("no-such-deployment-{}", std::process::id());
    let output = Command::new(aspect_cli())
        .args(["auth", "use", &name])
        .env(
            "ASPECT_CREDENTIALS_FILE",
            dir.path().join("credentials.json"),
        )
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .env_remove("ASPECT_DEBUG")
        .output()
        .expect("running `aspect auth use`");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1\n--- stderr ---\n{stderr}"
    );
    assert!(
        stderr.contains("ERROR: unknown deployment"),
        "expected the refusal as an ERROR line in:\n{stderr}"
    );
    assert!(
        !stderr.contains("Traceback"),
        "the refusal must not carry a traceback:\n{stderr}"
    );
}
