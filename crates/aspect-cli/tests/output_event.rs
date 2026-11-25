//! The `build` task's console processing, for real, against basil.
//!
//! * The `Console` feature's default stages collapse and shorten, its tip
//!   rules emit tips, and Bazel's raw console is kept in a log the summary
//!   names; disabled, the console is Bazel's own.
//! * `output_intercept` hooks answer the lines an `intercept` stage holds. Its
//!   timeout is far longer than the test, so a line nothing answered would
//!   show up unchanged and late: both the rewritten output and the elapsed
//!   time say the drain happened.
//! * A cancelled build still forwards what Bazel prints while it winds down.

#![cfg(unix)]

mod common;

use common::{aspect_cli, base_env, basil_bin};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The `intercept` stage's `timeout_ms`, as a bound the run must stay under.
const INTERCEPT_TIMEOUT: Duration = Duration::from_secs(60);

/// The config a case runs under. `no_bes` clears the trait's `build_event`
/// hooks, so the task has no event iterator and no drain loop: every line is
/// answered by `run_output_hooks(until_ended = True)`, which runs for the
/// whole build.
fn config(report: &str, no_bes: bool) -> String {
    let clear = if no_bes {
        "    t.build_start.append(lambda _ctx: t.build_event.clear())\n"
    } else {
        ""
    };
    format!(
        r#"load("@aspect//traits.axl", "BazelTrait")

def config(ctx: ConfigContext):
    seen = []
    after = []

    def _report():
        ctx.std.fs.create("{report}").write(",".join(seen) + ";" + ",".join(after) + "\n")

    def _note(task_ctx, event):
        seen.append(event.id)
        _report()

    def _after(task_ctx, event):
        after.append(event.id)
        _report()

    def _answer(task_ctx, event):
        if event.id == "secret":
            event.replace("(password elided)")
        elif event.id == "warn":
            event.drop()

    t = ctx.traits[BazelTrait]
    t.output_stages.append(bazel.output.intercept(
        {{"secret": "password", "warn": "^WARNING:", "info": "^INFO: Build"}},
        timeout_ms = {timeout},
    ))
    t.output_intercept.append(_note)
    t.output_intercept.append(_answer)
    t.output_intercept.append(_after)
{clear}    t.extra_flags.append("--scenario=stderr_chatter")
"#,
        timeout = INTERCEPT_TIMEOUT.as_millis(),
    )
}

/// A scratch workspace whose `.aspect/<file>` holds `source(root)`.
fn workspace(file: &str, source: impl FnOnce(&Path) -> String) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    std::fs::write(root.join("MODULE.bazel"), "").unwrap();
    std::fs::write(root.join("MODULE.aspect"), "").unwrap();
    std::fs::create_dir(root.join(".aspect")).unwrap();
    std::fs::write(root.join(".aspect").join(file), source(root)).unwrap();
    dir
}

/// `aspect <task>` in `root`, with basil as Bazel.
fn aspect(root: &Path, task: &str) -> Command {
    let mut cmd = Command::new(aspect_cli());
    cmd.arg(task)
        .current_dir(root)
        .env_clear()
        .envs(base_env())
        .env("BAZEL_REAL", basil_bin())
        .env("ASPECT_CREDENTIALS_FILE", root.join("credentials.json"))
        // The task history, where the raw console logs go, of its own.
        .env("ASPECT_HISTORY_DIR", root.join("history"))
        // Not `root` itself, whose `.aspect/` would then load as user config.
        .env("HOME", root.join("home"))
        .stdin(Stdio::null());
    cmd
}

/// Run `aspect build` in a fresh workspace; return its stderr and the hooks'
/// report: the ids the first hook saw, `;`, then those the last one saw.
fn run(no_bes: bool) -> (String, String) {
    let dir = workspace("config.axl", |root| {
        config(&root.join("report.txt").display().to_string(), no_bes)
    });
    let report = dir.path().join("report.txt");

    let start = Instant::now();
    let out = aspect(dir.path(), "build")
        .output()
        .expect("running aspect build");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        start.elapsed() < INTERCEPT_TIMEOUT,
        "a held line waited out its timeout:\n{stderr}"
    );
    assert!(out.status.success(), "aspect build failed:\n{stderr}");
    let seen = std::fs::read_to_string(&report).unwrap_or_default();
    (stderr, seen.trim().to_string())
}

