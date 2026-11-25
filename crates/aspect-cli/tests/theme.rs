//! The colour theme, end to end: `aspect feature`'s own lines under a theme
//! chosen by `config.json`, `ASPECT_THEME` and the background, and a theme
//! warning printed without hanging the process.
//!
//! Spawns the real CLI in a scratch workspace with a `HOME` of its own and no
//! ambient colour variables.

mod common;

use common::{aspect_cli, base_env};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `aspect feature` under `vars`, with `config` as the user's
/// `~/.aspect/config.json` when given; killed after a minute.
fn feature_list(config: Option<&str>, vars: &[(&str, &str)]) -> Output {
    feature_list_with(config, None, vars)
}

/// [`feature_list`] with `config_axl` as the repository's
/// `.aspect/config.axl` when given.
fn feature_list_with(
    config: Option<&str>,
    config_axl: Option<&str>,
    vars: &[(&str, &str)],
) -> Output {
    let files: Vec<(&str, &str)> = config_axl
        .map(|a| (".aspect/config.axl", a))
        .into_iter()
        .collect();
    let output = aspect(&["feature"], "", &files, config, vars);
    assert!(output.status.success(), "{output:?}");
    output
}

/// `aspect <args>` in a scratch workspace whose `MODULE.aspect` is `module`,
/// with `files` written (each relative to the workspace) and `config` as the
/// user's `~/.aspect/config.json`, under `vars`; killed after a minute.
fn aspect(
    args: &[&str],
    module: &str,
    files: &[(&str, &str)],
    config: Option<&str>,
    vars: &[(&str, &str)],
) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), module).expect("MODULE.aspect");
    for (path, content) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let home = dir.path().join("home");
    if let Some(config) = config {
        std::fs::create_dir_all(home.join(".aspect")).unwrap();
        std::fs::write(home.join(".aspect/config.json"), config).unwrap();
    }
    let mut child = Command::new(aspect_cli())
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("HOME", &home)
        .env("CLICOLOR_FORCE", "1")
        .envs(vars.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("running the CLI");
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().ok();
            panic!("aspect {args:?} did not finish");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

/// A run that ends before its configs run (a `MODULE.aspect` that does not
/// parse) still prints the theme's warnings, once.
#[test]
fn theme_warnings_print_when_the_run_ends_early() {
    let output = aspect(
        &["feature"],
        "this is not (axl",
        &[],
        Some(r#"{"theme": "drak"}"#),
        &[("BUILDKITE", "true")],
    );
    assert!(!output.status.success(), "{output:?}");
    let err = text(&output.stderr);
    assert_eq!(
        err.matches("theme: ~/.aspect/config.json \"theme\"")
            .count(),
        1,
        "{err:?}"
    );
}

/// `testonly_pin` is for tests: a task calling it fails, naming its file.
#[test]
fn testonly_pin_refuses_a_task() {
    let task = "def _impl(ctx):\n    ctx.std.theme.testonly_pin(\"ansi\")\n    return 0\n\npin = task(implementation = _impl)\n";
    let output = aspect(&["pin"], "", &[(".aspect/pin.axl", task)], None, &[]);
    assert!(!output.status.success(), "{output:?}");
    let err = text(&output.stderr);
    assert!(err.contains("theme.testonly_pin is for tests"), "{err:?}");
    assert!(err.contains("pin.axl"), "{err:?}");
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_known_dark_background_gets_the_fixed_colours() {
    let out = text(&feature_list(None, &[("BUILDKITE", "true")]).stdout);
    assert!(out.contains("\x1b[1;4;38;5;75mFeatures:"), "{out:?}");
    assert!(out.contains("\x1b[38;5;247mAspect CLI"), "{out:?}");
}

#[test]
fn a_light_background_gets_the_light_variant() {
    let out = text(&feature_list(None, &[("COLORFGBG", "0;15")]).stdout);
    assert!(out.contains("\x1b[1;4;38;5;26mFeatures:"), "{out:?}");
}

#[test]
fn config_json_and_aspect_theme_choose_the_theme() {
    let config = r#"{"theme": "colorblind", "appearance": "dark"}"#;
    let out = text(&feature_list(Some(config), &[]).stdout);
    assert!(
        out.contains("\x1b[1;4;38;5;43mFeatures:"),
        "colorblind accent: {out:?}"
    );
    let out = text(&feature_list(Some(config), &[("ASPECT_THEME", "mono")]).stdout);
    assert!(out.contains("\x1b[1;4mFeatures:"), "mono: {out:?}");
    assert!(!out.contains("38;"), "mono has no colour: {out:?}");
}

#[test]
fn no_color_is_plain_text() {
    let output = feature_list(None, &[("NO_COLOR", "1"), ("BUILDKITE", "true")]);
    assert!(!output.stdout.contains(&0x1b), "{:?}", text(&output.stdout));
}

/// A family `config.json` names that only the repository's `config.axl`
/// defines is found, with no warning from before the configs ran.
#[test]
fn a_config_json_name_a_config_axl_defines_needs_no_warning() {
    let axl = r#"
def config(ctx):
    ctx.theme.define("team-dark", {"extends": "aspect-dark", "roles": {"accent": 208}})
    ctx.theme.define("team-light", {"extends": "aspect-light", "roles": {"accent": 166}})
"#;
    let output = feature_list_with(
        Some(r#"{"theme": "team", "themes": {"mine-dark": {"extends": "team-dark"}}}"#),
        Some(axl),
        &[("BUILDKITE", "true")],
    );
    let err = text(&output.stderr);
    assert!(!err.contains("theme:"), "no theme warning: {err:?}");
    assert!(
        text(&output.stdout).contains("\x1b[1;4;38;5;208mFeatures:"),
        "{output:?}"
    );
}

/// The theme's own warnings are printed in the theme being resolved: asking
/// for it again from inside its resolution would hang the process.
#[test]
fn a_bad_theme_warns_once_and_falls_back() {
    let output = feature_list(Some(r#"{"theme": "drak"}"#), &[("BUILDKITE", "true")]);
    let err = text(&output.stderr);
    assert_eq!(
        err.matches("theme: ~/.aspect/config.json \"theme\"")
            .count(),
        1,
        "{err:?}"
    );
    assert!(
        err.contains("\x1b[1;38;5;221mWARNING\x1b[0m: theme:"),
        "{err:?}"
    );
    assert!(
        err.contains("did you mean \"dark\"") || err.contains("using \"aspect\""),
        "{err:?}"
    );
    assert!(text(&output.stdout).contains("\x1b[1;4;38;5;75mFeatures:"));
}
