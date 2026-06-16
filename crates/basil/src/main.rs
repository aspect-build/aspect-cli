//! basil — a fake `bazel` binary used to drive integration tests of
//! `ctx.bazel.build`. The axl-runtime `BazelBackend::Fake` path fork+execs
//! this binary directly (no `BAZEL_REAL`).
//!
//! All synthesis logic lives in the `basil-core` library so the standalone
//! binary and a shipped `aspect` self-exec subcommand (roadmap item 6) share
//! one implementation. This binary is just the argv/env front-end.
//!
//! Verb `build` / `test`: reads a declared `BazelExpectation` (length-delimited
//! protobuf) off the inherited control channel named by `ASPECT_FAKE_BAZEL_FD`,
//! then synthesizes a consistent BES stream onto `--build_event_binary_file`
//! and exits with the fixture's code.

use std::env;
use std::fs::File;
use std::io::Write;
use std::os::fd::FromRawFd;
use std::process;
use std::time::Duration;

use basil_core::{BazelExpectation, replay_expectation};

/// Env var naming the inherited control-channel fd carrying the serialized
/// `BazelExpectation`. Mirrors the constant the parent sets in
/// `axl-runtime`'s `Fake` backend.
const FAKE_FD_ENV: &str = "ASPECT_FAKE_BAZEL_FD";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    // First non-flag arg is the verb (e.g. "build"). Flags before it (like
    // bazel startup flags) are tolerated and ignored — we don't model bazel's
    // real flag positioning rules.
    let verb = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("");

    match verb {
        "build" | "test" => run_build(&args),
        "" => {
            eprintln!("basil: no verb given");
            process::exit(2);
        }
        other => {
            eprintln!("basil: unsupported verb: {other}");
            process::exit(2);
        }
    }
}

fn run_build(args: &[String]) {
    let bes_path = find_flag_value(args, "--build_event_binary_file");
    let Some(fd) = control_fd() else {
        eprintln!("basil: no {FAKE_FD_ENV} control fd inherited; nothing to replay");
        process::exit(2);
    };
    run_generic(fd, bes_path.as_deref());
}

/// Read the serialized `BazelExpectation` from the inherited control fd,
/// synthesize the BES stream into `bes_path`, and exit with the fixture's
/// code. Never returns.
fn run_generic(fd: i32, bes_path: Option<&str>) -> ! {
    // SAFETY: `fd` is an inherited control-channel fd the parent dup2'd into
    // place for us; we own this end for the lifetime of the process.
    let file = unsafe { File::from_raw_fd(fd) };
    let exp = match BazelExpectation::read_frame(file) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("basil: reading BazelExpectation from control fd {fd}: {e}");
            process::exit(2);
        }
    };
    // A small open_delay widens the window for the AXL iterator's late
    // `.subscribe()` to land before events fan out on the warm-daemon path.
    let code = replay_expectation(&exp, bes_path, Duration::from_millis(50));
    let _ = std::io::stderr().flush();
    process::exit(code);
}

/// The inherited control fd, if the parent named one via `ASPECT_FAKE_BAZEL_FD`.
fn control_fd() -> Option<i32> {
    env::var(FAKE_FD_ENV).ok().and_then(|s| s.parse().ok())
}

/// Finds `--name <value>` or `--name=<value>` in argv.
fn find_flag_value(args: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    for (i, a) in args.iter().enumerate() {
        if a == name {
            return args.get(i + 1).cloned();
        }
        if let Some(v) = a.strip_prefix(&prefix) {
            return Some(v.to_string());
        }
    }
    None
}