fn assert_answered(stderr: &str, seen: &str) {
    assert_eq!(
        seen, "warn,warn,secret,info;info",
        "every hook up to the one that answers, and no further: {stderr}"
    );
    assert!(stderr.contains("(password elided)"), "{stderr}");
    assert!(!stderr.contains("hunter2"), "{stderr}");
    assert!(
        !stderr.contains("WARNING: deprecated flag used"),
        "{stderr}"
    );
    assert!(stderr.contains("INFO: Analyzed 1 target"), "{stderr}");
    assert!(
        stderr.contains("INFO: Build completed successfully"),
        "a line no hook answers is kept: {stderr}"
    );
}

#[test]
fn the_build_task_answers_held_lines_through_the_hooks() {
    let (stderr, seen) = run(false);
    assert_answered(&stderr, &seen);
}

#[test]
fn an_intercepting_processor_drains_without_a_build_event_consumer() {
    let (stderr, seen) = run(true);
    assert_answered(&stderr, &seen);
}

/// `aspect build` in a fresh workspace with basil's `console_noise`
/// scenario, `args` and `env`; its stderr.
fn noisy_build(args: &[&str], env: &[(&str, &str)]) -> (tempfile::TempDir, String) {
    let dir = workspace("config.axl", |_| {
        r#"load("@aspect//traits.axl", "BazelTrait")

def config(ctx: ConfigContext):
    ctx.traits[BazelTrait].extra_flags.append("--scenario=console_noise")
"#
        .to_string()
    });
    // One file for both streams, as a terminal or `2>&1` gives: only then
    // does a pipe capture ask bazel for the colour its messages are told by.
    let log = dir.path().join("console.txt");
    let file = std::fs::File::create(&log).unwrap();
    let status = aspect(dir.path(), "build")
        .args(args)
        .envs(env.iter().copied())
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .status()
        .expect("running aspect build");
    let console = std::fs::read_to_string(&log).unwrap();
    assert!(status.success(), "aspect build failed:\n{console}");
    (dir, console)
}

/// A `--color=no` the user's rc sets decides what is shown: the console comes
/// out plain. Bazel is still asked for colour, after it, so the stages still
/// tell its messages apart and collapse them.
#[test]
fn the_users_own_color_choice_stands() {
    let dir = workspace("config.axl", |root| {
        std::fs::write(root.join(".bazelrc"), "common --color=no\n").unwrap();
        r#"load("@aspect//traits.axl", "BazelTrait")

def config(ctx: ConfigContext):
    ctx.traits[BazelTrait].extra_flags.append("--scenario=console_noise")
"#
        .to_string()
    });
    let log = dir.path().join("console.txt");
    let file = std::fs::File::create(&log).unwrap();
    let status = aspect(dir.path(), "build")
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .status()
        .expect("running aspect build");
    let console = std::fs::read_to_string(&log).unwrap();
    assert!(status.success(), "{console}");
    assert!(!console.contains('\x1b'), "{console}");
    assert!(console.contains("Aspect hid"), "{console}");
}

/// A `--bazel-flag=--color=…` decides as an rc's does.
#[test]
fn a_color_flag_of_the_users_own_stands() {
    let (_dir, console) = noisy_build(&["--bazel-flag=--color=no"], &[]);
    assert!(console.contains("Aspect hid"), "{console}");
    assert!(!console.contains('\x1b'), "{console}");
}

/// With stdout a pipe of its own, the capture takes it too and strips the
/// colour it asked for from both: the console is processed, and plain.
#[test]
fn a_separate_stdout_is_captured_and_plain() {
    let dir = workspace("config.axl", |_| {
        r#"load("@aspect//traits.axl", "BazelTrait")

def config(ctx: ConfigContext):
    ctx.traits[BazelTrait].extra_flags.append("--scenario=console_noise")
"#
        .to_string()
    });
    let out = aspect(dir.path(), "build")
        .output()
        .expect("running aspect build");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("Aspect hid"), "{stderr}");
    assert!(!stderr.contains('\x1b'), "{stderr}");
    assert!(!stdout.contains('\x1b'), "{stdout}");
}

