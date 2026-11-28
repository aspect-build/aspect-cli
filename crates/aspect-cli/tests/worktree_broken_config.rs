//! `aspect worktree` is how you get a clean checkout to fix a broken workspace
//! in, so the workspace's own AXL failing to load must not block it. `add`,
//! `release`, `list`, `path` and `inspect` run anyway, with one warning on
//! stderr naming each file that failed and the settings that may be at their
//! defaults, and stdout stays the answer a caller parses. Such a run takes no
//! stopped session's slot back and runs no feature but the ones those tasks
//! read, since the config that did not load may protect the one and turn off
//! the others. A usage error on one of them is Clap's to report. `prune`, any
//! other task, and a repository's own task in the `worktree` group still fail
//! as before, and a workspace that loads warns about nothing.
//!
//! Each case is a scratch git clone with its own `HOME` (see
//! [`common::isolated`]), so the pool and any user-global
//! `~/.aspect/config.axl` are the test's own.

mod common;

use common::{aspect_cli, base_env, git, isolated};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime};

/// The head of the degraded run's warning.
const WARNING: &str = "warning: `aspect worktree` runs without the AXL that failed to load:";

/// The warning's line for the settings a config would apply.
const SETTINGS: &str = "Settings these files would have applied to it may be at their defaults: \
                        the `WorktreePool` options (`max_slots`, `abandoned_grace_hours`), the \
                        `Worktrees` `score_slot` hooks";

/// The warning's line for the features that are off once a config failed.
const FEATURES: &str =
    "Only the `WorktreePool` feature runs, so any hook another feature adds is off.";

/// The end of the warning's line naming the feature files that failed while
/// every config loaded.
const MISSING: &str = "define do not run, so any hook they add is off.";

/// What kind of file failed, which decides the warning's lines past the files.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Lost {
    /// Only task files: no line.
    Tasks,
    /// A feature file, and no config: the line naming the feature files.
    Feature,
    /// A config: the settings line and the features line.
    Config,
}

/// A `config.axl` for each way of failing, with what the warning's line for
/// it must say.
const BROKEN_CONFIGS: &[(&str, &str, &str)] = &[
    (
        "a missing symbol",
        "load(\"@aspect//traits.axl\", \"NoSuchTrait\")\n\ndef config(ctx):\n    pass\n",
        "Module has no symbol `NoSuchTrait`",
    ),
    (
        "a syntax error",
        "def config(ctx)\n    pass\n",
        "Parse error",
    ),
    (
        "a failing load()",
        "load(\"./no_such_file.axl\", \"anything\")\n\ndef config(ctx):\n    pass\n",
        "no_such_file.axl",
    ),
    (
        "a fail() in config()",
        "def config(ctx):\n    fail(\"boom at config time\\nsecond line\")\n",
        "boom at config time",
    ),
];

/// A task file that cannot parse.
const UNPARSABLE: &str = "this is not valid starlark(\n";

/// A scratch clone, its `HOME` and the CLI's cache, removed on drop.
struct Workspace {
    dir: tempfile::TempDir,
}

