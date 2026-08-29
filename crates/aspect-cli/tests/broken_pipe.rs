//! A closed stdout must not abort the CLI.
//!
//! `println!` / `eprintln!` panic when their write fails, and a reader that
//! stops early — `aspect build … | head`, or a CI assertion piping into
//! `grep -q` — closes the pipe as soon as it has what it wants. The panic
//! (exit 101) aborts the process mid-task, so nothing runs the task's terminal
//! update and its GitHub check run is stranded "running" until the API sweeper
//! finalizes it as DISCONNECTED. See `axl_runtime::out`.
//!
//! Closing the read end before the child writes anything makes the first write
//! fail deterministically, rather than depending on the buffering race that
//! decides whether a real pipeline trips it.

use std::io::Write;
use std::process::{Command, Stdio};

/// Rust's exit code for a panic.
const PANIC_EXIT: i32 = 101;

/// Locate the CLI under test.
///
/// Bazel sets `ASPECT_CLI_BIN` from the `rust_test` rule's `env` via
/// `$(rootpath :aspect-cli)`, relative to the runfiles root that is a
/// Bazel-run test's cwd. Under cargo, `CARGO_BIN_EXE_*` points at the binary
/// cargo already built for this test. Mirrors `axl_runtime::test::basil_bin`.
fn aspect_cli() -> String {
    match std::env::var("ASPECT_CLI_BIN") {
        Ok(p) => std::fs::canonicalize(&p)
            .unwrap_or_else(|e| panic!("ASPECT_CLI_BIN={p:?} not found: {e}"))
            .to_string_lossy()
            .into_owned(),
        // `option_env!`, not `env!`: the cargo variable does not exist in a
        // Bazel build, and `env!` would fail to compile there.
        Err(_) => option_env!("CARGO_BIN_EXE_aspect-cli")
            .expect("set ASPECT_CLI_BIN or run under cargo")
            .to_string(),
    }
}

#[test]
fn closed_stdout_does_not_panic() {
    let mut child = Command::new(aspect_cli())
        // `describe` serializes the whole CLI surface through our own stdout
        // path (`cmd.rs`), unlike `--help`, which clap writes and error-checks
        // itself.
        .arg("describe")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn aspect-cli");

    // Close our read end before the child writes, so its writes get EPIPE.
    drop(child.stdout.take().expect("piped stdout"));

    let status = child.wait().expect("wait for aspect-cli");
    assert_ne!(
        status.code(),
        Some(PANIC_EXIT),
        "writing to a closed stdout panicked instead of exiting cleanly"
    );
}

/// `aspect feature` renders its list through `print!`, which the newline-less
/// `out!` covers; a partial migration would leave this path panicking.
#[test]
fn closed_stdout_does_not_panic_listing_features() {
    let mut child = Command::new(aspect_cli())
        .arg("feature")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn aspect-cli");
    drop(child.stdout.take().expect("piped stdout"));

    let status = child.wait().expect("wait for aspect-cli");
    assert_ne!(
        status.code(),
        Some(PANIC_EXIT),
        "`feature` panicked writing to a closed stdout"
    );
}

/// The credential helper's stdout is a protocol payload, not console output:
/// Bazel parses it. A failed write means Bazel got no response, so the helper
/// must report failure rather than exit 0 on output nobody received — the one
/// place `out!`/`outln!` would be the wrong tool.
#[test]
fn credential_helper_reports_a_failed_response_write() {
    let mut child = Command::new(aspect_cli())
        .arg("get")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn aspect-cli get");

    // Close the read end first, so the helper's response write cannot land.
    drop(child.stdout.take().expect("piped stdout"));
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(br#"{"uri":"https://example.com"}"#)
        .expect("write the helper request");

    let status = child.wait().expect("wait for aspect-cli get");
    assert!(
        !status.success(),
        "credential helper reported success for a response Bazel never received"
    );
}

/// AXL `print()` goes through starlark's default handler, a bare `eprintln!`
/// that panics on a failed write. Any task printing to a departed reader —
/// `aspect dev test-… | head` — aborted the process before it could report a
/// result. `dev test-bazel-results` prints heavily and needs no network.
#[test]
fn closed_stderr_does_not_panic_during_an_axl_task() {
    let mut child = Command::new(aspect_cli())
        .args(["dev", "test-bazel-results"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aspect-cli");

    // Close the read end before the task prints, so its writes get EPIPE.
    drop(child.stderr.take().expect("piped stderr"));

    let status = child.wait().expect("wait for aspect-cli");
    assert_ne!(
        status.code(),
        Some(PANIC_EXIT),
        "an AXL print() to a closed stderr panicked instead of being discarded"
    );
}
