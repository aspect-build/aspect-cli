//! `aspect worktree` styles its stderr report on a pipe whenever the CLI's
//! color rule allows it (`CLICOLOR_FORCE`, a CI host whose log renders ANSI),
//! as the console lines around it do, and never styles
//! what it writes to stdout (`--output=json`, `--output=path`).
//!
//! Spawns the real CLI and git in a scratch clone with their own `HOME`, no
//! system git config and no ambient `GIT_*` variables (a git hook's `GIT_DIR`
//! would point both at the outer repository), so the pool listed is the
//! test's own.

mod common;

use common::{aspect_cli, base_env, git, isolated};
use std::path::Path;
use std::process::{Command, Output};

/// The `muted` column header of the `taken` table.
const DIM_HEADER: &[u8] = b"\x1b[38;5;247m  slot";

/// The `label.info` prefix of the `worktree ready:` line.
const INFO_PREFIX: &[u8] = b"\x1b[1;38;5;247mINFO\x1b[0m: ";

/// The CLI with `args`, under [`base_env`] minus any color variable but
/// `vars`; asserts it succeeded.
fn aspect(dir: &Path, args: &[&str], vars: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(aspect_cli());
    cmd.env_clear().envs(base_env());
    let output = isolated(cmd, dir)
        .env("ASPECT_CLI_CACHE", dir.join("cache"))
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .envs(vars.iter().copied())
        .args(args)
        .output()
        .expect("running the CLI");
    assert!(
        output.status.success(),
        "aspect {args:?} failed: {output:?}"
    );
    output
}

fn has_escape(bytes: &[u8]) -> bool {
    bytes.contains(&0x1b)
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn reports_follow_the_color_rule_on_a_pipe_and_stdout_stays_plain() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("repo")).unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::write(dir.path().join("repo/MODULE.aspect"), "").unwrap();

    const FORCE: (&str, &str) = ("CLICOLOR_FORCE", "1");
    let added = aspect(
        dir.path(),
        &["worktree", "add", "--create", "feat", "--output=path"],
        &[FORCE],
    );
    assert!(
        !has_escape(&added.stdout),
        "--output=path styled: {:?}",
        String::from_utf8_lossy(&added.stdout)
    );
    assert!(
        contains(&added.stderr, INFO_PREFIX),
        "`INFO:` prefix unstyled: {:?}",
        String::from_utf8_lossy(&added.stderr)
    );

    for (vars, want) in [
        (&[][..], false),
        (&[FORCE][..], true),
        // The marker alone: `detect_ci` still says "not CI", so no CI-only
        // feature runs (`--remote` auto-wiring would read the keyring).
        (&[("CIRCLECI", "true")][..], true),
        (&[FORCE, ("NO_COLOR", "1")][..], false),
    ] {
        let stderr = aspect(dir.path(), &["worktree", "list"], vars).stderr;
        assert_eq!(
            contains(&stderr, DIM_HEADER),
            want,
            "under {vars:?}: {}",
            String::from_utf8_lossy(&stderr)
        );
    }

    let json = aspect(dir.path(), &["worktree", "list", "--output=json"], &[FORCE]);
    assert!(
        !has_escape(&json.stdout),
        "--output=json styled: {:?}",
        String::from_utf8_lossy(&json.stdout)
    );
}
