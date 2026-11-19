//! `aspect auth login --with-token` against a configured deployment.
//!
//! A token read from stdin is stored as given, never exchanged or refreshed, so
//! the deployment it is filed under needs no issuer or client id. This runs the
//! real command against a deployment configured with neither.
//!
//! The run gets its own `$HOME`, so the deployment comes from a `config.json`
//! the test writes, and `$ASPECT_CREDENTIALS_FILE` forces the file backend so the
//! developer's keyring is never touched.

mod common;

use common::aspect_cli;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn with_token_logs_in_to_a_deployment_with_no_issuer_or_client() {
    let home = tempfile::tempdir().expect("temp dir");
    let aspect_dir = home.path().join(".aspect");
    std::fs::create_dir_all(&aspect_dir).unwrap();
    std::fs::write(
        aspect_dir.join("config.json"),
        r#"{"deployments":[{"name":"acme","hosts":["cache.acme.example"]}]}"#,
    )
    .unwrap();
    let credentials = home.path().join("credentials.json");

    // The payload is `{"email":"ci@acme.example","name":"CI"}`, base64url-encoded.
    let token = "header.eyJlbWFpbCI6ImNpQGFjbWUuZXhhbXBsZSIsIm5hbWUiOiJDSSJ9.sig";

    let mut child = Command::new(aspect_cli())
        .args(["auth", "login", "--with-token", "--deployment", "acme"])
        .current_dir(home.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("ASPECT_CREDENTIALS_FILE", &credentials)
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("running `aspect auth login`");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(token.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "auth login exited {:?}\n{stderr}",
        output.status.code()
    );
    assert!(
        stderr.contains("Logged in to Aspect Workflows deployment 'acme' as CI (ci@acme.example)"),
        "{stderr}"
    );

    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&credentials).unwrap()).unwrap();
    assert_eq!(
        stored["default"]["acme"]["access_token"].as_str(),
        Some(token),
        "{stored}"
    );
}
