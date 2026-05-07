//! basil — a fake `bazel` binary used to drive integration tests of
//! `ctx.bazel.build`. The runtime spawns whichever binary `BAZEL_REAL`
//! points at; tests point it at this one.
//!
//! Verbs:
//!   - `info <key>...`  — prints `key: value` lines; supports `server_pid`,
//!     `release`, `output_base`. The pid printed defaults to the basil
//!     process's own pid; tests can override via `BASIL_SERVER_PID` so a
//!     long-lived holder process keeps galvanize's `IfOpenForPid` retry
//!     check satisfied for the whole test.
//!   - `build` / `test` — finds `--build_event_binary_file <path>`, finds
//!     `--scenario=<name>` somewhere in argv, and writes a sequence of
//!     length-delimited `BuildEvent` protobufs into the BES path according
//!     to the named scenario. Each "attempt" is one open/write/close cycle
//!     on the path, so multi-attempt scenarios faithfully simulate Bazel's
//!     reconnect-after-eviction behavior on a FIFO.
//!
//! Scenarios are added in `scenario_attempts`. Pick names that document the
//! bug or behavior they exercise (`bug1`, `success`, etc.) so the AXL test
//! reads obviously: `ctx.bazel.build(flags = ["--scenario=bug1"], ...)`.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process;
use std::thread;
use std::time::Duration;

use axl_proto::build_event_stream::{
    BuildEvent, BuildEventId, BuildFinished, BuildStarted,
    build_event::Payload,
    build_event_id::{BuildFinishedId, BuildStartedId, Id},
    build_finished::ExitCode,
};
use prost::Message;

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
    // Honors BASIL_SERVER_PID so tests can pin the reported pid to a holder
    // process (e.g. `sleep 60`) that outlives basil's own short-lived info
    // invocation. Required for galvanize's IfOpenForPid retry loop to keep
    // the FIFO read end open across the lifetime of the test.
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
    let scenario = find_flag_value(args, "--scenario").unwrap_or_else(|| "success".to_string());

    if let Some(path) = bes_path {
        write_scenario(&path, &scenario);
    }

    let exit_code: i32 = env::var("BASIL_BUILD_EXIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    process::exit(exit_code);
}

/// Finds `--name <value>` or `--name=<value>` in argv. The runtime emits both
/// forms (`--build_event_binary_file <path>` for paths, `--scenario=foo` for
/// user-supplied flags), so handling both keeps us tolerant.
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

/// One full BES interaction. Each attempt is one open/write/close cycle on
/// the FIFO. `open_delay` sleeps after the FIFO open and before any writes —
/// only set this on scenarios whose tests assert on what the AXL iterator
/// received. `build.build_events()` subscribes after `Build::spawn` returns
/// (the broadcaster doesn't replay history; see
/// `crates/axl-runtime/src/engine/bazel/stream/broadcaster.rs:271`), so a
/// pause widens the window for that subscribe to land before basil starts
/// fanning out events. Scenarios whose tests only check `build.wait()`
/// status don't need it — leave at zero.
struct Scenario {
    open_delay: Duration,
    attempts: Vec<Vec<BuildEvent>>,
}

fn write_scenario(path: &str, name: &str) {
    let scenario = scenario(name);
    for events in scenario.attempts {
        // One open/write/close per attempt: the read side observes a writer
        // appear, drain bytes, and disappear — same as Bazel reopening the
        // BEP file on each retry.
        let mut f = OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap_or_else(|e| panic!("basil: opening BES path {path:?} for write: {e}"));
        if !scenario.open_delay.is_zero() {
            thread::sleep(scenario.open_delay);
        }
        for ev in events {
            let mut buf = Vec::new();
            ev.encode_length_delimited(&mut buf)
                .expect("basil: encode BuildEvent");
            f.write_all(&buf)
                .unwrap_or_else(|e| panic!("basil: writing to BES path: {e}"));
        }
    }
}

/// Resolve a scenario by name. Each scenario documents the behavior or bug
/// it targets. Add new ones here.
fn scenario(name: &str) -> Scenario {
    match name {
        // Clean run: one attempt, terminates with last_message=true.
        // 50ms open_delay so AXL's `for event in build.build_events()` (a late
        // subscriber by API shape) lands its subscription before basil starts
        // fanning out events. Without this, the iterator races the producer
        // and yields zero events.
        "success" => Scenario {
            open_delay: Duration::from_millis(50),
            attempts: vec![vec![build_started(), build_finished(0, true)]],
        },

        // Bug 1: REMOTE_CACHE_EVICTED (exit code 39) on the *only* attempt.
        // Bazel emits last_message=true on the evicted BuildFinished, so
        // axl-runtime's stream sets `expecting_retry = true`. No retry ever
        // arrives. Today the stream loops swallowing BrokenPipe forever and
        // never closes its broadcaster — `for event in build.build_events()`
        // and `build.wait()` both hang. Track in:
        // crates/axl-runtime/src/engine/bazel/stream/build_event.rs:113
        "bug1" => Scenario {
            open_delay: Duration::ZERO,
            attempts: vec![vec![build_started(), build_finished(39, true)]],
        },

        // Reference scenario: REMOTE_CACHE_EVICTED followed by a successful
        // retry. Two attempts, one open/write/close each. Matches Bazel's
        // real reconnect-after-eviction shape and exercises the
        // `expecting_retry` swallow-BrokenPipe-and-keep-reading path.
        "cache_evicted_with_retry" => Scenario {
            open_delay: Duration::ZERO,
            attempts: vec![
                vec![build_started(), build_finished(39, false)],
                vec![build_started(), build_finished(0, true)],
            ],
        },

        other => {
            eprintln!("basil: unknown scenario {other:?}");
            process::exit(2);
        }
    }
}

fn build_started() -> BuildEvent {
    BuildEvent {
        // `id` is required for AXL's `event.kind` accessor (renamed from
        // `last_message` in axl-proto/build.rs) — it unwraps both
        // BuildEvent.id and BuildEventId.id.
        id: Some(BuildEventId {
            id: Some(Id::Started(BuildStartedId {})),
        }),
        last_message: false,
        payload: Some(Payload::Started(BuildStarted::default())),
        ..Default::default()
    }
}

fn build_finished(code: i32, last: bool) -> BuildEvent {
    BuildEvent {
        id: Some(BuildEventId {
            id: Some(Id::BuildFinished(BuildFinishedId {})),
        }),
        last_message: last,
        payload: Some(Payload::Finished(BuildFinished {
            exit_code: Some(ExitCode {
                code,
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}
