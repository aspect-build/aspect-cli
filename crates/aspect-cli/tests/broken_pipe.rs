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

use std::process::{Command, Stdio};

/// Rust's exit code for a panic.
const PANIC_EXIT: i32 = 101;

#[test]
fn closed_stdout_does_not_panic() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aspect-cli"))
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
