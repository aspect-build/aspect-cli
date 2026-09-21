//! `aspect setup workspace-data` must report `BUILD_USER`.
//!
//! The task replaces Bazel's own default workspace status script, which is the
//! sole source of `BUILD_USER`, so pointing `--workspace_status_command` at it
//! used to blank out the Web UI's "User" row. The key is resolved from the
//! process environment inside `get_build_metadata`, which takes a `std` handle
//! and has no AXL-level fake, so running the real command is what covers it.
//!
//! The status command's stdout is a protocol payload Bazel parses as `KEY
//! value` lines, and Bazel fails the build outright on a non-zero exit — so
//! these assert the line's exact shape, and that an environment naming no user
//! still exits 0 rather than emitting a keyless line.

mod common;

use common::aspect_cli;
use std::process::{Command, Output};
use tempfile::TempDir;

/// Run the task with `USER`, `LOGNAME` and `USERNAME` cleared, then whatever
/// `env` re-adds. Every case here is driven by the environment alone: a Bazel
/// test runs in a sandbox with no git checkout, so nothing may depend on
/// `git show HEAD` finding a commit.
///
/// `$ASPECT_CREDENTIALS_FILE` forces the file credential backend. A case that
/// names a CI host turns on CI detection, and with it the Deployment feature's
/// `auto` endpoint wiring, which asks whether the deployment is logged in —
/// reaching the real keyring, which fails outright under Bazel's sandbox and
/// would prompt for authorization outside it.
fn workspace_data(env: &[(&str, &str)]) -> Output {
    let dir = TempDir::new().expect("temp dir");
    let mut cmd = Command::new(aspect_cli());
    cmd.args(["setup", "workspace-data"])
        .env(
            "ASPECT_CREDENTIALS_FILE",
            dir.path().join("credentials.json"),
        )
        .env_remove("ASPECT_AUTH_PROFILE")
        .env_remove("ASPECT_API_TOKEN")
        .env_remove("USER")
        .env_remove("LOGNAME")
        .env_remove("USERNAME");
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().expect("running setup workspace-data")
}

fn stdout_of(output: &Output) -> String {
    assert!(
        output.status.success(),
        "task exited {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn build_user_reports_the_account_the_build_runs_under() {
    let out = workspace_data(&[("USER", "build-account")]);
    assert!(
        stdout_of(&out)
            .lines()
            .any(|l| l == "BUILD_USER build-account"),
        "no BUILD_USER line in:\n{}",
        stdout_of(&out)
    );
}

/// `USER` is the commit author or CI actor; `BUILD_USER` is the machine
/// account. Where the two differ they must both be reported, unmerged: `$USER`
/// is only the last resort for `USER`, and the point of `BUILD_USER` is that it
/// names the account even when `USER` has a better answer.
///
/// A CI actor is what makes them differ without needing a git checkout.
#[test]
fn build_user_is_distinct_from_user() {
    let stdout = stdout_of(&workspace_data(&[
        ("USER", "build-account"),
        ("GITHUB_ACTIONS", "true"),
        ("GITHUB_ACTOR", "ci-actor"),
    ]));
    assert!(
        stdout.lines().any(|l| l == "USER ci-actor"),
        "expected the CI actor to win USER in:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|l| l == "BUILD_USER build-account"),
        "no BUILD_USER line in:\n{stdout}"
    );
}

/// A session that leaves `USER` unset still names the account.
#[test]
fn logname_stands_in_for_user() {
    let stdout = stdout_of(&workspace_data(&[("LOGNAME", "fallback-login")]));
    assert!(
        stdout.lines().any(|l| l == "BUILD_USER fallback-login"),
        "no BUILD_USER line in:\n{stdout}"
    );
}

/// Windows names the account `USERNAME`.
#[test]
fn username_stands_in_for_user() {
    let stdout = stdout_of(&workspace_data(&[("USERNAME", "win-account")]));
    assert!(
        stdout.lines().any(|l| l == "BUILD_USER win-account"),
        "no BUILD_USER line in:\n{stdout}"
    );
}

/// With nothing naming the account, the key is dropped rather than emitted
/// bare: Bazel would read `BUILD_USER` with an empty value, and a non-zero exit
/// here would fail the build that ran the status command.
#[test]
fn an_unnamed_account_drops_the_key() {
    let out = workspace_data(&[]);
    let stdout = stdout_of(&out);
    assert!(
        !stdout.lines().any(|l| l.starts_with("BUILD_USER")),
        "expected no BUILD_USER line in:\n{stdout}"
    );
}
