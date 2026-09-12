//! `aspect auth status` must render every row it asks for.
//!
//! The `auth` AXL reads a `DeploymentSummary` field by field, but the summary is
//! a Rust Starlark value whose attributes are declared separately from the struct
//! itself. Adding a field without its `#[starlark(attribute)]` accessor compiles,
//! passes every unit test, and then fails at runtime with "has no attribute" the
//! first time the row is printed.
//!
//! Neither suite covers that contract: the Rust tests never evaluate AXL, and
//! `auth_test.axl` substitutes a plain Starlark struct for the summary, which has
//! whatever fields the test gives it. Running the real command is the only thing
//! that reads the real value, so this does exactly that.
//!
//! Deliberately assertion-light about *what* is rendered — that is `auth_test.axl`'s
//! job, where it can be checked without a subprocess. This asserts the command
//! completes, which is the part only an end-to-end run can tell us.

use std::process::Command;

/// Locate the CLI under test.
///
/// Bazel sets `ASPECT_CLI_BIN` from the `rust_test` rule's `env` via
/// `$(rootpath :aspect-cli)`, relative to the runfiles root that is a Bazel-run
/// test's cwd. Under cargo, `CARGO_BIN_EXE_*` points at the binary cargo already
/// built for this test.
fn aspect_cli() -> String {
    match std::env::var("ASPECT_CLI_BIN") {
        Ok(p) => std::fs::canonicalize(&p)
            .unwrap_or_else(|e| panic!("ASPECT_CLI_BIN={p:?} not found: {e}"))
            .to_string_lossy()
            .into_owned(),
        // `option_env!`, not `env!`: the cargo variable does not exist in a Bazel
        // build, and `env!` would fail to compile there.
        Err(_) => option_env!("CARGO_BIN_EXE_aspect-cli")
            .expect("set ASPECT_CLI_BIN or run under cargo")
            .to_string(),
    }
}

/// Run `auth status` against an empty credential store.
///
/// `$ASPECT_CREDENTIALS_FILE` forces the file backend, so the test never reaches
/// the developer's real keyring — which on macOS would block on an authorization
/// dialog, and which must not be read or written by a test either way.
///
/// `$HOME` is left alone, so any deployments in the developer's own
/// `config.json` are listed too. The assertions are about the built-in Aspect
/// Cloud entry, which is present whatever else is configured; overriding `$HOME`
/// would isolate them at the cost of re-extracting the AXL bundle on every run.
fn auth_status(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(aspect_cli())
        .args(["auth", "status"])
        .args(args)
        .env("ASPECT_CREDENTIALS_FILE", dir.join("credentials.json"))
        // Keep the run off any ambient profile selection.
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .output()
        .expect("running `aspect auth status`")
}

/// Assert the run succeeded, and return `(stdout, stderr)`.
///
/// Both matter, and they carry different things: the aligned table is written by
/// AXL `print()`, which goes through starlark's print handler to **stderr**, while
/// the JSON document is written to stdout through `ctx.std.io.stdout`. A test that
/// looks only at stdout sees nothing at all for the text output.
fn assert_ok(output: &std::process::Output, what: &str) -> (String, String) {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{what} exited {:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status.code()
    );
    // A Starlark failure does not always change the exit code, so look for it.
    assert!(
        !stderr.contains("has no attribute"),
        "{what} hit a missing Starlark attribute:\n{stderr}"
    );
    (stdout, stderr)
}

/// The logged-out path renders every row, including the `Log in` block whose
/// options name the deployment's API-token variable.
#[test]
fn auth_status_renders_a_logged_out_store() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (_, rendered) = assert_ok(&auth_status(dir.path(), &[]), "auth status");

    // Aspect Cloud is always listed, logged in or not.
    assert!(
        rendered.contains("Aspect Cloud"),
        "expected Aspect Cloud in:\n{rendered}"
    );
    // The rows that read the attributes this test exists to guard.
    assert!(
        rendered.contains("ASPECT_API_TOKEN"),
        "expected the API-token variable in:\n{rendered}"
    );
    assert!(
        rendered.contains("Log in"),
        "expected the login options in:\n{rendered}"
    );
}

/// The JSON document goes through a different builder than the text table, so it
/// reads its own set of attributes.
#[test]
fn auth_status_renders_json() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (stdout, _) = assert_ok(
        &auth_status(dir.path(), &["--output", "json"]),
        "auth status --output json",
    );

    let start = stdout.find('{').expect("a JSON document on stdout");
    let doc: serde_json::Value =
        serde_json::from_str(&stdout[start..]).expect("stdout parses as JSON");
    let cloud = doc["account"]
        .as_array()
        .and_then(|rows| rows.first())
        .expect("an Aspect Cloud row");
    assert_eq!(cloud["api_token_env"], "ASPECT_API_TOKEN");
    assert_eq!(cloud["login_command"], "aspect auth login");
}
