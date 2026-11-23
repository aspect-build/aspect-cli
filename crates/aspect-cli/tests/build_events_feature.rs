//! The BuildEvents feature's options reach `BazelTrait`, and a negative count,
//! a limit too small to hold a publish request, a malformed duration or a
//! `config.axl` value too large for the runtime is refused before the task
//! runs.
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
    show_in(args, &[], None)
}

/// [`show`] with `env` set on top of the base environment and
/// `.aspect/config.axl` holding `config`, if given.
fn show_in(args: &[&str], env: &[(&str, &str)], config: Option<&str>) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("tasks.axl"), TASKS).expect("tasks.axl");
    if let Some(config) = config {
        std::fs::write(aspect.join("config.axl"), config).expect("config.axl");
    }
    let output = Command::new(aspect_cli())
        .arg("show")
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("HOME", dir.path().join("home"))
        .envs(env.iter().copied())
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
    assert_eq!(fields(&show(&[])), "4|1s|None|0s|None");
}

/// Checking the durations builds no sink that reads the buffer budget's
/// environment, so a task that streams nowhere does not warn about it.
#[test]
fn validating_the_options_leaves_the_buffer_env_unread() {
    let out = show_in(
        &[],
        &[("ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES", "bogus")],
        None,
    );
    assert_eq!(fields(&out), "4|1s|None|0s|None");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES"),
        "{stderr}"
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

/// `out` is of a run that failed before the task ran with `error`, and no
/// traceback.
fn assert_refused(out: Output, error: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains(&format!("ERROR: {error}")), "{stderr}");
    assert!(!stderr.contains("fields:"), "the task ran:\n{stderr}");
    assert!(!stderr.contains("Traceback"), "{stderr}");
}

#[test]
fn a_limit_below_64_kib_is_refused_before_the_task_runs() {
    assert_refused(
        show(&["--build-events:max-event-bytes=1000"]),
        "--build-events:max-event-bytes must be at least 65536 (64 KiB), got 1000",
    );
}

#[test]
fn negative_counts_are_refused_before_the_task_runs() {
    assert_refused(
        show(&["--build-events:max-retries=-1"]),
        "--build-events:max-retries must be at least 0, got -1",
    );
    assert_refused(
        show(&["--build-events:retry-max-buffer-bytes=-1"]),
        "--build-events:retry-max-buffer-bytes must be at least 0, got -1",
    );
}

#[test]
fn malformed_durations_are_refused_before_the_task_runs() {
    assert_refused(
        show(&["--build-events:retry-min-delay=soon"]),
        "--build-events:retry-min-delay must be a duration such as '500ms', '2s' or '1m', got 'soon'",
    );
    assert_refused(
        show(&["--build-events:timeout=later"]),
        "--build-events:timeout must be a duration such as '500ms', '2s' or '1m', got 'later'",
    );
}

/// `aspect show` with `config.axl` setting the BuildEvents `option` to `value`.
fn show_configured(option: &str, value: &str) -> Output {
    let config = format!(
        "load(\"@aspect//feature/build_events.axl\", \"BuildEvents\")\n\n\
         def config(ctx: ConfigContext):\n    \
         ctx.features[BuildEvents].args.{option} = {value}\n"
    );
    show_in(&[], &[], Some(&config))
}

/// A `config.axl` value skips the flag parser's range check, so the feature
/// refuses one past what the runtime takes instead of failing when a sink is
/// built.
#[test]
fn a_config_value_too_large_for_the_runtime_is_refused_before_the_task_runs() {
    assert_refused(
        show_configured("max_retries", "2147483648"),
        "--build-events:max-retries must be at most 2147483647, got 2147483648",
    );
    assert_refused(
        show_configured("retry_max_buffer_bytes", "9223372036854775808"),
        "--build-events:retry-max-buffer-bytes must be at most 9223372036854775807, got 9223372036854775808",
    );
    assert_refused(
        show_configured("max_event_bytes", "9223372036854775808"),
        "--build-events:max-event-bytes must be at most 9223372036854775807, got 9223372036854775808",
    );
}

/// The largest value the runtime takes is not refused.
#[test]
fn a_config_value_at_the_runtime_bound_reaches_the_trait() {
    assert_eq!(
        fields(&show_configured("max_retries", "2147483647")),
        "2147483647|1s|None|0s|None"
    );
    assert_eq!(
        fields(&show_configured("max_event_bytes", "9223372036854775807")),
        "4|1s|None|0s|9223372036854775807"
    );
}