impl Workspace {
    /// A clone holding `files` in one commit, each a `(path, contents)` pair
    /// relative to the clone. A path under `~/` goes to the run's `HOME`
    /// instead.
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("repo")).unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        std::fs::write(dir.path().join("repo/MODULE.aspect"), "").unwrap();
        let ws = Workspace { dir };
        for (path, contents) in files {
            ws.write(path, contents);
        }
        git(ws.path(), &["init", "-q"]);
        git(ws.path(), &["add", "-A"]);
        git(ws.path(), &["commit", "-q", "-m", "init"]);
        ws
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Write `contents` to `path`, as [`Workspace::new`] places it.
    fn write(&self, path: &str, contents: &str) {
        let path = match path.strip_prefix("~/") {
            Some(rest) => self.path().join("home").join(rest),
            None => self.path().join("repo").join(path),
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// `aspect <args>` in the clone, under [`base_env`] plus `vars`.
    fn run(&self, args: &[&str], vars: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(aspect_cli());
        cmd.env_clear().envs(base_env());
        isolated(cmd, self.path())
            .env("ASPECT_CLI_CACHE", self.path().join("cache"))
            .envs(vars.iter().copied())
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")))
    }

    fn aspect(&self, args: &[&str]) -> Output {
        self.run(args, &[])
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `out` carries the degraded run's single warning, with a line naming `file`
/// and `headline` and the lines `lost` calls for. Nothing of it reached
/// stdout.
fn assert_warned(out: &Output, file: &str, headline: &str, lost: Lost) {
    let stderr = stderr(out);
    assert_eq!(
        stderr.matches("warning:").count(),
        1,
        "expected exactly one warning:\n{stderr}"
    );
    assert!(
        stderr.contains(WARNING),
        "no degraded warning in:\n{stderr}"
    );
    assert!(
        stderr
            .lines()
            .any(|l| l.starts_with("  ") && l.contains(file) && l.contains(headline)),
        "expected a line naming {file} with {headline:?} in:\n{stderr}"
    );
    assert_eq!(
        stderr.contains(SETTINGS),
        lost == Lost::Config,
        "the settings line, for {lost:?}, in:\n{stderr}"
    );
    assert_eq!(
        stderr.contains(FEATURES),
        lost == Lost::Config,
        "the features line, for {lost:?}, in:\n{stderr}"
    );
    assert_eq!(
        stderr
            .lines()
            .any(|l| l.starts_with("  The features ") && l.contains(file) && l.ends_with(MISSING)),
        lost == Lost::Feature,
        "the line naming {file} as a feature file, for {lost:?}, in:\n{stderr}"
    );
    assert!(
        stderr.contains("`ASPECT_DEBUG=1` prints each error in full."),
        "{stderr}"
    );
    assert!(
        !stdout(out).contains("failed to load") && !stdout(out).contains(headline),
        "the warning reached stdout:\n{}",
        stdout(out)
    );
}

/// `out` succeeded with the degraded run's warning (see [`assert_warned`]).
fn assert_degraded(out: &Output, file: &str, headline: &str) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "--- stderr ---\n{}",
        stderr(out)
    );
    assert_warned(out, file, headline, Lost::Config);
}

/// `out` succeeded and said nothing about a workspace that did not load.
fn assert_clean(out: &Output) {
    let stderr = stderr(out);
    assert_eq!(out.status.code(), Some(0), "--- stderr ---\n{stderr}");
    assert!(
        !stderr.contains("warning:"),
        "unexpected warning in:\n{stderr}"
    );
}

/// `out` failed on the config's own error, as an ordinary task does.
fn assert_fails_on(out: &Output, headline: &str) {
    let stderr = stderr(out);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&format!("error: {headline}")),
        "expected the config's error in:\n{stderr}"
    );
    assert!(!stderr.contains(WARNING), "ran degraded:\n{stderr}");
}

/// The JSON document on `out`'s stdout.
fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}", stdout(out)))
}

/// `add HEAD --detach --output=path`, `path` and `release` of the slot it
/// took, each checked by `check`. Stdout carries the slot's directory and
/// nothing else.
fn add_path_release(ws: &Workspace, check: impl Fn(&Output)) {
    let added = ws.aspect(&["worktree", "add", "HEAD", "--detach", "--output=path"]);
    check(&added);
    let printed = stdout(&added);
    let dir = printed
        .strip_suffix('\n')
        .unwrap_or_else(|| panic!("--output=path is one line: {printed:?}"));
    assert!(
        !dir.contains('\n') && Path::new(dir).is_dir(),
        "--output=path printed {printed:?}"
    );

    let path = ws.aspect(&["worktree", "path", dir]);
    check(&path);
    assert_eq!(stdout(&path), printed, "`path` prints the same directory");

    let released = ws.aspect(&["worktree", "release", dir, "--output=json"]);
    check(&released);
    assert_eq!(json(&released)["path"], dir, "{}", stdout(&released));
}

