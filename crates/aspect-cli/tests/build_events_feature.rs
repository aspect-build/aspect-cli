//! The BuildEvents feature's options, set on the command line or in
//! `config.axl`, reach the gRPC sinks `grpc_sink` builds through
//! `build_events_settings`, and a negative count, a limit too small to hold a
//! publish request, a malformed duration or a `config.axl` value too large for
//! the runtime is refused before the task runs, naming the option. With the
//! feature disabled the defaults apply, and `BazelTrait` carries none of these
//! settings.
//!
//! A scratch workspace's `show` task prints the settings, and its `sink` task
//! builds a gRPC sink from them without dialing. Its `kept` task streams a
//! basil build to a sink no one listens on, as a user's task would, passing no
//! event size limit of its own, and prints how much of an oversized command
//! line its iterator saw. The built-in `build` task is run the same way, with
//! a `--bes-backend` no one listens on, and read from the warning the cut
//! command line gives.

mod common;

use common::{aspect_cli, base_env, basil_bin};
use std::process::{Command, Output};

const TASKS: &str = r#"load("@aspect//bazel/build_events.axl", "grpc_sink")
load("@aspect//private/lib/build_events_settings.axl", s = "build_events_settings")

def _show_impl(ctx: TaskContext) -> int:
    print("fields:%s|%s|%s|%s|%s" % (s.max_retries(ctx), s.retry_min_delay(ctx), s.retry_max_buffer_bytes(ctx), s.timeout(ctx), s.max_event_bytes(ctx)))
    return 0

show = task(implementation = _show_impl, args = {})

def _sink_impl(ctx: TaskContext) -> int:
    grpc_sink(ctx, "grpc://127.0.0.1:1")
    print("sink built")
    return 0

sink = task(implementation = _sink_impl, args = {})

def _kept_impl(ctx: TaskContext) -> int:
    iter = bazel.build_events.iterator(kinds = ["action_executed"])
    sink = grpc_sink(ctx, "grpc://127.0.0.1:1")
    build = ctx.bazel.build(
        flags = ["--scenario=oversized_command_line"],
        build_events = [iter, sink],
        stderr = None,
    )
    for event in iter:
        print("kept:%d" % len(event.payload.command_line))
    build.wait()
    sink.wait()
    return 0

kept = task(implementation = _kept_impl, args = {})
"#;

/// Run `aspect show <args>` in a scratch workspace under an empty `$HOME`.
fn show(args: &[&str]) -> Output {
    show_in(args, &[], None)
}

/// [`show`] with `env` set on top of the base environment and
/// `.aspect/config.axl` holding `config`, if given.
fn show_in(args: &[&str], env: &[(&str, &str)], config: Option<&str>) -> Output {
    run("show", args, env, config)
}

