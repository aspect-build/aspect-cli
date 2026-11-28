//! A task's record in the task history names the git commit and branch it
//! ran on, and whether tracked files had changed.
//!
//! Runs the real CLI on a fixture task in a scratch repository, git there
//! under the test's own environment: no ambient `GIT_*` variable, no system
//! or global config, its own `HOME`, and no repository found above the
//! scratch folder. The task sleeps a second, so `git status` has answered
//! by its end, which takes the dirty flag only once it has come.

mod common;

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

use axl_runtime::engine::task_record::{self, Git};
use common::{aspect_cli, base_env};

const NAP: &str = r#"load("@std//time.axl", "sleep")

def _impl(ctx):
    sleep(1000)
    return 0

nap = task(summary = "Test fixture.", implementation = _impl)
"#;

/// [`base_env`] with git confined to `root`, and the task history in it.
fn env(root: &Path) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = base_env()
        .into_iter()
        .filter(|(k, _)| !k.starts_with("GIT_"))
        .collect();
    let path = |rel: &str| root.join(rel).to_string_lossy().into_owned();
    env.insert("HOME".into(), path("home"));
    env.insert("XDG_CONFIG_HOME".into(), path("home/.config"));
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    env.insert("GIT_CONFIG_GLOBAL".into(), path("home/.gitconfig"));
    env.insert("GIT_CEILING_DIRECTORIES".into(), path(""));
    env.insert("ASPECT_HISTORY_DIR".into(), path("history"));
    env.insert(
        "ASPECT_CREDENTIALS_FILE".into(),
        path("home/credentials.json"),
    );
    env.insert("DO_NOT_TRACK".into(), "1".into());
    env
}

/// git with `args` in `root/repo`, under [`env`], and its stdout.
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root.join("repo"))
        .env_clear()
        .envs(env(root))
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("running git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn a_task_records_the_commit_branch_and_dirty_flag_it_ran_on() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".aspect")).expect("repo");
    std::fs::create_dir(root.join("home")).expect("home");
    std::fs::write(repo.join("MODULE.bazel"), "").expect("MODULE.bazel");
    std::fs::write(repo.join(".aspect").join("nap.axl"), NAP).expect("fixture");
    git(root, &["init", "-q", "-b", "topic"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "one"]);
    let commit = git(root, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("MODULE.bazel"), "# changed\n").expect("change");

    let output = Command::new(aspect_cli())
        .arg("nap")
        .current_dir(&repo)
        .env_clear()
        .envs(env(root))
        .stdin(Stdio::null())
        .output()
        .expect("running the CLI");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");

    let runs: Vec<_> = std::fs::read_dir(root.join("history"))
        .expect("the history")
        .map(|e| e.expect("entry").path())
        .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .collect();
    assert_eq!(runs.len(), 1, "one run: {runs:?}");
    let record = task_record::read(&runs[0].join(task_record::FILE_NAME)).expect("its record");
    assert_eq!(
        record.git,
        Some(Git {
            commit,
            branch: Some("topic".into()),
            dirty: Some(true),
        }),
        "{stderr}"
    );
}
