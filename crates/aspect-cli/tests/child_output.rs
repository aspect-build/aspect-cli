//! A child a task spawns has its output passed on through the CLI, whole and
//! in order: flushed as it comes, copied to the history's `output.log` a line
//! at a time, so it never lands inside the CLI's own lines, and waited on
//! until it is all passed on. With no terminal, a child marked interactive
//! runs as any other: piped and copied, with no marker lines.

mod common;

use common::{aspect_cli, base_env};
use std::process::{Command, Output, Stdio};

const TASKS: &str = r#"
def _partial(ctx):
    ctx.std.process.command("sh").args(["-c", "printf partial; sleep 0.3; printf ' rest\n'; printf tail"]).spawn().wait()
    print("AFTER")
    return 0

def _interactive(ctx):
    ctx.std.process.command("sh").args(["-c", "echo from-the-child"]).interactive(True).spawn().wait()
    return 0

partial = task(implementation = _partial, summary = "A child's partial lines.")
interactive = task(implementation = _interactive, summary = "An interactive child.")
"#;

/// Run `aspect <task>` in a scratch workspace with stdin and stdout not a
/// terminal; its output and the history's `output.log`.
fn run(task: &str) -> (Output, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("tasks.axl"), TASKS).expect("tasks.axl");
    let history = dir.path().join("history");
    let output = Command::new(aspect_cli())
        .arg(task)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("ASPECT_HISTORY_DIR", &history)
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {task}`: {e}"));
    let log = std::fs::read_dir(&history)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .map(|e| e.path().join("output.log"))
                .find(|p| p.exists())
        })
        .map(|p| std::fs::read_to_string(p).expect("output.log"))
        .unwrap_or_default();
    (output, log)
}

/// The position of `needle` in `text`, failing with both when it is not there.
fn at(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("want {needle:?} in:\n{text}"))
}

#[test]
fn a_childs_partial_lines_are_passed_on_whole_and_in_order() {
    let (output, log) = run("partial");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    // Its stdout is the CLI's stdout (here not stderr's file), all of it.
    let shown = String::from_utf8_lossy(&output.stdout);
    assert_eq!(shown, "partial rest\ntail", "the child's stdout, all of it");
    assert!(
        at(&log, "partial rest\n") < at(&log, "tail\n") && at(&log, "tail\n") < at(&log, "AFTER"),
        "output.log keeps the child's lines whole, the CLI's after:\n{log}"
    );
}

#[test]
fn without_a_terminal_an_interactive_child_is_piped_and_unmarked() {
    let (output, log) = run("interactive");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(log.contains("from-the-child\n"), "copied:\n{log}");
    assert!(
        !log.contains("interactively"),
        "no marker lines without a terminal:\n{log}"
    );
}