#[test]
fn the_degraded_worktree_commands_run_on_a_config_that_does_not_load() {
    for (case, config, headline) in BROKEN_CONFIGS {
        eprintln!("case: {case}");
        let ws = Workspace::new(&[(".aspect/config.axl", config)]);
        let file = ".aspect/config.axl";
        assert_degraded(&ws.aspect(&["worktree", "inspect"]), file, headline);
        assert_degraded(&ws.aspect(&["worktree", "list"]), file, headline);
        add_path_release(&ws, |out| assert_degraded(out, file, headline));
    }
}

/// A task file, a feature file and the user-global config are the workspace's
/// AXL as much as the repository's `config.axl` is. A task file sets nothing
/// and a feature file only adds hooks, so neither brings the settings line;
/// the feature file is named as one whose hooks are off.
#[test]
fn a_broken_task_feature_or_user_config_does_not_block_it_either() {
    for (path, file, lost) in [
        (".aspect/broken.axl", ".aspect/broken.axl", Lost::Tasks),
        ("~/.aspect/config.axl", ".aspect/config.axl", Lost::Config),
        ("tools/feature.axl", "tools/feature.axl", Lost::Feature),
    ] {
        let ws = Workspace::new(&[(path, UNPARSABLE)]);
        if path.starts_with("tools/") {
            ws.write(
                "MODULE.aspect",
                "use_feature(\"tools/feature.axl\", \"Mine\")\n",
            );
        }
        let out = ws.aspect(&["worktree", "inspect"]);
        assert_eq!(out.status.code(), Some(0), "{path}:\n{}", stderr(&out));
        assert_warned(&out, file, "Parse error", lost);
    }
}

/// Several broken files share the one warning, task and feature files ahead of
/// configs, each with its own error; `ASPECT_DEBUG` prints each in full.
#[test]
fn every_failure_is_one_line_of_one_warning_and_aspect_debug_prints_it_in_full() {
    let config = BROKEN_CONFIGS[0].1;
    let ws = Workspace::new(&[
        (".aspect/broken.axl", UNPARSABLE),
        (".aspect/broken2.axl", "load(\"./nope.axl\", \"x\")\n"),
        (".aspect/config.axl", config),
    ]);
    let out = ws.aspect(&["worktree", "inspect"]);
    assert_degraded(&out, ".aspect/config.axl", BROKEN_CONFIGS[0].2);
    let stderr = stderr(&out);
    let line_of = |file: &str| {
        stderr
            .lines()
            .position(|l| l.starts_with("  ") && l.contains(file))
            .unwrap_or_else(|| panic!("no line for {file} in:\n{stderr}"))
    };
    let config_line = line_of(".aspect/config.axl:");
    for script in [".aspect/broken.axl:", ".aspect/broken2.axl:"] {
        assert!(
            line_of(script) < config_line,
            "{script} after the config:\n{stderr}"
        );
    }
    let excerpts = [
        "this is not valid starlark(",
        "load(\"@aspect//traits.axl\", \"NoSuchTrait\")",
    ];
    for excerpt in excerpts {
        assert!(
            !stderr.contains(excerpt),
            "a full error without ASPECT_DEBUG:\n{stderr}"
        );
    }

    let debug = ws.run(&["worktree", "inspect"], &[("ASPECT_DEBUG", "1")]);
    assert_eq!(debug.status.code(), Some(0), "{}", self::stderr(&debug));
    let full = self::stderr(&debug);
    for excerpt in excerpts {
        assert!(
            full.contains(excerpt),
            "ASPECT_DEBUG=1 left out {excerpt:?}:\n{full}"
        );
    }
    assert!(full.contains("nope.axl"), "{full}");
}

/// Only the listed built-in tasks run degraded: `prune`, whose default
/// `max_idle_days` may delete slots a config keeps, another built-in task,
/// the surface `describe` reports, and a repository's own task in the
/// `worktree` group, which may need what failed, still fail on it.
#[test]
fn every_other_command_still_fails_on_a_config_that_does_not_load() {
    let (_, config, headline) = BROKEN_CONFIGS[0];
    let mine = "def _impl(ctx: TaskContext) -> int:\n    return 0\n\n\
                mine = task(group = [\"worktree\"], summary = \"Mine.\", implementation = _impl)\n";
    let ws = Workspace::new(&[(".aspect/config.axl", config), (".aspect/mine.axl", mine)]);
    for args in [
        &["worktree", "prune", "--dry-run"][..],
        &["history", "list"][..],
        &["describe"][..],
        &["worktree", "mine"][..],
    ] {
        eprintln!("aspect {args:?}");
        assert_fails_on(&ws.aspect(args), headline);
    }
}

