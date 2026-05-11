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
//! Scenarios are added in `scenario`. Pick names that document the behavior
//! they exercise (`success`, `cache_evicted_no_retry`, etc.) so the AXL test
//! reads obviously: `ctx.bazel.build(flags = ["--scenario=cache_evicted_no_retry"], ...)`.

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
    let scenario_name =
        find_flag_value(args, "--scenario").unwrap_or_else(|| "success".to_string());
    let s = scenario(&scenario_name);

    if let Some(path) = bes_path {
        write_scenario(&path, &s);
    }

    match s.exit {
        ExitBehavior::Code(c) => process::exit(c),
        ExitBehavior::Signal(sig) => {
            // SAFETY: `libc::raise` is async-signal-safe and only
            // delivers the named signal to the current process.
            unsafe {
                libc::raise(sig);
            }
            // If `raise` returned (e.g. signal caught/ignored, which
            // we don't expect with SIGKILL or default handlers), fall
            // through to a non-zero exit so the caller still sees an
            // abnormal-looking outcome.
            process::exit(128 + sig);
        }
    }
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

/// How basil terminates after writing the BES event stream. `Code(n)`
/// shells out to `process::exit(n)`; `Signal(n)` raises Unix signal `n`
/// on itself so the parent's `ExitStatus::code()` is `None`, modeling
/// Bazel being killed by a signal rather than exiting cleanly.
enum ExitBehavior {
    Code(i32),
    Signal(i32),
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
///
/// `exit` controls how basil terminates after the event sequence is
/// flushed. Defaults to `Code(0)`; set explicitly to model nonzero exits
/// or signal kills.
struct Scenario {
    open_delay: Duration,
    attempts: Vec<Vec<BuildEvent>>,
    exit: ExitBehavior,
}

fn write_scenario(path: &str, scenario: &Scenario) {
    for events in &scenario.attempts {
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
            exit: ExitBehavior::Code(0),
        },

        // Regression for aspect-build/aspect-cli#1060: a single attempt with
        // REMOTE_CACHE_EVICTED (exit code 39) and last_message=true, then
        // basil exits without writing a retry attempt. axl-runtime's stream
        // sets `expecting_retry = true` on the evicted BuildFinished and
        // would otherwise loop swallowing BrokenPipe forever. The fix in
        // crates/axl-runtime/src/engine/bazel/stream/build_event.rs falls
        // through to a graceful close once it observes the writer pid is
        // dead, so this scenario must terminate the AXL build promptly.
        "cache_evicted_no_retry" => Scenario {
            open_delay: Duration::ZERO,
            attempts: vec![vec![build_started(), build_finished(39, true)]],
            exit: ExitBehavior::Code(0),
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
            exit: ExitBehavior::Code(0),
        },

        // Like `success`, but basil exits with code 2 (a genuine Bazel
        // build failure). Used by the fail_at_end-preserves-bazel-exit
        // regression test: even when the sink reports terminal failure,
        // wait() must surface code 2 rather than the synthetic 36.
        "nonzero_exit" => Scenario {
            open_delay: Duration::ZERO,
            attempts: vec![vec![build_started(), build_finished(2, true)]],
            exit: ExitBehavior::Code(2),
        },

        // Like `success`, but basil is killed by SIGKILL after the event
        // sequence is flushed. The parent's `ExitStatus::code()` is
        // `None`, which exercises the signal-kill path in `wait()`'s
        // exit-code mapping — fail_at_end must not collapse `None` into
        // the synthetic 36.
        "signal_killed_sigkill" => Scenario {
            open_delay: Duration::ZERO,
            attempts: vec![vec![build_started(), build_finished(0, true)]],
            exit: ExitBehavior::Signal(libc::SIGKILL),
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
