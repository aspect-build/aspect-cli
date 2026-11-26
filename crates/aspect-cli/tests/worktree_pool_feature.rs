//! The WorktreePool feature's options reach the worktree tasks: an out-of-range
//! `max_slots` or `abandoned_grace_hours`, set on the command line or in
//! `config.axl`, is refused by `aspect worktree add` naming the option, and the
//! defaults (or a disabled feature) pass the check.
//!
//! The scratch workspace is not a git repository, so `add` checks the settings
//! and then refuses with `not_a_repository`: no worktree is ever created.

mod common;

use common::{aspect_cli, base_env};
use std::process::{Command, Output};

/// Run `aspect worktree add probe --output=json <args>` in a scratch workspace
/// under an empty `$HOME`, with `.aspect/config.axl` holding `config`, if given.
fn add(args: &[&str], config: Option<&str>) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    if let Some(config) = config {
        let aspect = dir.path().join(".aspect");
        std::fs::create_dir(&aspect).expect(".aspect");
        std::fs::write(aspect.join("config.axl"), config).expect("config.axl");
    }
    let output = Command::new(aspect_cli())
        .args(["worktree", "add", "probe", "--output=json"])
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("HOME", dir.path().join("home"))
        // Keep git from finding a repository above the scratch dir.
        .env(
            "GIT_CEILING_DIRECTORIES",
            dir.path().parent().expect("temp dir parent"),
        )
        .output()
        .unwrap_or_else(|e| panic!("running `aspect worktree add {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

/// The `error` code of `out`'s JSON refusal on stdout.
fn error(out: &Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("\"error\": \""))
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no error code in:\n{stdout}"))
        .to_owned()
}

/// `out` refused the setting `option` as `invalid_config`.
fn assert_refused(out: Output, option: &str) {
    assert_eq!(error(&out), "invalid_config");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("\"setting\": \"{option}\"")),
        "{stdout}"
    );
}

#[test]
fn defaults_pass_the_check() {
    assert_eq!(error(&add(&[], None)), "not_a_repository");
}

#[test]
fn an_out_of_range_flag_is_refused_naming_it() {
    assert_refused(
        add(&["--worktree-pool:max-slots=0"], None),
        "--worktree-pool:max-slots",
    );
    assert_refused(
        add(&["--worktree-pool:abandoned-grace-hours=-1"], None),
        "--worktree-pool:abandoned-grace-hours",
    );
}

#[test]
fn a_config_axl_value_reaches_the_tasks() {
    let config = r#"load("@aspect//feature/worktree_pool.axl", "WorktreePool")

def config(ctx):
    ctx.features[WorktreePool].args.abandoned_grace_hours = -1
"#;
    assert_refused(
        add(&[], Some(config)),
        "--worktree-pool:abandoned-grace-hours",
    );
}

#[test]
fn a_disabled_feature_leaves_the_defaults() {
    let config = r#"load("@aspect//feature/worktree_pool.axl", "WorktreePool")

def config(ctx):
    ctx.features[WorktreePool].enabled = False
    ctx.features[WorktreePool].args.max_slots = 0
"#;
    assert_eq!(error(&add(&[], Some(config))), "not_a_repository");
}
