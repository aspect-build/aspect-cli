//! basil — a fake `bazel` binary used to drive integration tests of
//! `ctx.bazel.build`. The runtime spawns whichever binary `BAZEL_REAL`
//! points at; tests point it at this one.
//!
//! All replay/synthesis logic lives in the `basil-core` library so the
//! standalone binary and a shipped `aspect` self-exec subcommand
//! (roadmap item 6) share one implementation. This binary is just the
//! argv/env front-end.
//!
//! Verbs:
//!   - `info <key>...`  — prints `key: value` lines; supports `server_pid`,
//!     `release`, `output_base`. The pid printed defaults to the basil
//!     process's own pid; tests can override via `BASIL_SERVER_PID` so a
//!     long-lived holder process keeps galvanize's `IfOpenForPid` retry
//!     check satisfied for the whole test.
//!   - `build` / `test` — two modes:
//!       * **named scenario** (`--scenario=<name>` in argv): replays one of
//!         basil-core's hard-coded scenarios (`success`,
//!         `cache_evicted_no_retry`, …). Used by the existing axl-runtime
//!         Rust integration tests.
//!       * **generic fixture** (control fd present): reads a declared
//!         `BazelExpectation` (length-delimited protobuf) off the inherited
//!         control channel named by `ASPECT_FAKE_BAZEL_FD`, then synthesizes
//!         a consistent BES stream. This is the path the AXL test runner's
//!         `Fake` backend drives.

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::os::fd::FromRawFd;
use std::process;
use std::time::Duration;

use basil_core::{BazelExpectation, ExitBehavior, replay_expectation, scenario, write_scenario};

/// Env var naming the inherited control-channel fd carrying the serialized
/// `BazelExpectation`. Mirrors the constant the parent sets in
/// `axl-runtime`'s `Fake` backend.
const FAKE_FD_ENV: &str = "ASPECT_FAKE_BAZEL_FD";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    // First non-flag arg is the verb (e.g. "info", "build"). Flags before it
    // (like bazel startup flags) are tolerated and ignored — we don't model
    // bazel's real flag positioning rules.
    let verb = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("");

    match verb {
        "info" => run_info(&args),
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

fn run_info(args: &[String]) {
    let pid: u32 = env::var("BASIL_SERVER_PID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(process::id);
    let release = env::var("BASIL_RELEASE").unwrap_or_else(|_| "9.0.0".to_string());

    for key in args.iter().filter(|a| !a.starts_with('-') && *a != "info") {
        match key.as_str() {
            "server_pid" => println!("server_pid: {pid}"),
            "release" => println!("release: {release}"),
            "output_base" => {
                let base =
                    env::var("BASIL_OUTPUT_BASE").unwrap_or_else(|_| format!("/tmp/basil-{pid}"));
                let _ = fs::create_dir_all(format!("{base}/server"));
                let _ = fs::write(format!("{base}/server/server.pid.txt"), pid.to_string());
                println!("{base}");
            }
            _ => {}
        }
    }
}

fn run_build(args: &[String]) {
    let bes_path = find_flag_value(args, "--build_event_binary_file");

    // Generic fixture mode: a control fd was inherited, so read the declared
    // `BazelExpectation` off it and synthesize the BES stream from it.
    if let Some(fd) = control_fd() {
        run_generic(fd, bes_path.as_deref());
    }

    // Named-scenario mode (legacy): driven by `--scenario=<name>` in argv.
    let scenario_name =
        find_flag_value(args, "--scenario").unwrap_or_else(|| "success".to_string());
    let Some(s) = scenario(&scenario_name) else {
        eprintln!("basil: unknown scenario {scenario_name:?}");
        process::exit(2);
    };

    if let Some(path) = bes_path {
        write_scenario(&path, &s);
    }

    match s.exit {
        ExitBehavior::Code(c) => process::exit(c),
        ExitBehavior::Signal(sig) => {
            // libc(3)'s `raise` declared directly to avoid pulling in the
            // libc crate for one symbol. async-signal-safe and only
            // delivers the named signal to the current process.
            unsafe extern "C" {
                fn raise(sig: i32) -> i32;
            }
            // SAFETY: `raise` is a safe-to-call libc function with a
            // well-defined contract on every Unix.
            unsafe {
                raise(sig);
            }
            process::exit(128 + sig);
        }
    }
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
    // A small open_delay mirrors the legacy `success` scenario: it widens the
    // window for the AXL iterator's late `.subscribe()` to land before events
    // fan out on the warm-daemon path.
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
