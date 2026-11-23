//! The BuildEvents feature's options reach `BazelTrait`, as do the deprecated
//! `--workflows:bes-*` spellings, with a warning, unless the option is set
//! too to other than its default; a negative count or a limit too small to hold a publish request is
//! refused before the task runs.
//!
//! A scratch workspace's `show` task prints the trait's fields, so nothing
//! here runs Bazel.

mod common;

use common::{aspect_cli, base_env};
use std::process::{Command, Output};

const TASKS: &str = r#"load("@aspect//bazel.axl", "BazelTrait")

def _show_impl(ctx: TaskContext) -> int:
    t = ctx.traits[BazelTrait]
    print("fields:%s|%s|%s|%s|%s" % (t.bes_max_retries, t.bes_retry_min_delay, t.bes_retry_max_buffer_bytes, t.bes_timeout, t.max_build_event_bytes))
    return 0

show = task(implementation = _show_impl, traits = [BazelTrait], args = {})
"#;

/// Run `aspect show <args>` in a scratch workspace under an empty `$HOME`.
fn show(args: &[&str]) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("tasks.axl"), TASKS).expect("tasks.axl");
    let output = Command::new(aspect_cli())
        .arg("show")
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("HOME", dir.path().join("home"))
        .output()
        .unwrap_or_else(|e| panic!("running `aspect show {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

/// The fields line `show` printed.
fn fields(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    stderr
        .lines()
        .find_map(|l| l.split_once("fields:").map(|(_, f)| f.to_owned()))
        .unwrap_or_else(|| panic!("no fields line in:\n{stderr}"))
}

#[test]
fn unset_options_leave_the_trait_defaults() {
    let out = show(&[]);
    assert_eq!(fields(&out), "4|1s|None|0s|None");
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("deprecated"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn set_options_reach_the_trait() {
    let out = show(&[
        "--build-events:max-retries=7",
        "--build-events:retry-min-delay=250ms",
        "--build-events:retry-max-buffer-bytes=1048576",
        "--build-events:timeout=90s",
        "--build-events:max-event-bytes=16777216",
    ]);
    assert_eq!(fields(&out), "7|250ms|1048576|90s|16777216");
}

#[test]
fn the_workflows_spellings_still_apply_and_warn() {
    let out = show(&[
        "--workflows:bes-max-retries=0",
        "--workflows:bes-retry-min-delay=2s",
        "--workflows:bes-retry-max-buffer-bytes=65536",
        "--workflows:bes-timeout=5m",
    ]);
    assert_eq!(fields(&out), "0|2s|65536|5m|None");
    let stderr = String::from_utf8_lossy(&out.stderr);
    for (old, new) in [
        ("bes-max-retries", "max-retries"),
        ("bes-retry-min-delay", "retry-min-delay"),
        ("bes-retry-max-buffer-bytes", "retry-max-buffer-bytes"),
        ("bes-timeout", "timeout"),
    ] {
        let warning = format!("--workflows:{old} is deprecated; use --build-events:{new}.");
        assert!(stderr.contains(&warning), "no `{warning}` in:\n{stderr}");
    }
}

#[test]
fn an_option_outranks_its_workflows_spelling() {
    let out = show(&[
        "--workflows:bes-max-retries=2",
        "--build-events:max-retries=7",
        "--workflows:bes-timeout=5m",
    ]);
    assert_eq!(fields(&out), "7|1s|None|5m|None");
}

/// An option given its default is not told from one left unset, so the
/// Workflows spelling's value stands.
#[test]
fn an_option_at_its_default_leaves_the_workflows_spelling() {
    let out = show(&[
        "--workflows:bes-max-retries=2",
        "--build-events:max-retries=4",
    ]);
    assert_eq!(fields(&out), "2|1s|None|0s|None");
}

/// `aspect show <args>` fails before the task runs with `error`, and no
/// traceback.
fn assert_refused(args: &[&str], error: &str) {
    let out = show(args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains(&format!("ERROR: {error}")), "{stderr}");
    assert!(!stderr.contains("fields:"), "the task ran:\n{stderr}");
    assert!(!stderr.contains("Traceback"), "{stderr}");
}

#[test]
fn a_limit_below_64_kib_is_refused_before_the_task_runs() {
    assert_refused(
        &["--build-events:max-event-bytes=1000"],
        "--build-events:max-event-bytes must be at least 65536 (64 KiB), got 1000",
    );
}

#[test]
fn negative_counts_are_refused_before_the_task_runs() {
    assert_refused(
        &["--build-events:max-retries=-1"],
        "--build-events:max-retries must be at least 0, got -1",
    );
    assert_refused(
        &["--build-events:retry-max-buffer-bytes=-1"],
        "--build-events:retry-max-buffer-bytes must be at least 0, got -1",
    );
}
