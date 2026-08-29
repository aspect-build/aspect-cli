//! End-to-end tests for the fatal-signal crash reporter: spawn the real CLI
//! binary with the internal crash trigger (`ASPECT_INTERNAL_TEST_CRASH`) and
//! assert the report reaches stderr while the process still dies with the
//! original signal (so CI harnesses observe an unchanged exit status).
//!
//! These run under `cargo test` (they need the built binary via
//! `CARGO_BIN_EXE_*`). The handler's pure logic is unit-tested inside
//! `crash_handler.rs`, which is what the Bazel `:test` target covers.

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

/// Assert a crash report was printed and the process died by `signal`.
fn assert_reported(out: &Output, signal_label: &str, signal: libc::c_int) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("fatal signal: {signal_label}")),
        "missing signal line in stderr: {stderr}"
    );
    // The fault site must be reported. `crash pc` is populated from the
    // ucontext on Linux (the reliable address where unwinding truncates on
    // static-musl); elsewhere the raw frame walk supplies the addresses. Either
    // way, at least one `0x…` instruction pointer must reach stderr — that is
    // the part that previously faulted and printed nothing inside the handler.
    assert!(
        stderr.contains("crash pc:") && stderr.contains("addr2line -fe aspect-cli"),
        "missing crash-site report in stderr: {stderr}"
    );
    // Code addresses print as `<runtime> (+<static offset>)`; at least one
    // frame line must reach stderr — this is the part that previously faulted
    // and printed nothing inside the handler on static-musl builds.
    assert!(
        stderr.contains(" (+0x"),
        "no resolvable frame offsets in backtrace: {stderr}"
    );
    assert_eq!(
        out.status.signal(),
        Some(signal),
        "expected death by {signal_label}, got {:?}",
        out.status
    );
}

#[test]
fn segv_on_main_thread_is_reported() {
    assert_reported(&run_with_trigger("segv", &[]), "SIGSEGV", libc::SIGSEGV);
}

/// The Starlark task runs on a spawned (tokio blocking/worker) thread, so a
/// crash there is the realistic case — the handler must report it too.
#[test]
fn segv_on_spawned_thread_is_reported() {
    assert_reported(
        &run_with_trigger("segv-thread", &[]),
        "SIGSEGV",
        libc::SIGSEGV,
    );
}

/// Stack overflow lands on the alternate signal stack; the raw-address walk
/// must still emit frames rather than double-faulting into silence.
#[test]
fn stack_overflow_on_spawned_thread_is_reported() {
    let out = run_with_trigger("stackoverflow-thread", &[]);
    // SIGSEGV on Linux, SIGBUS on macOS — both are handled fatal signals.
    let sig = out.status.signal().expect("expected death by signal");
    assert!(
        sig == libc::SIGSEGV || sig == libc::SIGBUS,
        "expected SIGSEGV/SIGBUS, got signal {sig}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("fatal signal:") && stderr.contains(" (+0x"),
        "stack-overflow crash produced no backtrace: {stderr}"
    );
}

#[test]
fn abort_is_reported() {
    assert_reported(&run_with_trigger("abort", &[]), "SIGABRT", libc::SIGABRT);
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

/// The allocator is built in secure mode and we register an error handler that
/// aborts on any corruption code. mimalloc's own default handler reports a
/// double free (`EAGAIN`) and then *continues*, which would let a corrupted
/// heap run on to fail somewhere unrelated — so assert the abort actually
/// happens and is reported.
#[test]
fn allocator_corruption_aborts_and_is_reported() {
    let out = run_with_trigger("double-free", &[("MIMALLOC_SHOW_ERRORS", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("double free detected"),
        "allocator did not report the double free: {stderr}"
    );
    assert_reported(&out, "SIGABRT", libc::SIGABRT);
}