/// A command line Clap rejects for a degraded command gets Clap's error and
/// exit code, after the warning, rather than the config's error. One for a
/// task that cannot run degraded, or naming no task at all, still gets the
/// load failure: the task it names may be one whose file did not load.
#[test]
fn a_usage_error_on_a_degraded_command_is_reported_as_one() {
    let (_, config, headline) = BROKEN_CONFIGS[0];
    let ws = Workspace::new(&[(".aspect/config.axl", config)]);
    for (args, clap_says) in [
        (&["worktree", "add", "x", "--detatch"][..], "--detatch"),
        (&["worktree", "path"][..], "required"),
    ] {
        let out = ws.aspect(args);
        let stderr = stderr(&out);
        assert_eq!(out.status.code(), Some(2), "aspect {args:?}:\n{stderr}");
        assert_warned(&out, ".aspect/config.axl", headline, Lost::Config);
        assert!(
            stderr
                .lines()
                .any(|l| l.starts_with("error: ") && l.contains(clap_says)),
            "aspect {args:?}: expected Clap's error naming {clap_says:?} in:\n{stderr}"
        );
        assert!(
            !stderr.contains(&format!("error: {headline}")),
            "aspect {args:?} failed on the config:\n{stderr}"
        );
    }
    assert_fails_on(&ws.aspect(&["worktree", "prune", "--bogus"]), headline);
    assert_fails_on(&ws.aspect(&["worktree", "bogus"]), headline);

    let sync = "load(\"@aspect//traits.axl\", \"NoSuchTrait\")\n\n\
                def _impl(ctx: TaskContext) -> int:\n    return 0\n\n\
                sync = task(group = [\"worktree\"], summary = \"Mine.\", implementation = _impl)\n";
    let ws = Workspace::new(&[(".aspect/mytask.axl", sync)]);
    assert_fails_on(&ws.aspect(&["worktree", "sync"]), headline);

    // A flag a broken feature file would have added is unknown to Clap
    // because the file failed, so the load failure is the error.
    let feature = "load(\"@aspect//traits.axl\", \"NoSuchTrait\")\n\n\
                   def _impl(ctx: FeatureContext):\n    pass\n\n\
                   Mine = feature(implementation = _impl, args = {\"opt\": args.string(default = \"d\")})\n";
    let ws = Workspace::new(&[
        (
            "MODULE.aspect",
            "use_feature(\"tools/feat.axl\", \"Mine\")\n",
        ),
        ("tools/feat.axl", feature),
    ]);
    let out = ws.aspect(&["worktree", "list", "--mine:opt=x"]);
    assert_fails_on(&out, headline);
    assert!(
        !stderr(&out).contains("unexpected argument"),
        "{}",
        stderr(&out)
    );
}

