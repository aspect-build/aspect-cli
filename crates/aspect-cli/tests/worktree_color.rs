//! `aspect worktree list` styles its report on a piped stderr whenever the
//! CLI's color rule allows it, as the console lines around it do.
//!
//! Spawns the real CLI in a scratch git clone with its own `HOME`, so the pool
//! it lists is the test's own.

mod common;

use common::{aspect_cli, base_env};
use std::path::Path;
use std::process::Command;

/// The dimmed column header of the `taken` table.
const DIM_HEADER: &[u8] = b"\x1b[2m  slot";

fn aspect(dir: &Path, args: &[&str], vars: &[(&str, &str)]) -> std::process::Output {
    let output = Command::new(aspect_cli())
        .args(args)
        .current_dir(dir.join("repo"))
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.join("cache"))
        .env("HOME", dir.join("home"))
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .envs(vars.iter().copied())
        .output()
        .expect("running the CLI");
    assert!(
        output.status.success(),
        "aspect {args:?} failed: {output:?}"
    );
    output
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .current_dir(repo)
        .env("HOME", repo.parent().unwrap().join("home"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .status()
        .expect("running git");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn the_list_report_follows_the_color_rule_on_a_pipe() {
    let dir = tempfile::tempdir().expect("temp dir");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::write(repo.join("MODULE.aspect"), "").unwrap();
    aspect(dir.path(), &["worktree", "add", "--create", "feat"], &[]);

    for (vars, want) in [
        (&[][..], false),
        (&[("CLICOLOR_FORCE", "1")][..], true),
        (&[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")][..], false),
    ] {
        let stderr = aspect(dir.path(), &["worktree", "list"], vars).stderr;
        let styled = stderr.windows(DIM_HEADER.len()).any(|w| w == DIM_HEADER);
        assert_eq!(
            styled,
            want,
            "under {vars:?}: {}",
            String::from_utf8_lossy(&stderr)
        );
    }
}
