//! End-to-end tests for the fatal-signal crash reporter: spawn the real CLI
//! binary with the internal crash trigger (`ASPECT_INTERNAL_TEST_CRASH`) and
//! assert the report reaches stderr while the process still dies with the
//! original signal (so CI harnesses observe an unchanged exit status).

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output};

fn run_with_trigger(kind: &str, extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aspect-cli"));
    cmd.env("ASPECT_INTERNAL_TEST_CRASH", kind);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to spawn aspect-cli")
}

#[test]
fn segv_reports_backtrace_and_dies_with_original_signal() {
    let out = run_with_trigger("segv", &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("fatal signal: SIGSEGV (segmentation fault)"),
        "missing signal line in stderr: {stderr}"
    );
    assert!(
        stderr.contains("backtrace:"),
        "missing backtrace in stderr: {stderr}"
    );
    assert_eq!(
        out.status.signal(),
        Some(libc::SIGSEGV),
        "expected death by SIGSEGV, got {:?}",
        out.status
    );
}

#[test]
fn abort_reports_and_dies_with_original_signal() {
    let out = run_with_trigger("abort", &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("fatal signal: SIGABRT (abort)"),
        "missing signal line in stderr: {stderr}"
    );
    assert_eq!(
        out.status.signal(),
        Some(libc::SIGABRT),
        "expected death by SIGABRT, got {:?}",
        out.status
    );
}

#[test]
fn opt_out_env_skips_the_report_but_not_the_crash() {
    let out = run_with_trigger("segv", &[("ASPECT_NO_CRASH_HANDLER", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("fatal signal"),
        "handler ran despite opt-out: {stderr}"
    );
    assert_eq!(
        out.status.signal(),
        Some(libc::SIGSEGV),
        "expected death by SIGSEGV, got {:?}",
        out.status
    );
}