/// With only a task file broken the configs ran in full, so the run is not
/// degraded: a repository's feature still runs (its `score_slot` hook scores
/// the free slot), and a stopped session's slot is judged by the grace period
/// as ever, here released once that is over.
#[test]
fn a_broken_task_file_alone_leaves_settings_and_features_alone() {
    let scorer = "load(\"@aspect//traits.axl\", \"Worktrees\")\n\n\
                  def _score(ctx, slot, want):\n    print(\"scorer ran\")\n    return 1\n\n\
                  def _impl(ctx: FeatureContext):\n    ctx.traits[Worktrees].score_slot.append(_score)\n\n\
                  Scorer = feature(implementation = _impl)\n";
    let ws = Workspace::new(&[
        (
            "MODULE.aspect",
            "use_feature(\"tools/scorer.axl\", \"Scorer\")\n",
        ),
        ("tools/scorer.axl", scorer),
        (".aspect/config.axl", "def config(ctx):\n    pass\n"),
    ]);
    let taken = ws.aspect(&["worktree", "add", "feat", "--create", "--output=json"]);
    assert_clean(&taken);
    let slot = json(&taken)["slot"].as_str().expect("a slot").to_owned();
    let slot_dir = PathBuf::from(json(&taken)["path"].as_str().expect("a path"));
    let freed = ws.aspect(&["worktree", "add", "spare", "--create", "--output=path"]);
    assert_clean(&freed);
    let spare = stdout(&freed).trim_end().to_owned();
    assert_clean(&ws.aspect(&["worktree", "release", &spare]));
    stop_and_age(&ws, &slot_dir, &slot, Duration::from_secs(30 * 3600));

    ws.write(".aspect/broken.axl", UNPARSABLE);
    let added = ws.aspect(&["worktree", "add", "other", "--create", "--output=json"]);
    assert_eq!(added.status.code(), Some(0), "{}", stderr(&added));
    assert_warned(&added, ".aspect/broken.axl", "Parse error", Lost::Tasks);
    assert!(
        stderr(&added).contains("scorer ran"),
        "the feature did not run:\n{}",
        stderr(&added)
    );

    let released = ws.aspect(&["worktree", "release", "feat", "--output=json"]);
    assert_eq!(released.status.code(), Some(0), "{}", stderr(&released));
    assert!(
        !stderr(&released).contains("did not load"),
        "{}",
        stderr(&released)
    );
    assert!(!slot_dir.exists(), "the stopped session's slot was kept");
}

/// A feature file alone leaves every config loaded, so the grace period a
/// config sets is known: its 2 hours let `release` end a stopped session's
/// lease idle for 3, which the 24-hour default would keep. The configs decide
/// which features run, so a healthy repository feature still runs, its
/// `score_slot` hook included. The warning
/// lists no setting, and names the broken file as a feature file whose hooks
/// are off.
#[test]
fn a_broken_feature_file_alone_keeps_the_configured_grace() {
    let config = "load(\"@aspect//feature/worktree_pool.axl\", \"WorktreePool\")\n\n\
                  def config(ctx):\n    ctx.features[WorktreePool].args.abandoned_grace_hours = 2\n";
    let scorer = "load(\"@aspect//traits.axl\", \"Worktrees\")\n\n\
                  def _score(ctx, slot, want):\n    print(\"scorer ran\")\n    return 1\n\n\
                  def _impl(ctx: FeatureContext):\n    ctx.traits[Worktrees].score_slot.append(_score)\n\n\
                  Scorer = feature(implementation = _impl)\n";
    let ws = Workspace::new(&[
        (
            "MODULE.aspect",
            "use_feature(\"tools/scorer.axl\", \"Scorer\")\n",
        ),
        ("tools/scorer.axl", scorer),
        (".aspect/config.axl", config),
    ]);
    let taken = ws.aspect(&["worktree", "add", "feat", "--create", "--output=json"]);
    assert_clean(&taken);
    let slot = json(&taken)["slot"].as_str().expect("a slot").to_owned();
    let slot_dir = PathBuf::from(json(&taken)["path"].as_str().expect("a path"));
    let spare = ws.aspect(&["worktree", "add", "spare", "--create", "--output=path"]);
    assert_clean(&spare);
    assert_clean(&ws.aspect(&["worktree", "release", stdout(&spare).trim_end()]));
    stop_and_age(&ws, &slot_dir, &slot, Duration::from_secs(3 * 3600));

    ws.write("tools/broken.axl", UNPARSABLE);
    ws.write(
        "MODULE.aspect",
        "use_feature(\"tools/scorer.axl\", \"Scorer\")\n\
         use_feature(\"tools/broken.axl\", \"Broken\")\n",
    );
    let added = ws.aspect(&["worktree", "add", "other", "--create", "--output=json"]);
    assert_eq!(added.status.code(), Some(0), "{}", stderr(&added));
    assert_warned(&added, "tools/broken.axl", "Parse error", Lost::Feature);
    assert!(
        stderr(&added).contains("scorer ran"),
        "the healthy feature did not run:\n{}",
        stderr(&added)
    );

    let released = ws.aspect(&["worktree", "release", "feat", "--output=json"]);
    assert_eq!(
        released.status.code(),
        Some(0),
        "{}{}",
        stderr(&released),
        stdout(&released)
    );
    assert_warned(&released, "tools/broken.axl", "Parse error", Lost::Feature);
    assert!(!slot_dir.exists(), "the stopped session's slot was kept");
}

