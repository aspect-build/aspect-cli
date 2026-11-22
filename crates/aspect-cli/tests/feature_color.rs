//! `aspect feature` (rendered by hand) and `aspect feature <name>` (rendered by
//! clap) color a piped stdout under the same environment, so the CLI's own
//! color rule agrees with clap's.
//!
//! Spawns the real CLI with stdout captured, so neither stream is a terminal.

mod common;

use common::{aspect_cli, base_env};
use std::process::Command;

/// Whether `aspect <args>` wrote any ANSI escape to its piped stdout, run in a
/// scratch workspace under `vars` with no ambient color variables.
fn colored(args: &[&str], vars: &[(&str, &str)]) -> bool {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let output = Command::new(aspect_cli())
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("HOME", dir.path().join("home"))
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .envs(vars.iter().copied())
        .output()
        .expect("running the CLI");
    assert!(
        output.status.success(),
        "aspect {args:?} failed: {output:?}"
    );
    output.stdout.contains(&0x1b)
}

#[test]
fn the_feature_list_and_a_feature_page_agree_on_color() {
    for (vars, want) in [
        (&[][..], false),
        (&[("CLICOLOR", "1"), ("CI", "true")][..], false),
        (&[("CLICOLOR_FORCE", "1")][..], true),
        (&[("CLICOLOR_FORCE", "1"), ("CLICOLOR", "0")][..], true),
        (&[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")][..], false),
    ] {
        assert_eq!(colored(&["feature"], vars), want, "list under {vars:?}");
        assert_eq!(
            colored(&["feature", "console"], vars),
            want,
            "page under {vars:?}"
        );
    }
}
