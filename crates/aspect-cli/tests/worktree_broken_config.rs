//! `aspect worktree` is how you get a clean checkout to fix a broken workspace
//! in, so the workspace's own AXL failing to load must not block it. Every
//! subcommand runs anyway, with one warning on stderr naming each file that
//! failed and the settings that may be at their defaults, and stdout stays the
//! answer a caller parses. Any other task, a repository's own task in the
//! `worktree` group included, still fails as before, and a workspace that
//! loads warns about nothing.
//!
//! Each case is a scratch git clone with its own `HOME`, no system git config
//! and no ambient `GIT_*` variables, so the pool and any user-global
//! `~/.aspect/config.axl` are the test's own.

mod common;

use common::{aspect_cli, base_env};
use std::path::Path;
use std::process::{Command, Output};

/// The head of the degraded run's warning.
const WARNING: &str = "warning: `aspect worktree` is running without AXL that failed to load:";

/// A `config.axl` for each way of failing to load, with what the warning's
/// line for it must say.
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
];

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
        for (path, contents) in files {
            let path = match path.strip_prefix("~/") {
                Some(rest) => dir.path().join("home").join(rest),
                None => dir.path().join("repo").join(path),
            };
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        let ws = Workspace { dir };
        ws.git(&["init", "-q"]);
        ws.git(&["add", "-A"]);
        ws.git(&["commit", "-q", "-m", "init"]);
        ws
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// `cmd` run in the clone under the test's own git environment.
    fn isolated(&self, mut cmd: Command) -> Command {
        for (k, _) in std::env::vars_os() {
            if k.to_string_lossy().starts_with("GIT_") {
                cmd.env_remove(k);
            }
        }
        cmd.current_dir(self.path().join("repo"))
            .env("HOME", self.path().join("home"))
            .env("XDG_CONFIG_HOME", self.path().join("home/.config"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        cmd
    }

    fn git(&self, args: &[&str]) {
        let status = self
            .isolated(Command::new("git"))
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .status()
            .expect("running git");
        assert!(status.success(), "git {args:?} failed");
    }

    /// `aspect <args>` in the clone, under [`base_env`].
    fn aspect(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(aspect_cli());
        cmd.env_clear().envs(base_env());
        self.isolated(cmd)
            .env("ASPECT_CLI_CACHE", self.path().join("cache"))
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")))
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `out` succeeded with the degraded run's warning: a line naming `file` with
/// `headline`, and the settings that may be at their defaults. Nothing of it
/// reached stdout.
fn assert_degraded(out: &Output, file: &str, headline: &str) {
    let stderr = stderr(out);
    assert_eq!(out.status.code(), Some(0), "--- stderr ---\n{stderr}");
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
    assert!(
        stderr.contains("`abandoned_grace_hours`") && stderr.contains("`score_slot`"),
        "expected the settings at their defaults in:\n{stderr}"
    );
    assert!(
        !stdout(out).contains("failed to load") && !stdout(out).contains(headline),
        "the warning reached stdout:\n{}",
        stdout(out)
    );
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

/// `aspect worktree add HEAD --detach --output=json`, then `release` of the
/// slot it took, each checked by `check`.
fn add_and_release(ws: &Workspace, check: impl Fn(&Output)) {
    let added = ws.aspect(&["worktree", "add", "HEAD", "--detach", "--output=json"]);
    check(&added);
    let doc: serde_json::Value = serde_json::from_slice(&added.stdout)
        .unwrap_or_else(|e| panic!("add's stdout is not JSON ({e}):\n{}", stdout(&added)));
    let slot = doc["slot"].as_str().expect("a slot id").to_owned();
    assert!(
        Path::new(doc["path"].as_str().expect("a path")).is_dir(),
        "{doc}"
    );

    let released = ws.aspect(&["worktree", "release", &slot, "--output=json"]);
    check(&released);
    let doc: serde_json::Value = serde_json::from_slice(&released.stdout)
        .unwrap_or_else(|e| panic!("release's stdout is not JSON ({e}):\n{}", stdout(&released)));
    assert_eq!(doc["slot"], slot.as_str(), "{doc}");
}

#[test]
fn every_worktree_command_runs_on_a_config_that_does_not_load() {
    for (case, config, headline) in BROKEN_CONFIGS {
        eprintln!("case: {case}");
        let ws = Workspace::new(&[(".aspect/config.axl", config)]);
        let file = ".aspect/config.axl";
        assert_degraded(&ws.aspect(&["worktree", "inspect"]), file, headline);
        assert_degraded(&ws.aspect(&["worktree", "list"]), file, headline);
        add_and_release(&ws, |out| assert_degraded(out, file, headline));
    }
}

/// A task file and the user-global config are the workspace's AXL as much as
/// the repository's `config.axl` is.
#[test]
fn a_broken_task_file_or_user_config_does_not_block_it_either() {
    for (file, path) in [
        ("broken.axl", ".aspect/broken.axl"),
        (".aspect/config.axl", "~/.aspect/config.axl"),
    ] {
        let ws = Workspace::new(&[(path, "this is not valid starlark(\n")]);
        assert_degraded(&ws.aspect(&["worktree", "inspect"]), file, "Parse error");
    }
}

/// Only the built-in `aspect worktree` tasks run degraded: another built-in
/// task, the surface `describe` reports, and a repository's own task in the
/// `worktree` group, which may need what failed, still fail on it.
#[test]
fn every_other_command_still_fails_on_a_config_that_does_not_load() {
    let (_, config, headline) = BROKEN_CONFIGS[0];
    let mine = "def _impl(ctx: TaskContext) -> int:\n    return 0\n\n\
                mine = task(group = [\"worktree\"], summary = \"Mine.\", implementation = _impl)\n";
    let ws = Workspace::new(&[(".aspect/config.axl", config), (".aspect/mine.axl", mine)]);
    for args in [
        &["history", "list"][..],
        &["describe"][..],
        &["worktree", "mine"][..],
    ] {
        let out = ws.aspect(args);
        let stderr = stderr(&out);
        assert_eq!(out.status.code(), Some(1), "aspect {args:?}:\n{stderr}");
        assert!(
            stderr.contains(&format!("error: {headline}")),
            "aspect {args:?} must fail on the config's error:\n{stderr}"
        );
        assert!(
            !stderr.contains(WARNING),
            "aspect {args:?} ran degraded:\n{stderr}"
        );
    }
}

/// A workspace that loads, its config setting a worktree option, runs as it
/// always has: no warning, and the subcommands succeed.
#[test]
fn a_config_that_loads_runs_as_before() {
    let config = "load(\"@aspect//feature/worktree_pool.axl\", \"WorktreePool\")\n\n\
                  def config(ctx):\n    ctx.features[WorktreePool].args.abandoned_grace_hours = 48\n";
    let ws = Workspace::new(&[(".aspect/config.axl", config)]);
    assert_clean(&ws.aspect(&["worktree", "inspect"]));
    assert_clean(&ws.aspect(&["worktree", "list"]));
    add_and_release(&ws, assert_clean);
}