/// A config that turns a feature off and then fails to load leaves it on, so a
/// degraded run must not run it: here no span reaches the console exporter the
/// environment asks for, as one does when the same config loads and leaves
/// `Telemetry` on.
#[test]
fn a_degraded_run_runs_no_feature_its_tasks_do_not_read() {
    let exporter = [("OTEL_TRACES_EXPORTER", "console")];
    let spans = |out: &Output| {
        stderr(out)
            .lines()
            .filter(|l| l.starts_with("span "))
            .count()
    };

    let ws = Workspace::new(&[(
        ".aspect/config.axl",
        "load(\"@aspect//feature/telemetry.axl\", \"Telemetry\", \"FromTheFuture\")\n\n\
         def config(ctx):\n    ctx.features[Telemetry].enabled = False\n",
    )]);
    let out = ws.run(&["worktree", "inspect"], &exporter);
    assert_degraded(&out, ".aspect/config.axl", "FromTheFuture");
    assert_eq!(
        spans(&out),
        0,
        "a degraded run exported spans:\n{}",
        stderr(&out)
    );

    // A flag for a feature that does not run is named as ignored, in the one
    // warning. One turning a feature off is not: the run does what it asks.
    // Nor is the console's, which the runtime reads before the features run.
    for flag in [
        "--telemetry=false",
        "--telemetry:enabled=false",
        "--console:template-preset=compact",
        "--console:verbose=debug",
    ] {
        let out = ws.run(&["worktree", "inspect", flag], &exporter);
        assert_degraded(&out, ".aspect/config.axl", "FromTheFuture");
        assert!(
            !stderr(&out).contains("This run ignores"),
            "{flag} is not ignored:\n{}",
            stderr(&out)
        );
    }
    let out = ws.run(
        &["worktree", "inspect", "--telemetry:enabled=true"],
        &exporter,
    );
    assert_degraded(&out, ".aspect/config.axl", "FromTheFuture");
    assert!(
        stderr(&out).contains("  This run ignores --telemetry:enabled: "),
        "{}",
        stderr(&out)
    );
    assert_eq!(spans(&out), 0, "{}", stderr(&out));

    ws.write(".aspect/config.axl", "def config(ctx):\n    pass\n");
    let out = ws.run(&["worktree", "inspect"], &exporter);
    assert_clean(&out);
    assert!(spans(&out) > 0, "the exporter is not on:\n{}", stderr(&out));
}

