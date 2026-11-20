//! A `.bazelrc` that can't be resolved, as the user sees it: the task fails at
//! setup with the rc problem worded like Bazel's own error — which file asked
//! for the config, which rc files were searched — and no AXL traceback, which
//! would make a mistake in the user's rc read like a crash in the CLI. The
//! traceback is still there under `ASPECT_DEBUG`.
//!
//! Every run gets a stub `bazel` that records its argv and answers the version
//! probes, so nothing here starts a server, and an environment built from
//! scratch so no CI or `ASPECT_*` variable of the host steers the task.

#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

use common::aspect_cli;

const STUB_BAZEL: &str = r#"#!/bin/sh
echo "$@" >> "$BAZEL_ARGV_LOG"
case "$1" in
--version) echo 'bazel 9.0.0' ;;
*) echo 'server_pid: 1'; echo 'release: release 9.0.0' ;;
esac
"#;

const MISSING_RC: &str = "build --config=missing\n";

/// Run `aspect <args>` in a workspace whose `.bazelrc` is `bazelrc`, returning
/// the output and every argv the stub `bazel` saw.
fn run(bazelrc: &str, args: &[&str], debug: bool) -> (Output, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    let ws = root.join("ws");
    std::fs::create_dir(&ws).expect("workspace");
    std::fs::write(ws.join("MODULE.bazel"), "").expect("MODULE.bazel");
    std::fs::write(ws.join(".bazelrc"), bazelrc).expect(".bazelrc");

    let bin = root.join("bin");
    std::fs::create_dir(&bin).expect("bin");
    let bazel = bin.join("bazel");
    std::fs::write(&bazel, STUB_BAZEL).expect("stub bazel");
    std::fs::set_permissions(&bazel, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let argv_log = root.join("bazel-argv.log");

    let home = root.join("home");
    std::fs::create_dir(&home).expect("home");
    let mut cmd = Command::new(aspect_cli());
    cmd.args(args)
        .current_dir(&ws)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", &home)
        .env("BAZEL_ARGV_LOG", &argv_log)
        .env("ASPECT_CREDENTIALS_FILE", home.join("credentials.json"))
        .env("DO_NOT_TRACK", "1");
    if debug {
        cmd.env("ASPECT_DEBUG", "1");
    }
    let out = cmd.output().expect("spawn aspect-cli");
    let argv = std::fs::read_to_string(&argv_log).unwrap_or_default();
    (out, argv)
}

/// Whether the stub saw a `bazel build`, as opposed to the version probes.
fn ran_bazel_build(argv: &str) -> bool {
    argv.lines()
        .any(|l| l.split_whitespace().any(|a| a == "build"))
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn an_undefined_config_in_the_bazelrc_fails_with_the_rc_problem_and_no_traceback() {
    let (out, argv) = run(MISSING_RC, &["build", "//..."], false);
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains(
            "error: bazelrc: config value 'missing' is not defined in any .rc file (needed by 'build')"
        ),
        "{err}"
    );
    assert!(err.contains("--config=missing is requested by:"), "{err}");
    assert!(
        err.contains(&format!("ws{}.bazelrc", std::path::MAIN_SEPARATOR)),
        "the report names the rc file:\n{err}"
    );
    assert!(err.contains("Define a `common:missing`"), "{err}");
    assert!(!err.contains("Traceback"), "{err}");
    assert!(
        !ran_bazel_build(&argv),
        "bazel build ran before the rc was checked:\n{argv}"
    );
}

#[test]
fn a_task_that_opens_its_setup_phase_fails_there() {
    let (out, argv) = run(MISSING_RC, &["run", "//x"], false);
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains("config value 'missing' is not defined"),
        "{err}"
    );
    assert!(!err.contains("Traceback"), "{err}");
    assert!(!err.contains("🔨 Build"), "the build phase opened:\n{err}");
    assert!(
        !ran_bazel_build(&argv),
        "bazel build ran before the rc was checked:\n{argv}"
    );
}

#[test]
fn aspect_debug_still_prints_the_traceback() {
    let (out, _) = run(MISSING_RC, &["build", "//..."], true);
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains("config value 'missing' is not defined"),
        "{err}"
    );
    assert!(err.contains("Traceback"), "{err}");
}

#[test]
fn a_defined_config_is_not_reported() {
    let rc = "build --config=present\nbuild:present --jobs=4\n";
    let (out, _) = run(rc, &["build", "//..."], false);
    let err = stderr(&out);
    assert!(!err.contains("bazelrc:"), "{err}");
}
