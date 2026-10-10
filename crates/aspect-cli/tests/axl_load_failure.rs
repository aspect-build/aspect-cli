//! A `.aspect/*.axl` file the user broke must not cost them the commands they
//! need to find it.
//!
//! The CLI's whole surface is defined in AXL, so a single unparsable file used
//! to fail every command — `aspect help`, the way out, included. The policy
//! these tests pin (see `Unloadable` in `main.rs`): `version` and `help` run
//! anyway and report the failure as a warning, while anything that dispatches
//! a task still fails on it. Both halves matter. A silently swallowed syntax
//! error would be worse than the hard failure it replaced, so every degraded
//! run is checked for the diagnostic as well as the exit code.
//!
//! Also pinned here: the diagnostic carries exactly one `error:` prefix.
//! Starlark renders its own, and the CLI used to add a second.
//!
//! Every run gets a throwaway `$HOME` so the developer's own
//! `~/.aspect/config.axl` cannot join the run and change what is reported.

mod common;

use common::aspect_cli;
use std::path::Path;
use std::process::{Command, Output};

/// A task file that parses, so a degraded `help` has something of the user's
/// own left to list.
const GOOD_SCRIPT: &str = r#"
def _impl(ctx: TaskContext) -> int:
    return 0

goodtask = task(summary = "A task that loads.", implementation = _impl)
"#;

/// The reported shape: a typo that cannot parse. `is` is a reserved keyword,
/// so this fails in the parser rather than at runtime.
const UNPARSABLE_SCRIPT: &str = "\nthis is not valid starlark(\n";

/// A `config.axl` that parses and then fails when its `config()` runs, which
/// is the other way a hand-edited file breaks the startup phases.
const FAILING_CONFIG: &str = r#"
def config(ctx):
    ctx.tasks["no-such-task"].args.anything = 1
"#;

