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
//!     When `--execution_log_compact_file <path>` is also present, a scenario
//!     with `execlog_entries` writes that many `ExecLogEntry` protobufs there,
//!     in the real format: one zstd frame over varint-length-prefixed messages,
//!     written as a regular file the way Bazel writes it. Entry counts above the
//!     decoded channel's 1000-entry capacity are the point — truncation past the
//!     capacity is invisible without a log longer than it.
//!
//!     `BASIL_EXECLOG_PUMP_HANDSHAKE` turns that into a two-way exchange — see
//!     [`await_pump_handshake`].
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
use axl_proto::tools::protos::{ExecLogEntry, exec_log_entry};
use prost::Message;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    record_argv(&args);
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

/// Append this invocation's argv to `BASIL_ARGV_LOG`, one line per invocation,
/// when a test asked for it.
///
/// The CLI runs `bazel info server_pid` before `bazel build` and nominates that
/// pid as the execution log's holder. Bazel kills a running server whose startup
/// options differ from the ones it is handed, so the two invocations have to
/// agree or the pid belongs to a server the build replaces — and the only place
/// that agreement is visible is the argv each one received.
///
/// Appends rather than truncates, because one build is several invocations, and
/// a reader is expected to pick out the lines it cares about.
fn record_argv(args: &[String]) {
    let Ok(path) = env::var("BASIL_ARGV_LOG") else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all((args.join(" ") + "\n").as_bytes());
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
    let execlog_path = find_flag_value(args, "--execution_log_compact_file");
    let scenario_name =
        find_flag_value(args, "--scenario").unwrap_or_else(|| "success".to_string());
    let s = scenario(&scenario_name);

    let log = || match &execlog_path {
        Some(path) if s.execlog_entries > 0 => write_execlog(path, s.execlog_entries),
        _ => {}
    };

    // Before the BES write, which blocks on a FIFO until the reader opens it.
    // Bazel writes the execution log as it executes, i.e. during the build, and
    // the reader tails it; writing it first here is the closest a one-shot fake
    // gets to that, and it means the entries are already on disk while the AXL
    // side is draining BES.
    if s.execlog_delay.is_zero() {
        log();
    }

    // The handshake needs a log the consumer can read before the event stream
    // ends — meaningless without a log at all, and unsatisfiable for a scenario
    // that deliberately publishes one afterwards.
    let handshake = if s.execlog_entries > 0 && s.execlog_delay.is_zero() {
        env::var("BASIL_EXECLOG_PUMP_HANDSHAKE").ok()
    } else {
        None
    };

    if let Some(path) = bes_path {
        write_scenario(&path, &s, handshake.as_deref());
    }

    if !s.execlog_delay.is_zero() {
        thread::sleep(s.execlog_delay);
        log();
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
///
/// Last occurrence wins, as Bazel resolves a single-valued flag given more than
/// once. That is not a detail here: a caller clearing
/// `--execution_log_compact_file=` and the CLI appending its own is exactly the
/// shape the reuse logic has to get right, and a first-wins fake would answer
/// the opposite of the thing under test.
fn find_flag_value(args: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    let mut found = None;
    for (i, a) in args.iter().enumerate() {
        if a == name {
            // A bare `--name` immediately before another flag names no value.
            found = args.get(i + 1).filter(|v| !v.starts_with('-')).cloned();
        } else if let Some(v) = a.strip_prefix(&prefix) {
            found = Some(v.to_string());
        }
    }
    found
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
    /// How many `ExecLogEntry` messages to write to
    /// `--execution_log_compact_file`, when the runtime asked for one. Zero
    /// writes no log at all, which is what a build that ran no actions does.
    execlog_entries: u32,
    /// How long after the event stream closes to publish that log. Zero
    /// publishes it before BES, where a consumer can read it during the build;
    /// non-zero puts it out of reach of anything but an end-of-build drain.
    execlog_delay: Duration,
}

/// A clean single-attempt run with no pauses and no execution log — so each
/// scenario below names only the fields that make it the case it is.
impl Default for Scenario {
    fn default() -> Self {
        Self {
            open_delay: Duration::ZERO,
            attempts: vec![vec![build_started(), build_finished(0, true)]],
            exit: ExitBehavior::Code(0),
            execlog_entries: 0,
            execlog_delay: Duration::ZERO,
        }
    }
}

fn write_scenario(path: &str, scenario: &Scenario, handshake: Option<&str>) {
    let last_attempt = scenario.attempts.len().saturating_sub(1);
    for (attempt, events) in scenario.attempts.iter().enumerate() {
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
        for (i, ev) in events.iter().enumerate() {
            // Holding back the last event of the last attempt is what keeps the
            // consumer's drain loop alive for the handshake.
            if let Some(file) = handshake {
                if attempt == last_attempt && i + 1 == events.len() {
                    await_pump_handshake(file);
                }
            }
            let mut buf = Vec::new();
            ev.encode_length_delimited(&mut buf)
                .expect("basil: encode BuildEvent");
            f.write_all(&buf)
                .unwrap_or_else(|e| panic!("basil: writing to BES path: {e}"));
        }
    }
}

/// How long [`await_pump_handshake`] waits for the consumer. Long enough that a
/// loaded machine cannot time out a working handshake, short enough that a
/// broken one reports rather than hangs out the test runner's own limit.
const PUMP_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

/// Wait for `file` to appear, then record whether it did in `{file}.status`.
///
/// Set `BASIL_EXECLOG_PUMP_HANDSHAKE=<file>` and have the consumer create
/// `<file>` the first time an execution log entry reaches it. basil then holds
/// back the final build event until that happens, which makes "the consumer
/// dispatched an entry *during* the build" an observable fact rather than a
/// race: the only way the consumer can see an entry before the event stream
/// ends is from inside its own drain loop.
///
/// `{file}.status` holds `pumped` or `timeout`, so the absence of a mid-build
/// dispatch is a specific assertion failure instead of a test-wide hang. A
/// consumer that only drains after the stream closes cannot write `<file>`
/// until basil gives up, which is exactly what `timeout` records.
fn await_pump_handshake(file: &str) {
    let deadline = std::time::Instant::now() + PUMP_HANDSHAKE_TIMEOUT;
    let mut status = "timeout";
    while std::time::Instant::now() < deadline {
        if fs::metadata(file).is_ok() {
            status = "pumped";
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let path = format!("{file}.status");
    fs::write(&path, format!("{status}\n"))
        .unwrap_or_else(|e| panic!("basil: writing handshake status {path:?}: {e}"));
}

/// Write `count` `ExecLogEntry` messages to `path` in the compact execution log
/// format: one zstd frame over varint-length-prefixed protobufs.
///
/// The entries are `file` records, which is the cheapest kind to synthesize and
/// enough for a consumer to count and to tell apart by `id`. `id` runs from 1 so
/// a test can assert it received a contiguous `1..=count` and catch a prefix.
///
/// Built under a sibling `.partial` name and renamed into place, so the path
/// appears atomically. A real Bazel daemon holds the log open while it writes, so
/// a reader that reaches the current end of file is told to wait; here the writer
/// is this short-lived process and the nominated holder is the daemon stand-in,
/// which never opens it. A reader arriving between `create` and the last write
/// would therefore see an empty file, be told the holder has closed it, and fail
/// to read even a zstd header — yielding zero entries perhaps half the time. The
/// rename means a reader sees either no file yet (and keeps polling, since this
/// process is alive) or the whole thing.
fn write_execlog(path: &str, count: u32) {
    let partial = format!("{path}.partial");
    let file = fs::File::create(&partial)
        .unwrap_or_else(|e| panic!("basil: creating execlog path {partial:?}: {e}"));
    let mut encoder = zstd::Encoder::new(file, 0).expect("basil: zstd encoder");
    for id in 1..=count {
        let entry = ExecLogEntry {
            id,
            r#type: Some(exec_log_entry::Type::File(exec_log_entry::File {
                path: format!("basil/f{id}.txt"),
                digest: None,
            })),
        };
        encoder
            .write_all(&entry.encode_length_delimited_to_vec())
            .unwrap_or_else(|e| panic!("basil: writing execlog: {e}"));
    }
    let mut file = encoder.finish().expect("basil: finishing zstd frame");
    file.flush().expect("basil: flushing execlog");
    drop(file);
    fs::rename(&partial, path)
        .unwrap_or_else(|e| panic!("basil: publishing execlog to {path:?}: {e}"));
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
            ..Default::default()
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
            attempts: vec![vec![build_started(), build_finished(39, true)]],
            ..Default::default()
        },

        // Reference scenario: REMOTE_CACHE_EVICTED followed by a successful
        // retry. Two attempts, one open/write/close each. Matches Bazel's
        // real reconnect-after-eviction shape and exercises the
        // `expecting_retry` swallow-BrokenPipe-and-keep-reading path.
        "cache_evicted_with_retry" => Scenario {
            attempts: vec![
                vec![build_started(), build_finished(39, false)],
                vec![build_started(), build_finished(0, true)],
            ],
            ..Default::default()
        },

        // Like `success`, but basil exits with code 2 (a genuine Bazel
        // build failure). Used by the fail_at_end-preserves-bazel-exit
        // regression test: even when the sink reports terminal failure,
        // wait() must surface code 2 rather than the synthetic 36.
        "nonzero_exit" => Scenario {
            attempts: vec![vec![build_started(), build_finished(2, true)]],
            exit: ExitBehavior::Code(2),
            ..Default::default()
        },

        // Bazel rejecting the command line: it exits nonzero having never
        // opened the BEP file, so no attempts at all. The read side waits on
        // a writer that cannot come, which is what `spawn_open_watchdog`
        // exists to break out of.
        "rejects_command_line" => Scenario {
            attempts: vec![],
            exit: ExitBehavior::Code(2),
            ..Default::default()
        },

        // Like `success`, but basil is killed by SIGKILL after the event
        // sequence is flushed. The parent's `ExitStatus::code()` is
        // `None`, which exercises the signal-kill path in `wait()`'s
        // exit-code mapping — fail_at_end must not collapse `None` into
        // the synthetic 36.
        "signal_killed_sigkill" => Scenario {
            // SIGKILL: signal 9 on every Unix. Hard-coded to avoid a
            // libc dep for a single constant.
            exit: ExitBehavior::Signal(9),
            ..Default::default()
        },

        // A clean run that also writes a compact execution log longer than the
        // decoded channel's capacity. A shorter log cannot tell a stream that
        // delivers everything apart from one that silently stops at the capacity;
        // 3000 is comfortably past it and still a fraction of a second to write.
        //
        // 50ms open_delay as in `success`, so an AXL iterator that subscribes
        // after the spawn is not racing the BES burst.
        "execlog_beyond_capacity" => Scenario {
            open_delay: Duration::from_millis(50),
            execlog_entries: 3000,
            ..Default::default()
        },

        // The same log, published only after the event stream has closed. A
        // task's drain loop ends with that stream, so nothing it does can read
        // this log — only the end-of-build drain (`exec_log.close`, before
        // `wait()`) can, which is what makes a missing or misplaced drain
        // visible as an empty hook rather than as a coin flip.
        //
        // Two seconds is an order of magnitude more than the 250ms tick a task
        // takes to notice the stream ended, so the ordering does not depend on
        // scheduling. Bazel is slower than this to finish a build after its last
        // build event in practice.
        "execlog_after_bes" => Scenario {
            execlog_entries: 3000,
            execlog_delay: Duration::from_secs(2),
            ..Default::default()
        },

        // A log of the same shape, on an invocation Bazel fails with
        // BLAZE_INTERNAL_ERROR (37) — which `BazelTrait.build_retry` retries by
        // default, so a task driving this runs its whole spawn → drain → wait
        // cycle once per attempt. Each attempt writes its own log, numbered from
        // 1 again, so a consumer that receives `1..=3000` twice has proved that
        // every attempt got a live handle of its own.
        "execlog_retryable_failure" => Scenario {
            open_delay: Duration::from_millis(50),
            attempts: vec![vec![build_started(), build_finished(37, true)]],
            exit: ExitBehavior::Code(37),
            execlog_entries: 3000,
            ..Default::default()
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