#[test]
fn the_console_feature_makes_the_build_console_easier_to_read() {
    let (dir, stderr) = noisy_build(&[], &[]);
    let count = |needle: &str| stderr.matches(needle).count();
    assert_eq!(
        count("errors encountered while analyzing target"),
        2,
        "one shown, one quoted in the summary:\n{stderr}"
    );
    assert_eq!(
        count("Analysis failed"),
        2,
        "a pipe marks no body, so an unindented line under a hidden warning stays, \
         and the two in a row fold: {stderr}"
    );
    assert!(
        stderr.contains("Analysis failed\n(last line repeated 1 more time)\n"),
        "{stderr}"
    );
    for advice in [
        "canonical reproducible form",
        "defined by rule",
        "instantiated at:",
        "Repository rule",
    ] {
        assert!(
            stderr.contains(advice),
            "nothing is dropped by default, {advice:?} included:\n{stderr}"
        );
    }
    assert!(
        stderr.contains("src/a.ts:3:4 from src/a.ts:3:4"),
        "{stderr}"
    );
    assert!(
        stderr.contains("TIP[server-crash] 🟡 warning: The Bazel server crashed"),
        "{stderr}"
    );
    assert!(
        stderr.contains(
            "TIP[tips-hint] 💬 note: Silence a tip with `--tips:silence=<id>` (e.g. `--tips:silence=tips-hint`) or in `.aspect/config.axl`"
        ),
        "the hint:\n{stderr}"
    );
    assert_eq!(
        stderr.matches("TIP[tips-hint]").count(),
        1,
        "the hint once:\n{stderr}"
    );
    assert!(
        stderr.find("TIP[tips-hint]") > stderr.find("TIP[server-crash]"),
        "the hint after the tips:\n{stderr}"
    );
    assert!(
        !stderr.contains("[1 / 2] Compiling"),
        "plain lines show no status line before 5s: {stderr}"
    );
    let summary = stderr
        .lines()
        .find(|l| l.starts_with("INFO: Aspect hid 2 console lines; the raw console is in "))
        .unwrap_or_else(|| panic!("no summary:\n{stderr}"));
    assert!(
        stderr
            .contains("  2 more like: WARNING: errors encountered while analyzing target '//:g0'"),
        "{stderr}"
    );
    let log = summary.rsplit(" is in ").next().unwrap();
    let history = std::fs::canonicalize(dir.path().join("history")).expect("the history");
    assert!(
        std::fs::canonicalize(log).unwrap().starts_with(&history)
            && log.ends_with("/bazel/1/console.log"),
        "the raw log is the build's own, in the task invocation's history: {log}"
    );
    let group = stderr
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("all 3 in "))
        .unwrap_or_else(|| panic!("no group file named:\n{stderr}"));
    assert_eq!(group, log.replace(".log", ".collapsed-1.log"), "{stderr}");
    let messages = std::fs::read_to_string(group).expect("the group file the summary names");
    assert_eq!(
        messages
            .matches("WARNING: errors encountered while analyzing target")
            .count(),
        3,
        "{messages}"
    );
    let raw = std::fs::read_to_string(log).expect("the raw log the summary names");
    assert_eq!(raw.matches("Analysis failed").count(), 3, "{raw}");
    assert!(raw.contains("/execroot/_main/src/a.ts"), "{raw}");
}

/// The user's own `--curses` decides what is shown, under any profile:
/// `yes` keeps the curses redraws in the output, `no` gives plain lines.
#[test]
fn the_users_own_curses_choice_decides_the_output() {
    let (_dir, stderr) = noisy_build(&["--bazel-flag=--curses=yes"], &[("CLAUDECODE", "1")]);
    assert!(
        stderr.contains("\x1b[1A"),
        "curses kept for an agent that asked:\n{stderr}"
    );
    let (_dir, stderr) = noisy_build(
        &["--bazel-flag=--curses=no", "--console:profile=human"],
        &[],
    );
    assert!(!stderr.contains("\x1b[1A"), "{stderr}");
    assert!(stderr.contains("Aspect hid"), "{stderr}");
}

