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

/// `aspect feature` renders its list through `print!`, which the newline-less
/// `out!` covers; a partial migration would leave this path panicking.
#[test]
fn closed_stdout_does_not_panic_listing_features() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aspect-cli"))
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_aspect-cli"))
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_aspect-cli"))
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

/// The panicking print macros must not come back.
///
/// `println!` and friends panic when the write fails, and a reader that leaves
/// early makes that routine — see CLAUDE.md, "Writing to stdout/stderr". The
/// tolerant `outln!` / `errln!` / `out!` replace them. This scans the crate's
/// own sources so a reintroduction fails here rather than as a stranded task
/// months later.
///
/// Sources are read from a compile-time embed rather than the filesystem so the
/// test passes under Bazel's sandbox, matching `loads_use_canonical_public_private_form`.
#[test]
fn no_panicking_print_macros() {
    use include_dir::{Dir, include_dir};
    static SRC: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src");

    let mut offenders = Vec::new();
    let mut stack = vec![&SRC];
    while let Some(dir) = stack.pop() {
        for entry in dir.entries() {
            match entry {
                include_dir::DirEntry::Dir(d) => stack.push(d),
                include_dir::DirEntry::File(f) => {
                    let path = f.path().to_string_lossy().to_string();
                    if !path.ends_with(".rs") {
                        continue;
                    }
                    let Some(text) = f.contents_utf8() else {
                        continue;
                    };
                    // `out.rs` defines the replacements in terms of the real
                    // writers; test modules may panic freely.
                    if path.ends_with("out.rs") {
                        continue;
                    }
                    let mut in_tests = false;
                    for (n, line) in text.lines().enumerate() {
                        if line.trim_start().starts_with("#[cfg(test)]") {
                            in_tests = true;
                        }
                        if in_tests {
                            continue;
                        }
                        let t = line.trim_start();
                        if t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
                            continue;
                        }
                        for m in ["println!(", "eprintln!(", "print!(", "eprint!("] {
                            // `outln!(` ends in `print!(`-free text; match the
                            // macro at a token boundary.
                            let is_call = line.contains(m)
                                && !line.contains(&format!("out{m}"))
                                && !line.contains(&format!("err{m}"));
                            if is_call {
                                offenders.push(format!("{path}:{}: {}", n + 1, t));
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use outln!/errln!/out! from axl_runtime::out instead (see CLAUDE.md):\n  {}",
        offenders.join("\n  ")
    );
}