/// The `WorktreePool` grace period a config sets is not known while that
/// config does not load, so a degraded run takes no stopped session's slot
/// back. Two clean leases idle for 30 hours, past the 24-hour default and
/// inside the config's 72, stay through a degraded `add` of another branch, an
/// `add` of the stopped session's own branch, and a `release` of it, each
/// reporting the grace left as unknown; `release --force=all` still ends one.
/// A grace period given on the command line beats any config, so with one the
/// other lease is taken back, which also shows the leases were ones a run that
/// knows the grace would take.
#[test]
fn a_degraded_run_takes_no_stopped_sessions_slot_back() {
    let broken = "load(\"@aspect//feature/worktree_pool.axl\", \"WorktreePool\", \"FromTheFuture\")\n\n\
                  def config(ctx):\n    ctx.features[WorktreePool].args.abandoned_grace_hours = 72\n";
    let ws = Workspace::new(&[(".aspect/config.axl", "def config(ctx):\n    pass\n")]);
    // Both are taken before either is aged, or the second `add` would take
    // the first back.
    let stopped: Vec<(String, PathBuf)> = ["feat", "feat2"]
        .iter()
        .map(|branch| {
            let taken = ws.aspect(&["worktree", "add", branch, "--create", "--output=json"]);
            assert_clean(&taken);
            let doc = json(&taken);
            (
                doc["slot"].as_str().expect("a slot").to_owned(),
                PathBuf::from(doc["path"].as_str().expect("a path")),
            )
        })
        .collect();
    for (slot, dir) in &stopped {
        stop_and_age(&ws, dir, slot, Duration::from_secs(30 * 3600));
    }
    let degraded =
        |out: &Output| assert_warned(out, ".aspect/config.axl", "FromTheFuture", Lost::Config);
    ws.write(".aspect/config.axl", broken);

    let added = ws.aspect(&["worktree", "add", "other", "--create", "--output=json"]);
    assert_eq!(added.status.code(), Some(0), "{}", stderr(&added));
    degraded(&added);
    let doc = json(&added);
    assert!(
        stopped.iter().all(|(slot, _)| doc["slot"] != slot.as_str()),
        "took a stopped lease: {doc}"
    );
    assert!(!stdout(&added).contains("lease_reclaimed"), "{doc}");

    let own = ws.aspect(&["worktree", "add", "feat", "--output=json"]);
    assert_eq!(own.status.code(), Some(1), "{}", stderr(&own));
    degraded(&own);
    let doc = json(&own);
    assert_eq!(doc["error"], "branch_in_use", "{doc}");
    assert_eq!(doc["process"], "stopped", "{doc}");
    assert!(doc["grace_left_ms"].is_null(), "{doc}");

    let released = ws.aspect(&["worktree", "release", "feat", "--output=json"]);
    assert_eq!(released.status.code(), Some(1), "{}", stderr(&released));
    degraded(&released);
    let doc = json(&released);
    assert_eq!(doc["error"], "held_by_another_session", "{doc}");
    assert!(doc["grace_left_ms"].is_null(), "{doc}");
    assert!(
        stopped.iter().all(|(_, dir)| dir.is_dir()),
        "a stopped session's checkout went"
    );

    let forced = ws.aspect(&["worktree", "release", "feat", "--force=all"]);
    assert_eq!(forced.status.code(), Some(0), "{}", stderr(&forced));
    assert!(!stopped[0].1.exists(), "--force=all kept the checkout");

    let given = ws.aspect(&[
        "worktree",
        "add",
        "feat2",
        "--worktree-pool:abandoned-grace-hours=1",
        "--output=json",
    ]);
    assert_eq!(given.status.code(), Some(0), "{}", stderr(&given));
    degraded(&given);
    let doc = json(&given);
    assert!(
        doc["warnings"].as_array().is_some_and(|w| w
            .iter()
            .any(|w| w["code"] == "lease_reclaimed" && w["branch"] == "feat2")),
        "the given grace did not take feat2 back: {doc}"
    );
}

/// Make slot `slot`'s lease look like a session that stopped `idle` ago: a
/// holder whose process has exited, and nothing in the slot newer than that.
fn stop_and_age(ws: &Workspace, slot_dir: &Path, slot: &str, idle: Duration) {
    let mut exited = Command::new("true").spawn().expect("spawning `true`");
    let pid = exited.id();
    exited.wait().expect("waiting for `true`");
    let then = SystemTime::now() - idle;
    let then_ms = then
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let registry = slot_dir.parent().expect("a pool").join("registry.json");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&registry).expect("the registry")).unwrap();
    let lease = &mut doc["slots"][slot]["lease"];
    assert!(lease.is_object(), "no lease on {slot}: {doc}");
    lease["agent"] = serde_json::json!({
        "kind": "claude-code",
        "id": "a-stopped-session",
        "pid": pid,
        "pid_start": "Thu Jan  1 00:00:00 2026",
    });
    lease["since"] = serde_json::json!(then_ms);
    std::fs::write(&registry, serde_json::to_string_pretty(&doc).unwrap()).unwrap();

    let git_dir = git(
        ws.path(),
        &[
            "-C",
            &slot_dir.to_string_lossy(),
            "rev-parse",
            "--absolute-git-dir",
        ],
    );
    for marker in ["index", "HEAD", "logs/HEAD"] {
        let path = Path::new(git_dir.trim()).join(marker);
        if let Ok(file) = std::fs::File::options().write(true).open(&path) {
            file.set_modified(then).expect("setting a marker's mtime");
        }
    }
}