/// A coding agent gets the `agent` profile: plain lines without Bazel's
/// progress, every test's output kept.
#[test]
fn an_agent_gets_the_agent_profile() {
    let (_dir, stderr) = noisy_build(&[], &[("CLAUDECODE", "1")]);
    assert!(!stderr.contains("[1 / 2] Compiling"), "{stderr}");
    assert!(stderr.contains("assertion a failed"), "{stderr}");
    assert!(stderr.contains("assertion b failed"), "{stderr}");
    assert!(
        stderr.contains("INFO: Aspect hid 2 console lines"),
        "{stderr}"
    );
}

/// The feature runs for every task, so a bad setting warns and the rest of
/// the defaults still apply.
#[test]
fn a_bad_console_setting_warns_instead_of_failing() {
    let (_dir, stderr) = noisy_build(&["--console:disable=bogus", "--console:drop=("], &[]);
    assert!(
        stderr.contains("unknown console stage \"bogus\" is ignored"),
        "{stderr}"
    );
    assert!(
        stderr.contains("console pattern ignored: invalid regex for '('"),
        "{stderr}"
    );
    assert!(
        stderr.contains("INFO: Aspect hid 2 console lines"),
        "{stderr}"
    );
}

/// A rule of the repository's own aggregates its matches over the whole
/// console, and shows one tip at the end from the totals.
#[test]
fn a_tip_rule_aggregates_its_matches() {
    let dir = workspace("config.axl", |_| {
        r#"load("@aspect//traits.axl", "BazelTrait")
load("@aspect//tips.axl", "TIP_SHOW_END", "TIP_SUGGESTION", "TipsTrait", "agg", "on_console_line", "tip_rule")

def config(ctx: ConfigContext):
    ctx.traits[BazelTrait].extra_flags.append("--scenario=console_noise")
    ctx.traits[TipsTrait].rules.append(tip_rule(
        id = "analysis",
        when = on_console_line(r"errors encountered while analyzing target '(?P<target>[^']+)'"),
        aggregate = {"targets": agg.unique("target")},
        show = TIP_SHOW_END,
        severity = TIP_SUGGESTION,
        title = "{{ count }} targets failed analysis",
        body = "{{ targets | join(', ') }}",
    ))
"#
        .to_string()
    });
    let log = dir.path().join("console.txt");
    let file = std::fs::File::create(&log).unwrap();
    let status = aspect(dir.path(), "build")
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .status()
        .expect("running aspect build");
    let stderr = std::fs::read_to_string(&log).unwrap();
    assert!(status.success(), "{stderr}");
    assert!(
        stderr.contains(
            "TIP[analysis] 💡 suggestion: 3 targets failed analysis\n  //:g0, //:g1, //:g2"
        ),
        "{stderr}"
    );
    assert_eq!(stderr.matches("TIP[analysis]").count(), 1, "{stderr}");
}

/// A built-in tip silenced by id is not shown; a user's drop applies.
#[test]
fn tips_and_drops_follow_their_settings() {
    let (_dir, stderr) = noisy_build(
        &[
            "--tips:silence=server-crash",
            "--console:drop=indicated that a canonical",
        ],
        &[],
    );
    assert!(!stderr.contains("The Bazel server crashed"), "{stderr}");
    assert!(!stderr.contains("canonical reproducible form"), "{stderr}");
    assert!(
        stderr.contains("INFO: Aspect hid 4 console lines"),
        "{stderr}"
    );
}

/// A layout that leaves out Aspect's own lines still shows the end of Bazel's
/// console: the summary and the tips are Bazel's console, not Aspect's lines.
#[test]
fn a_silent_layout_keeps_the_end_of_bazels_console() {
    let (_dir, stderr) = noisy_build(&["--console:template-preset=silent"], &[]);
    assert!(!stderr.contains("Running"), "no header: {stderr}");
    assert!(!stderr.contains("Passed"), "no verdict: {stderr}");
    assert!(
        stderr.contains("INFO: Aspect hid 2 console lines"),
        "{stderr}"
    );
    assert!(
        stderr.contains("TIP[server-crash] 🟡 warning: The Bazel server crashed"),
        "{stderr}"
    );
}