/// Run `aspect <args>` in a scratch workspace whose `.aspect/` holds `files`,
/// each a `(name, contents)` pair.
fn run_in_workspace(files: &[(&str, &str)], args: &[&str]) -> Output {
    let dir = tempfile::tempdir().expect("temp dir");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).expect("home");
    std::fs::write(dir.path().join("MODULE.bazel"), "").expect("MODULE.bazel");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    for (name, contents) in files {
        std::fs::write(aspect.join(name), contents).expect("writing a fixture");
    }
    let output = Command::new(aspect_cli())
        .args(args)
        .current_dir(dir.path())
        .env("HOME", &home)
        .env(
            "ASPECT_CREDENTIALS_FILE",
            dir.path().join("credentials.json"),
        )
        .env_remove("ASPECT_DEBUG")
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {}`: {e}", args.join(" ")));
    dir.close().expect("removing temp dir");
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The command succeeded, and said on stderr which file failed and why.
fn assert_degraded(output: &Output, file: &str) {
    let stderr = stderr(output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected success despite the broken file\n--- stderr ---\n{stderr}"
    );
    assert!(
        stderr.contains("warning: ") && stderr.contains(file),
        "expected a warning naming {file} in:\n{stderr}"
    );
    assert_one_error_prefix(&stderr);
}

/// The command failed with the AXL diagnostic as its reason.
fn assert_failed_on_axl(output: &Output, needle: &str) {
    let stderr = stderr(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1\n--- stderr ---\n{stderr}"
    );
    assert!(stderr.contains(needle), "expected {needle:?} in:\n{stderr}");
    assert_one_error_prefix(&stderr);
}

/// Starlark renders its diagnostics with their own `error: ` title, so the CLI
/// must not prefix them again.
fn assert_one_error_prefix(stderr: &str) {
    assert!(
        !stderr.contains("error: error:"),
        "the diagnostic was prefixed twice:\n{stderr}"
    );
}

#[test]
fn version_survives_an_unparsable_script() {
    let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["version"]);
    assert_degraded(&output, "policy.axl");
    assert!(
        stderr(&output).contains("Parse error"),
        "the parse error must still be visible:\n{}",
        stderr(&output)
    );
    assert!(
        !stdout(&output).trim().is_empty(),
        "expected a version on stdout"
    );
}

#[test]
fn help_survives_an_unparsable_script_and_lists_what_did_load() {
    let output = run_in_workspace(
        &[("policy.axl", UNPARSABLE_SCRIPT), ("good.axl", GOOD_SCRIPT)],
        &["help"],
    );
    assert_degraded(&output, "policy.axl");
    let stderr = stderr(&output);
    assert!(
        stderr.contains("Parse error"),
        "the parse error must still be visible:\n{stderr}"
    );
    assert!(
        stderr.contains("the tasks it defines are unavailable"),
        "help must say what the broken file cost:\n{stderr}"
    );

    let stdout = stdout(&output);
    for listed in ["build", "test", "goodtask"] {
        assert!(
            stdout.contains(listed),
            "expected {listed:?} in the degraded help:\n{stdout}"
        );
    }
}

/// `--help`, `--version` and a bare `aspect` are rendered by Clap rather than
/// by the `help` / `version` subcommands, so they are their own path.
#[test]
fn clap_rendered_help_survives_an_unparsable_script() {
    let flag = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["--help"]);
    assert_degraded(&flag, "policy.axl");
    assert!(stdout(&flag).contains("build"), "expected the task list");

    let version = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["--version"]);
    assert_degraded(&version, "policy.axl");
    assert!(
        !stdout(&version).trim().is_empty(),
        "expected a version on stdout"
    );

    // A bare `aspect` prints the same help, but keeps Clap's "no command"
    // exit code; what matters is that it is not the AXL failure's.
    let bare = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &[]);
    assert_eq!(
        bare.status.code(),
        Some(2),
        "--- stderr ---\n{}",
        stderr(&bare)
    );
    assert!(
        stderr(&bare).contains("warning:") && stderr(&bare).contains("Tasks:"),
        "expected the warning and the help:\n{}",
        stderr(&bare)
    );
}

#[test]
fn a_task_still_fails_on_an_unparsable_script() {
    // A built-in, which needs nothing from the broken file, and the task the
    // broken file was supposed to define. Both must fail: the workspace did
    // not fully load, so neither can be trusted to run.
    for args in [vec!["build", "//..."], vec!["policy"]] {
        let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &args);
        assert_failed_on_axl(&output, "Parse error");
    }
}

/// `describe` and `feature` render the resolved AXL surface itself, so a
/// surface that quietly lost entries would be a wrong answer rather than a
/// degraded one.
#[test]
fn the_surface_reporting_commands_still_fail_on_an_unparsable_script() {
    for args in [vec!["describe"], vec!["feature"]] {
        let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &args);
        assert_failed_on_axl(&output, "Parse error");
    }
}

#[test]
fn version_and_help_survive_a_failing_config() {
    for args in [vec!["version"], vec!["help"]] {
        let output = run_in_workspace(&[("config.axl", FAILING_CONFIG)], &args);
        assert_degraded(&output, "config.axl");
        assert!(
            stderr(&output).contains("no task found"),
            "the config's own error must still be visible:\n{}",
            stderr(&output)
        );
    }
}

#[test]
fn a_task_still_fails_on_a_failing_config() {
    let output = run_in_workspace(&[("config.axl", FAILING_CONFIG)], &["build", "//..."]);
    assert_failed_on_axl(&output, "no task found");
}

/// The warning is a consequence of a broken file, not of every run.
#[test]
fn a_healthy_workspace_warns_about_nothing() {
    let output = run_in_workspace(&[("good.axl", GOOD_SCRIPT)], &["help"]);
    let stderr = stderr(&output);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");
    assert!(
        stderr.is_empty(),
        "a workspace that loads must say nothing on stderr:\n{stderr}"
    );
    assert!(
        stdout(&output).contains("goodtask"),
        "expected the user's task in the help"
    );
}

/// Keeps the fixture honest: `tempfile` hands out a path, and a test that
/// silently wrote its `.aspect` somewhere else would pass for the wrong reason.
#[test]
fn the_fixture_workspace_is_where_the_test_thinks_it_is() {
    let output = run_in_workspace(&[("policy.axl", UNPARSABLE_SCRIPT)], &["version"]);
    let stderr = stderr(&output);
    let named = stderr
        .lines()
        .find_map(|line| line.strip_prefix("warning: "))
        .and_then(|line| line.split(" could not be loaded").next())
        .map(Path::new)
        .expect("a warning naming the file");
    assert!(
        named.is_absolute() && named.ends_with(".aspect/policy.axl"),
        "expected an absolute path to the fixture, got {named:?}"
    );
}
