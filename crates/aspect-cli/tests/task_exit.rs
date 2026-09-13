//! A refusal a Rust builtin raises through `TaskExit` ends the task with its
//! message and no traceback.
//!
//! `aspect auth use <unknown>` exercises the path end to end: the runtime
//! prints the `unknown deployment` refusal as an `ERROR:` line and exits 1.
//! The command only reads the deployment config before refusing, so the
//! developer's own `~/.aspect/config.json` is left alone; the credential store
//! is redirected regardless so the test can never reach a keyring.

mod common;

use common::aspect_cli;
use std::process::Command;

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
