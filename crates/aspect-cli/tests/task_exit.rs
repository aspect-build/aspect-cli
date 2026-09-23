//! A refusal a Rust builtin raises through `TaskExit` ends the run with its
//! message and no traceback, on both paths the runtime has for it.
//!
//! `aspect auth use <unknown>` refuses inside the task body, so the task runner
//! reports it. `aspect build --remote --deployment=<unknown>` refuses inside
//! the Deployment feature, before the task runs, so the CLI's top-level error
//! arm reports it instead. Both only read the deployment config before
//! refusing, so the developer's own `~/.aspect/config.json` is left alone; the
//! credential store is redirected regardless so no test can reach a keyring.
//!
//! An AXL error value whose type has `traceback = False` is the same kind of
//! refusal. Raised from a task body, the task runner reports it; raised from
//! `config.axl`, before any task runs, the CLI's top-level arm does.

mod common;

use common::aspect_cli;
use std::process::{Command, Output};

/// Run the CLI with `args` against an isolated credential store and a
/// deployment name that cannot exist.
fn refuse(args: &[&str]) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    let name = format!("no-such-deployment-{}", std::process::id());
    let args: Vec<String> = args.iter().map(|a| a.replace("{name}", &name)).collect();
    let output = Command::new(aspect_cli())
        .args(&args)
        .env(
            "ASPECT_CREDENTIALS_FILE",
            dir.path().join("credentials.json"),
        )
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .env_remove("ASPECT_DEBUG")
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

fn assert_clean_refusal(output: &Output) {
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

#[test]
fn a_task_body_refusal_is_reported_without_a_traceback() {
    assert_clean_refusal(&refuse(&["auth", "use", "{name}"]));
}

#[test]
fn a_feature_impl_refusal_is_reported_without_a_traceback() {
    assert_clean_refusal(&refuse(&[
        "build",
        "--remote",
        "--deployment={name}",
        "//nothing",
    ]));
}

/// Run the CLI with `args` in a scratch workspace whose `.aspect/` holds
/// `files`, each a `(name, contents)` pair.
fn run_in_workspace(files: &[(&str, &str)], args: &[&str]) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.bazel"), "").expect("MODULE.bazel");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    for (name, contents) in files {
        std::fs::write(aspect.join(name), contents).expect("writing a fixture");
    }
    let output = Command::new(aspect_cli())
        .args(args)
        .current_dir(dir.path())
        .env(
            "ASPECT_CREDENTIALS_FILE",
            dir.path().join("credentials.json"),
        )
        .env_remove("ASPECT_DEBUG")
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

const REFUSE_TASK: &str = r#"
NotLoggedIn = error.type(traceback = False)

def _impl(ctx):
    fail(NotLoggedIn("unknown deployment: log in first"))

refuse = task(summary = "Test fixture.", implementation = _impl)
"#;

#[test]
fn a_traceback_free_error_from_a_task_body_is_reported_without_a_traceback() {
    assert_clean_refusal(&run_in_workspace(
        &[("refuse.axl", REFUSE_TASK)],
        &["refuse"],
    ));
}

#[test]
fn a_traceback_free_error_from_config_is_reported_without_a_traceback() {
    let config = r#"
NotLoggedIn = error.type(traceback = False)

def config(ctx):
    fail(NotLoggedIn("unknown deployment: log in first"))
"#;
    let noop = r#"
noop = task(summary = "Test fixture.", implementation = lambda ctx: 0)
"#;
    assert_clean_refusal(&run_in_workspace(
        &[("config.axl", config), ("noop.axl", noop)],
        &["noop"],
    ));
}
