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

/// A CLI a test spawns keeping the whole environment, as this file's do,
/// records its run in the test process's own history, never the developer's.
#[test]
fn a_spawned_clis_history_is_the_test_processs_own() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.bazel"), "").expect("MODULE.bazel");
    std::fs::create_dir(dir.path().join(".aspect")).expect(".aspect");
    std::fs::write(
        dir.path().join(".aspect/tasks.axl"),
        "def _impl(ctx):\n    return 0\n\nguarded = task(implementation = _impl, summary = \"x\")\n",
    )
    .expect("tasks.axl");
    let status = Command::new(aspect_cli())
        .arg("guarded")
        .current_dir(dir.path())
        .status()
        .expect("running the CLI");
    assert!(status.success(), "{status:?}");
    let history = common::test_history_dir();
    assert!(history.starts_with(common::scratch_root()), "{history:?}");
    let here = std::fs::canonicalize(dir.path()).expect("canonical");
    let recorded = std::fs::read_dir(&history)
        .expect("the test's history")
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("task.json")).ok())
        .filter_map(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .any(|t| {
            t["cwd"]
                .as_str()
                .and_then(|c| std::fs::canonicalize(c).ok())
                .is_some_and(|c| c == here)
        });
    assert!(recorded, "no entry for {here:?} in {history:?}");
}

/// A post-task hook that holds the run (as the Dashboard's finished screen
/// does while it is read) is not the task's time: its record ends where its
/// implementation returned, with that return's exit code.
#[test]
fn a_held_post_task_hook_is_not_in_the_recorded_duration() {
    const HOLD_MS: u64 = 3000;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.bazel"), "").expect("MODULE.bazel");
    std::fs::create_dir(dir.path().join(".aspect")).expect(".aspect");
    std::fs::write(
        dir.path().join(".aspect/held.axl"),
        format!(
            r#"load("@std//time.axl", "sleep")

def _hold(ctx, outcome):
    sleep({HOLD_MS})

def _impl(ctx):
    ctx.hooks.post_task(_hold)
    return 3

held = task(implementation = _impl, summary = "Test fixture.")
"#
        ),
    )
    .expect("held.axl");
    let history = dir.path().join("history");
    let status = Command::new(aspect_cli())
        .arg("held")
        .current_dir(dir.path())
        .env("ASPECT_HISTORY_DIR", &history)
        .env(
            "ASPECT_CREDENTIALS_FILE",
            dir.path().join("credentials.json"),
        )
        .env_remove("ASPECT_DEBUG")
        .status()
        .expect("running the CLI");
    assert_eq!(status.code(), Some(3), "{status:?}");
    let record: serde_json::Value = std::fs::read_dir(&history)
        .expect("the history")
        .flatten()
        .find_map(|e| std::fs::read_to_string(e.path().join("task.json")).ok())
        .map(|t| serde_json::from_str(&t).expect("task.json"))
        .expect("a task.json");
    assert_eq!(record["status"], "ended", "{record}");
    assert_eq!(record["exit_code"], 3, "{record}");
    let took = record["duration_ms"].as_u64().expect("duration_ms");
    assert!(
        took < HOLD_MS - 1000,
        "the hold counted: {took}ms in {record}"
    );
}