/// Run `aspect <task> <args>` in a scratch workspace as [`show_in`] does.
fn run(task: &str, args: &[&str], env: &[(&str, &str)], config: Option<&str>) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("tasks.axl"), TASKS).expect("tasks.axl");
    if let Some(config) = config {
        std::fs::write(aspect.join("config.axl"), config).expect("config.axl");
    }
    let output = Command::new(aspect_cli())
        .arg(task)
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("HOME", dir.path().join("home"))
        .envs(env.iter().copied())
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {task} {}`: {e}", args.join(" ")));
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
fn unset_options_leave_the_defaults() {
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
fn set_options_reach_the_tasks() {
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

/// A `config.axl` setting the BuildEvents `option` to `value`.
fn configured(option: &str, value: &str) -> String {
    format!(
        "load(\"@aspect//feature/build_events.axl\", \"BuildEvents\")\n\n\
         def config(ctx: ConfigContext):\n    \
         ctx.features[BuildEvents].args.{option} = {value}\n"
    )
}

/// `aspect show` with `config.axl` setting the BuildEvents `option` to `value`.
fn show_configured(option: &str, value: &str) -> Output {
    show_in(&[], &[], Some(&configured(option, value)))
}

/// A `config.axl` value is held to the bounds a command-line one is.
#[test]
fn a_bad_config_value_is_refused_before_the_task_runs() {
    assert_refused(
        show_configured("max_retries", "-1"),
        "--build-events:max-retries must be at least 0, got -1",
    );
    assert_refused(
        show_configured("retry_max_buffer_bytes", "-1"),
        "--build-events:retry-max-buffer-bytes must be at least 0, got -1",
    );
    assert_refused(
        show_configured("max_event_bytes", "1000"),
        "--build-events:max-event-bytes must be at least 65536 (64 KiB), got 1000",
    );
    assert_refused(
        show_configured("retry_min_delay", "\"soon\""),
        "--build-events:retry-min-delay must be a duration such as '500ms', '2s' or '1m', got 'soon'",
    );
    assert_refused(
        show_configured("timeout", "\"later\""),
        "--build-events:timeout must be a duration such as '500ms', '2s' or '1m', got 'later'",
    );
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
fn a_config_value_at_the_runtime_bound_reaches_the_tasks() {
    assert_eq!(
        fields(&show_configured("max_retries", "2147483647")),
        "2147483647|1s|None|0s|None"
    );
    assert_eq!(
        fields(&show_configured(
            "retry_max_buffer_bytes",
            "9223372036854775807"
        )),
        "4|1s|9223372036854775807|0s|None"
    );
    assert_eq!(
        fields(&show_configured("max_event_bytes", "9223372036854775807")),
        "4|1s|None|0s|9223372036854775807"
    );
}

#[test]
fn config_options_reach_the_tasks() {
    assert_eq!(
        fields(&show_configured("max_retries", "9")),
        "9|1s|None|0s|None"
    );
    assert_eq!(
        fields(&show_configured("timeout", "\"30s\"")),
        "4|1s|None|30s|None"
    );
    let built = run("sink", &[], &[], Some(&configured("timeout", "\"30s\"")));
    let stderr = String::from_utf8_lossy(&built.stderr);
    assert!(stderr.contains("sink built"), "{stderr}");
}

#[test]
fn a_disabled_feature_leaves_the_defaults() {
    let config = "load(\"@aspect//feature/build_events.axl\", \"BuildEvents\")\n\n\
                  def config(ctx: ConfigContext):\n    \
                  ctx.features[BuildEvents].enabled = False\n    \
                  ctx.features[BuildEvents].args.max_retries = 9\n";
    assert_eq!(
        fields(&show_in(&[], &[], Some(config))),
        "4|1s|None|0s|None"
    );
}

/// The settings live only on the feature: `BazelTrait` has no field for them.
#[test]
fn bazel_trait_has_no_build_events_settings() {
    let config = "load(\"@aspect//bazel.axl\", \"BazelTrait\")\n\n\
                  def config(ctx: ConfigContext):\n    \
                  ctx.traits[BazelTrait].bes_timeout = \"30s\"\n";
    let out = show_in(&[], &[], Some(config));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(out.status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("bes_timeout"), "{stderr}");
    assert!(!stderr.contains("fields:"), "the task ran:\n{stderr}");
}

/// How many of basil's `oversized_command_line` action's 2000 arguments of 100
/// bytes the `kept` task's iterator saw, run with `args`. The sink gives up
/// on its first failed dial.
fn kept(args: &[&str]) -> usize {
    let mut args = args.to_vec();
    args.push("--build-events:max-retries=0");
    let out = run(
        "kept",
        &args,
        &[
            ("BAZEL_REAL", basil_bin()),
            ("ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES", "1048576"),
        ],
        None,
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    stderr
        .lines()
        .find_map(|l| {
            l.split_once("kept:")
                .map(|(_, n)| n.trim().parse().unwrap())
        })
        .unwrap_or_else(|| panic!("no kept line in:\n{stderr}"))
}

/// A task that streams through `grpc_sink` holds the build to the feature's
/// `max_event_bytes` without passing it anywhere itself, cutting the command
/// line to what fits in 96 KiB, less 4 KiB for the request, at 102 encoded
/// bytes an argument. Unset, the 4 MiB default keeps all of it.
#[test]
fn a_task_streaming_through_grpc_sink_is_held_to_the_option() {
    assert_eq!(kept(&[]), 2000);
    let n = kept(&["--build-events:max-event-bytes=98304"]);
    let fits = (98304 - 4096) / 102;
    assert!((fits - 10..=fits).contains(&n), "kept {n}, {fits} fit");
}

/// A `config.axl` running every build under basil's `oversized_command_line`
/// scenario, with `extra` appended to `config`.
fn oversized_config(extra: &str) -> String {
    format!(
        "load(\"@aspect//feature/build_events.axl\", \"BuildEvents\")\n\
         load(\"@aspect//traits.axl\", \"BazelTrait\")\n\n\
         def config(ctx: ConfigContext):\n    \
         ctx.traits[BazelTrait].extra_flags.append(\"--scenario=oversized_command_line\")\n{extra}"
    )
}

/// How many arguments of basil's oversized command line the built-in `build`
/// task cut it to, streaming to a `--bes-backend` no one listens on, run with
/// `args` and `config.axl` extended by `extra`; None when it was not cut.
fn built_in_kept(args: &[&str], extra: &str) -> Option<usize> {
    let mut args = args.to_vec();
    args.extend([
        "--bes-backend=grpc://127.0.0.1:1",
        "--build-events:max-retries=0",
    ]);
    let out = run(
        "build",
        &args,
        &[
            ("BAZEL_REAL", basil_bin()),
            ("ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES", "1048576"),
        ],
        Some(&oversized_config(extra)),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "the scenario fails:\n{stderr}");
    stderr.lines().find_map(|l| {
        let (_, rest) = l.split_once("cut to the first ")?;
        let (n, _) = rest.split_once(" of 2000 arguments")?;
        Some(n.parse().unwrap())
    })
}

/// The built-in `build` task holds its BES stream to the feature's
/// `max_event_bytes`, set on the command line or in `config.axl`, and leaves
/// the command line whole under the 4 MiB default.
#[test]
fn the_built_in_build_task_is_held_to_the_option() {
    assert_eq!(built_in_kept(&[], ""), None);
    let fits = (98304 - 4096) / 102;
    for kept in [
        built_in_kept(&["--build-events:max-event-bytes=98304"], ""),
        built_in_kept(
            &[],
            "    ctx.features[BuildEvents].args.max_event_bytes = 98304\n",
        ),
    ] {
        let n = kept.expect("the command line was not cut");
        assert!((fits - 10..=fits).contains(&n), "kept {n}, {fits} fit");
    }
}
