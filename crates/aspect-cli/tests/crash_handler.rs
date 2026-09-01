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

use std::os::fd::FromRawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output};

/// How long to wait for a spawned crash to land. Generous because the debug
/// binary this suite spawns is large and unoptimized: ~2.5s just to reach
/// `main` on a warm laptop, more on a loaded CI machine.
const CRASH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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
    let path = scratch_dir().join(format!("aspect-crash-{}.log", std::process::id()));
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

/// The crash log must be reachable when stderr *blocks*, not only when it loses
/// data. A pipe whose reader has stopped draining and whose buffer is full
/// makes `write(2)` on fd 2 block forever, so a handler that opened the log
/// only after its first stderr write would never open it at all — the fallback
/// gated behind the sink it exists to replace.
///
/// Asserts the file appears and names the signal, which requires the `open(2)`
/// to precede every stderr write. The body legitimately may not be there: the
/// handler writes the marker to stderr before the steps that can fault, so it
/// is that write which wedges, and the process stays hung (hence the kill).
#[test]
fn crash_log_is_written_when_stderr_blocks() {
    let path = scratch_dir().join(format!("aspect-crash-blocked-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);

    // A pipe nobody reads, pre-filled so the child's first stderr write blocks.
    // Filling uses O_NONBLOCK on our own copy of the write end; the flag lives
    // on the shared open file description, so it must be cleared before the
    // spawn or the child would get EAGAIN instead of blocking.
    let (read_fd, write_fd) = new_pipe();
    set_nonblocking(write_fd, true);
    let filler = [0u8; 4096];
    loop {
        // SAFETY: writing a valid buffer to a pipe fd this test owns.
        let n = unsafe { libc::write(write_fd, filler.as_ptr().cast(), filler.len()) };
        if n <= 0 {
            break; // EAGAIN: the pipe buffer is full
        }
    }
    set_nonblocking(write_fd, false);

    let mut child = Command::new(cli_bin())
        .env("ASPECT_INTERNAL_TEST_CRASH", "segv")
        .env("ASPECT_CRASH_LOG", path.to_str().unwrap())
        // SAFETY: `Stdio` takes ownership of the duplicate, leaving the
        // test's own `write_fd` open (closed at the end of the test).
        .stderr(unsafe { std::process::Stdio::from_raw_fd(libc::dup(write_fd)) })
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn aspect-cli");

    let start = std::time::Instant::now();
    while !path.exists() && start.elapsed() < CRASH_TIMEOUT {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // The child is wedged on the blocked stderr write by design; it will not
    // exit on its own.
    let _ = child.kill();
    let _ = child.wait();
    // SAFETY: fds this test opened and no longer uses.
    unsafe {
        libc::close(read_fd);
        libc::close(write_fd);
    }

    let log = std::fs::read_to_string(&path).expect(
        "no crash log while stderr was blocked: the handler must open it before writing to stderr",
    );
    let _ = std::fs::remove_file(&path);
    assert!(
        log.contains("fatal signal: SIGSEGV"),
        "crash log missing the signal marker: {log}"
    );
}

/// Bazel gives each test a private scratch dir; fall back to the system temp
/// dir under plain `cargo test`.
fn scratch_dir() -> std::path::PathBuf {
    std::env::var("TEST_TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// A fresh `pipe(2)` as `(read, write)` raw fds.
fn new_pipe() -> (libc::c_int, libc::c_int) {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid 2-element array for pipe(2) to fill.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    assert_eq!(rc, 0, "pipe(2) failed: {}", std::io::Error::last_os_error());
    (fds[0], fds[1])
}

fn set_nonblocking(fd: libc::c_int, on: bool) {
    // SAFETY: F_GETFL/F_SETFL on an fd this test owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        let flags = if on {
            flags | libc::O_NONBLOCK
        } else {
            flags & !libc::O_NONBLOCK
        };
        libc::fcntl(fd, libc::F_SETFL, flags);
    }
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
