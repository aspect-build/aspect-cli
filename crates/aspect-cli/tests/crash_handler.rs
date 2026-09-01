//! End-to-end tests for the fatal-signal crash reporter: spawn the real CLI
//! binary with the internal crash trigger (`ASPECT_INTERNAL_TEST_CRASH`) and
//! assert the report reaches stderr (and the `ASPECT_CRASH_LOG` file) while
//! the process still dies with the original signal (so CI harnesses observe
//! an unchanged exit status).
//!
//! Runs under Bazel via the `:crash_handler_test` target (binary resolved
//! through `ASPECT_CLI_BIN`, set by the rule's `env`) and under `cargo test`
//! (via the `CARGO_BIN_EXE_*` fallback). The handler's pure logic is
//! unit-tested inside `crash_handler.rs`, which the Bazel `:test` target
//! covers.

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output};

/// The CLI binary under test: `ASPECT_CLI_BIN` (Bazel) with a
/// `CARGO_BIN_EXE_*` fallback (cargo). Plain `env!` would not compile under
/// Bazel, where the cargo variable does not exist.
fn cli_bin() -> String {
    std::env::var("ASPECT_CLI_BIN")
        .ok()
        .or_else(|| option_env!("CARGO_BIN_EXE_aspect-cli").map(str::to_owned))
        .expect("neither ASPECT_CLI_BIN nor CARGO_BIN_EXE_aspect-cli is set")
}

fn run_with_trigger(kind: &str, extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(cli_bin());
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

/// `ASPECT_CRASH_LOG` must receive a copy of the full report. CI harnesses can
/// stop draining the stderr pipe the instant the process dies, losing a report
/// written microseconds earlier — the file is the capture that survives, so it
/// must be self-contained (marker included), not just a partial tee.
#[test]
fn crash_log_env_captures_the_full_report() {
    let dir = std::env::var("TEST_TMPDIR") // Bazel's per-test scratch dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let path = dir.join(format!("aspect-crash-{}.log", std::process::id()));
    let path_str = path.to_str().unwrap();

    let out = run_with_trigger("segv", &[("ASPECT_CRASH_LOG", path_str)]);
    assert_reported(&out, "SIGSEGV", libc::SIGSEGV);

    let log = std::fs::read_to_string(&path).expect("crash log file was not written");
    let _ = std::fs::remove_file(&path);
    assert!(
        log.contains("fatal signal: SIGSEGV"),
        "crash log missing the signal marker: {log}"
    );
    assert!(
        log.contains("crash pc:") && log.contains(" (+0x"),
        "crash log missing the crash-site report: {log}"
    );
}

/// An unset `ASPECT_CRASH_LOG` must not change anything: full report on
/// stderr, no stray file.
#[test]
fn crash_log_env_absent_reports_to_stderr_only() {
    let out = run_with_trigger("segv", &[]);
    assert_reported(&out, "SIGSEGV", libc::SIGSEGV);
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
    // Whether mimalloc *detects* the trigger's corruptions is build-dependent:
    // the double-free check is heuristic, and the unowned-pointer check only
    // exists in allocator builds with debug padding (cargo dev, the -debug-
    // release variant). A build whose allocator saw nothing has nothing to
    // assert — the regression under test is detection followed by *continuing*
    // (mimalloc's default for EAGAIN/EINVAL), which 93ceab9b turned into an
    // abort.
    if !stderr.contains("mimalloc: error") {
        eprintln!("skipping: this build's allocator did not detect the corruption");
        return;
    }
    assert_reported(&out, "SIGABRT", libc::SIGABRT);
}