/// Turning the feature off sets aside a chosen layout, too.
#[test]
fn a_disabled_console_feature_takes_the_full_layout() {
    let (_dir, stderr) = noisy_build(
        &[
            "--console:enabled=false",
            "--console:template-preset=silent",
        ],
        &[],
    );
    assert!(stderr.contains("Running"), "the full header: {stderr}");
    assert!(stderr.contains("Passed"), "the full verdict: {stderr}");
    assert!(!stderr.contains("Aspect hid"), "{stderr}");
}

#[test]
fn a_disabled_console_feature_leaves_the_console_alone() {
    let (_dir, stderr) = noisy_build(&["--console:enabled=false"], &[]);
    assert_eq!(stderr.matches("Analysis failed").count(), 3, "{stderr}");
    assert!(stderr.contains("canonical reproducible form"), "{stderr}");
    assert!(!stderr.contains("Aspect hid"), "{stderr}");
    assert!(!stderr.contains("The Bazel server crashed"), "{stderr}");
}

/// Bazel writes a test's output to stdout and the header above it to
/// stderr. Merged into one file (`2>&1`), they must stay in that order, so
/// the capture takes stdout too.
#[test]
fn merged_output_keeps_bazels_order() {
    let dir = workspace("config.axl", |_| {
        r#"load("@aspect//traits.axl", "BazelTrait")

def config(ctx: ConfigContext):
    ctx.traits[BazelTrait].extra_flags.append("--scenario=merged_console")
"#
        .to_string()
    });
    let log = dir.path().join("merged.txt");
    let file = std::fs::File::create(&log).unwrap();
    let status = aspect(dir.path(), "build")
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .status()
        .expect("running aspect build");
    let merged = std::fs::read_to_string(&log).unwrap();
    assert!(status.success(), "{merged}");
    let at = |needle: &str| {
        merged
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} missing from:\n{merged}"))
    };
    let order = [
        at("INFO: From Testing //pkg:t:"),
        at("==== Test output for //pkg:t:"),
        at("failing output"),
        at("FAIL: //pkg:t"),
    ];
    assert!(order.is_sorted(), "out of order:\n{merged}");
}

/// Under `--output=json` Bazel's stdout goes to our stderr, into the
/// console's capture of it, so it keeps Bazel's order there and our stdout is
/// only the document.
#[test]
fn a_json_build_keeps_bazels_order_on_stderr() {
    let dir = workspace("config.axl", |_| {
        r#"load("@aspect//traits.axl", "BazelTrait")

def config(ctx: ConfigContext):
    ctx.traits[BazelTrait].extra_flags.append("--scenario=merged_console")
"#
        .to_string()
    });
    let (out, err) = (dir.path().join("out.json"), dir.path().join("err.txt"));
    let status = aspect(dir.path(), "build")
        .arg("--output=json")
        .stdout(std::fs::File::create(&out).unwrap())
        .stderr(std::fs::File::create(&err).unwrap())
        .status()
        .expect("running aspect build");
    let (out, err) = (
        std::fs::read_to_string(&out).unwrap(),
        std::fs::read_to_string(&err).unwrap(),
    );
    assert!(status.success(), "{err}");
    let doc: serde_json::Value = serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("{e}: stdout is not one document:\n{out}"));
    assert!(doc.get("schema_version").is_some(), "{out}");
    let at = |needle: &str| {
        err.find(needle)
            .unwrap_or_else(|| panic!("{needle:?} missing from:\n{err}"))
    };
    let order = [
        at("INFO: From Testing //pkg:t:"),
        at("==== Test output for //pkg:t:"),
        at("failing output"),
        at("FAIL: //pkg:t"),
    ];
    assert!(order.is_sorted(), "out of order:\n{err}");
    assert!(!err.contains('\x1b'), "no colour in a file:\n{err}");
}

/// `--output=ndjson` streams: every stdout line is a JSON object with a
/// `type`, the first the attempt, a failed action's line before the end,
/// and the document last, as `"type": "result"`.
#[test]
fn an_ndjson_build_streams_its_failures_then_the_document() {
    let dir = workspace("config.axl", |_| {
        r#"load("@aspect//traits.axl", "BazelTrait")

def config(ctx: ConfigContext):
    ctx.traits[BazelTrait].extra_flags.append("--scenario=action_and_test_logs")
"#
        .to_string()
    });
    let (out, err) = (dir.path().join("out.ndjson"), dir.path().join("err.txt"));
    let status = aspect(dir.path(), "build")
        .arg("--output=ndjson")
        .stdout(std::fs::File::create(&out).unwrap())
        .stderr(std::fs::File::create(&err).unwrap())
        .status()
        .expect("running aspect build");
    let (out, err) = (
        std::fs::read_to_string(&out).unwrap(),
        std::fs::read_to_string(&err).unwrap(),
    );
    assert!(!status.success(), "the scenario fails: {err}");
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: not a JSON line: {l}")))
        .collect();
    let types: Vec<&str> = lines.iter().map(|l| l["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["attempt", "action_failed", "result"], "{out}");
    assert_eq!(lines[1]["label"], "//pkg:broken", "{out}");
    assert!(
        lines[1]["stderr_path"]
            .as_str()
            .unwrap()
            .ends_with("stderr-1"),
        "{out}"
    );
    assert_eq!(lines[2]["status"], "failed", "{out}");
    assert_eq!(
        lines[2]["failures"]["actions"][0]["label"], "//pkg:broken",
        "{out}"
    );
}

/// An `--output=ndjson` stream a failure cuts short still ends: with Bazel
/// not there to start, its attempt line is followed by an `error` line, as
/// `--output=json` gets an error document.
#[test]
fn an_ndjson_stream_an_error_cuts_short_ends_with_the_error() {
    let dir = workspace("config.axl", |_| {
        "def config(ctx: ConfigContext):\n    pass\n".to_string()
    });
    for (output, want) in [
        ("ndjson", vec!["attempt", "error"]),
        ("json", vec!["task_error"]),
    ] {
        let out = aspect(dir.path(), "build")
            .arg(format!("--output={output}"))
            .env("BAZEL_REAL", dir.path().join("no-bazel"))
            .output()
            .expect("running aspect build");
        assert!(!out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<serde_json::Value> = stdout
            .lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
            .collect();
        let got: Vec<&str> = lines
            .iter()
            .map(|l| l["type"].as_str().or(l["error"].as_str()).unwrap())
            .collect();
        assert_eq!(got, want, "{output}: {stdout}");
    }
}

/// `gazelle --watch` refuses JSON output, and says so on stdout too.
#[test]
fn gazelle_watch_refuses_json_output_with_a_document() {
    let dir = workspace("config.axl", |_| {
        "def config(ctx: ConfigContext):\n    pass\n".to_string()
    });
    for output in ["json", "ndjson"] {
        let out = aspect(dir.path(), "gazelle")
            .args(["--watch", &format!("--output={output}")])
            .output()
            .expect("running aspect gazelle");
        assert!(!out.status.success());
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        assert_eq!(doc["error"], "usage", "{doc}");
        assert_eq!(doc.get("type").is_some(), output == "ndjson", "{doc}");
    }
}

/// A feature's `--<name>` disagreeing with its `--<name>:enabled` is a usage
/// error, and says so on stdout too.
#[test]
fn a_feature_flag_conflict_under_json_output_gets_a_usage_document() {
    let dir = workspace("config.axl", |_| {
        "def config(ctx: ConfigContext):\n    pass\n".to_string()
    });
    for output in ["json", "ndjson"] {
        let out = aspect(dir.path(), "build")
            .args([
                "--tips=false",
                "--tips:enabled=true",
                &format!("--output={output}"),
            ])
            .output()
            .expect("running aspect build");
        assert_eq!(out.status.code(), Some(2), "{out:?}");
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        assert_eq!(doc["error"], "usage", "{doc}");
        assert_eq!(
            doc["message"],
            "--tips=false conflicts with --tips:enabled=true; pass only one of them",
            "{doc}"
        );
        assert_eq!(doc.get("type").is_some(), output == "ndjson", "{doc}");
    }
}

/// A graceful cancel waits for the client, and the console with it: the
/// cleanup line basil prints after the interrupt is forwarded, under CI's stop
/// sequence, which sends two SIGINTs and leaves the client to finish on its
/// own. The task calls `ctx.bazel.build` itself, the shape most exposed to
/// that.
#[test]
fn a_cancelled_build_forwards_what_bazel_prints_while_it_winds_down() {
    cancel_a_build(false);
}

/// The same behind a wrapper script that runs Bazel as its child rather than
/// `exec` it (as bazelisk, or a `tools/bazel` that does not `exec`): the
/// real client, a grandchild holding the console's PTY, still gets the
/// interrupt and its last line still reaches the console.
#[test]
fn a_cancelled_build_behind_a_wrapper_still_forwards_the_clients_last_line() {
    cancel_a_build(true);
}

/// Cancel a build at basil's prompt and check its wind-down line was
/// forwarded; `wrapped` runs basil behind a shell wrapper.
fn cancel_a_build(wrapped: bool) {
    let dir = workspace("fixture.axl", |_| {
        r#"def _impl(ctx):
    processor = bazel.output.processor(display = "lines")
    build = ctx.bazel.build(
        flags = ["--scenario=stderr_after_interrupt"] + processor.flags,
        output = processor,
    )
    build.wait()
    return 0

cancelled = task(implementation = _impl)
"#
        .to_string()
    });
    let mut cmd = aspect(dir.path(), "cancelled");
    if wrapped {
        let wrapper = dir.path().join("bazel-wrapper.sh");
        // `; status=$?`, so the shell waits on basil rather than exec it.
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\n\"{}\" \"$@\"; status=$?; exit $status\n",
                basil_bin()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        cmd.env("BAZEL_REAL", &wrapper);
    }
    let mut child = cmd
        .env("CI", "true")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning aspect");
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut seen = String::new();
    while !seen.contains("basil: waiting for interrupt") {
        let mut line = String::new();
        if stderr.read_line(&mut line).expect("reading stderr") == 0 {
            panic!("aspect ended before basil waited:\n{seen}");
        }
        seen.push_str(&line);
    }
    // From the interrupt, not start-up, which a loaded machine slows.
    let interrupted = Instant::now();
    // SAFETY: signalling a child this test spawned and has not reaped.
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
        0
    );
    stderr.read_to_string(&mut seen).expect("draining stderr");
    child.wait().expect("waiting for aspect");
    assert!(
        seen.contains("INFO: cleaned up after the interrupt"),
        "the interrupted client's last line was lost:\n{seen}"
    );
    // Well inside the console's linger cap: the stream ended with the client.
    assert!(
        interrupted.elapsed() < Duration::from_secs(30),
        "took {:?}:\n{seen}",
        interrupted.elapsed()
    );
}

/// One build both captures its console and keeps its logs: the console's
/// raw log and the kept logs' indexes share its history folder.
#[test]
fn a_build_keeps_its_console_and_its_logs_in_one_folder() {
    let scratch = tempfile::tempdir().expect("temp dir");
    let dir = workspace("fixture.axl", |_| {
        format!(
            r#"def _impl(ctx):
    # A stage that can drop a line, so the raw console is kept.
    processor = bazel.output.processor(display = "lines", stages = [bazel.output.drop(["^never matches$"])])
    build = ctx.bazel.build(
        flags = ["--scenario=action_and_test_logs", "--basil-scratch={scratch}"] + processor.flags,
        output = processor,
        keep_action_logs = "failed",
        keep_test_logs = "all",
    )
    build.wait()
    return 0

both = task(implementation = _impl)
"#,
            scratch = scratch.path().display()
        )
    });
    let out = aspect(dir.path(), "both")
        .env("CI", "true")
        .output()
        .expect("running aspect");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let entries: Vec<_> = std::fs::read_dir(dir.path().join("history"))
        .expect("history")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("bazel/1").is_dir())
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    let invocation = entries[0].join("bazel/1");
    for name in ["console.log", "actions.json", "testlogs.json"] {
        assert!(
            invocation.join(name).is_file(),
            "{name} missing from {}",
            invocation.display()
        );
    }
    assert!(
        invocation
            .join("actions/pkg/broken/Genrule-1.stderr")
            .is_file()
    );
}
