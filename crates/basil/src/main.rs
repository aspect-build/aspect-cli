//! basil — a fake `bazel` binary used to drive integration tests of
//! `ctx.bazel.build`. The runtime spawns whichever binary `BAZEL_REAL`
//! points at; tests point it at this one.
//!
//! Verbs:
//!   - `info <key>...`  — prints `key: value` lines; supports `server_pid`,
//!     `release`, `output_base`. The pid printed defaults to the basil
//!     process's own pid; tests can override via `BASIL_SERVER_PID` so a
//!     long-lived holder process stands in for the server whose open files
//!     the `bazel` crate's streams ask about, for the whole test.
//!   - `--version` — prints `bazel <release>`, from `BASIL_RELEASE` as `info`'s
//!     `release` is; with any other argument beside it, fails as Bazel does.
//!   - `build` / `test` — finds `--build_event_binary_file <path>`, finds
//!     `--scenario=<name>` somewhere in argv, and writes a sequence of
//!     length-delimited `BuildEvent` protobufs into the BES path according
//!     to the named scenario. Each "attempt" creates the file over, writes it
//!     and closes it, as Bazel truncates and rewrites the file for each
//!     attempt of a build it retries after a cache eviction.
//!
//!     When `--execution_log_compact_file <path>` is also present, a scenario
//!     with `execlog_entries` writes that many `ExecLogEntry` protobufs there, in
//!     the real format: one zstd frame over varint-length-prefixed messages,
//!     written as a regular file the way Bazel writes it. Entry counts above the
//!     decoded channel's 1000-entry capacity are the point — truncation past the
//!     capacity is invisible without a log longer than it.
//!
//!     When `--experimental_workspace_rules_log_file <path>` is present, a
//!     scenario with `workspace_events` writes that many length-delimited
//!     `WorkspaceEvent` protobufs there, `location` numbering them from 1; one
//!     with `workspace_corrupt` writes a single event that does not decode.
//!
//!     `BASIL_EXECLOG_PUMP_HANDSHAKE` turns that into a two-way exchange — see
//!     [`await_pump_handshake`].
//!
//!     When `--profile <path>` is present, any build writes a stand-in
//!     profile there before the client exits, as Bazel finishes its own.
//!
//!     When `--build_event_json_file <path>` is present, a scenario with
//!     `json_bep_tests` writes the loading-only test selection that
//!     `aspect cache diff` establishes its scope from — see [`write_json_bep`].
//!
//! Flags are read with `bazel::rc::last_flag_value` for the invocation's verb — the
//! same resolver the CLI decides with, so the fake sees argv the way Bazel does:
//! the `--flag=value` and `--flag value` forms alike, last occurrence winning, and
//! the `--default_override=0:<section>=…` token the CLI restates an rc entry as.
//! Each of those matters to a test. A caller clearing
//! `--execution_log_compact_file=` while the CLI appends its own is the shape the
//! reuse logic has to get right, and a fake blind to `--default_override` would
//! write no log at all for an rc-file path — reporting a working feature as a
//! missing log.
//!
//! Scenarios are added in `scenario`. Pick names that document the behavior
//! they exercise (`success`, `cache_evicted_no_retry`, etc.) so the AXL test
//! reads obviously: `ctx.bazel.build(flags = ["--scenario=cache_evicted_no_retry"], ...)`.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use axl_proto::build_event_stream::{
    ActionExecuted, BuildEvent, BuildEventId, BuildFinished, BuildStarted, File, NamedSetOfFiles,
    TestResult, TestStatus,
    build_event::Payload,
    build_event_id::{
        ActionCompletedId, BuildFinishedId, BuildStartedId, Id, NamedSetOfFilesId, TestResultId,
    },
    build_finished::ExitCode,
    file,
};
use axl_proto::tools::protos::{ExecLogEntry, exec_log_entry};
use axl_proto::workspace_log::WorkspaceEvent;
use prost::Message;

/// What basil writes at `--profile <path>`, standing in for a trace profile.
const BASIL_PROFILE: &[u8] = b"basil profile\n";

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

    if args == ["--version"] {
        println!("bazel {}", release());
        return;
    }
    if args.iter().any(|a| a == "--version") {
        eprintln!("FATAL: Unknown startup option: '--version'.");
        process::exit(2);
    }
    match verb {
        "info" => run_info(&args),
        "build" | "test" => run_build(&args, verb),
        "__async_bep_tail_writer" => run_async_bep_tail_writer(&args),
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

/// The Bazel release basil reports: `BASIL_RELEASE`, else 9.0.0.
fn release() -> String {
    env::var("BASIL_RELEASE").unwrap_or_else(|_| "9.0.0".to_string())
}

/// When a test asked for it (`BASIL_STDERR_STAMPS`), append when `line` is
/// written to stderr, as milliseconds since the epoch, a tab and the line, so
/// the test can time what the CLI does with it from basil's write on.
fn stamp_stderr_line(line: &str) {
    let Some(path) = env::var_os("BASIL_STDERR_STAMPS") else {
        return;
    };
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(format!("{ms}\t{line}\n").as_bytes());
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
    // invocation. Required for the `bazel` crate's streams, which ask the
    // server pid whether it still holds the path open, to have a live process
    // to ask across the lifetime of the test.
    let pid: u32 = bazel::rc::last_flag_value(args, "--basil-server-pid", "info")
        .and_then(|s| s.parse().ok())
        .or_else(|| env::var("BASIL_SERVER_PID").ok()?.parse().ok())
        .unwrap_or_else(process::id);
    let release = release();

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

fn run_build(args: &[String], verb: &str) {
    let flag = |name: &str| bazel::rc::last_flag_value(args, name, verb);
    let bes_path = flag("--build_event_binary_file");
    let execlog_path = flag("--execution_log_compact_file");
    // Bazel finishes the profile before its client exits; any build writes one.
    if let Some(path) = flag("--profile") {
        fs::write(&path, BASIL_PROFILE).unwrap_or_else(|e| panic!("basil: writing {path}: {e}"));
    }
    let scenario_name = flag("--scenario").unwrap_or_else(|| "success".to_string());
    if scenario_name == "async_bep_tail_after_client_exit" {
        let bes_path = bes_path.unwrap_or_else(|| {
            panic!("basil: async BEP tail scenario requires --build_event_binary_file")
        });
        let control_dir = flag("--basil-control-dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                panic!("basil: async BEP tail scenario requires --basil-control-dir")
            });
        fs::create_dir_all(&control_dir)
            .unwrap_or_else(|e| panic!("basil: creating control directory: {e}"));
        publish_control_file(&control_dir.join("bep_path"), bes_path.as_bytes());
        wait_for_file(&control_dir.join("ready"), "the server's BEP prefix");

        let upload_mode = flag("--build_event_binary_file_upload_mode").unwrap_or_default();
        publish_control_file(&control_dir.join("upload_mode"), upload_mode.as_bytes());
        if upload_mode == "fully_async" {
            thread::sleep(ASYNC_BEP_CLIENT_EXIT_DELAY);
            fs::write(control_dir.join("client_exiting"), b"")
                .unwrap_or_else(|e| panic!("basil: publishing client exit: {e}"));
        } else {
            wait_for_file(&control_dir.join("done"), "the server's BEP tail");
        }
        process::exit(0);
    }

    if let Some(dir) = flag("--basil-scratch") {
        // Where a scenario writes files for the runtime to read: a folder the
        // test owns, so they go with it.
        let _ = fs::create_dir_all(&dir);
        // The arguments it ran with, for the test to read.
        let _ = fs::write(Path::new(&dir).join("args"), args.join("\n"));
        let _ = SCRATCH.set(PathBuf::from(dir));
    }
    let s = scenario(&scenario_name);
    // The last `--color` wins, as it does for bazel.
    let colour = std::env::args()
        .filter(|a| a.starts_with("--color="))
        .last()
        .is_some_and(|a| a == "--color=yes");
    // `--curses=yes`, last wins: a progress line is erased before the next
    // line, as curses erases its progress area to write something new.
    let curses = std::env::args()
        .filter(|a| a.starts_with("--curses="))
        .last()
        .is_some_and(|a| a == "--curses=yes");
    let mut drawn = false;
    for line in s.stderr_lines {
        stamp_stderr_line(line);
        if std::mem::take(&mut drawn) {
            eprint!("\r\x1b[1A\x1b[K");
        }
        match line.strip_prefix("stdout:") {
            Some(out) => {
                println!("{out}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            None if colour => eprintln!("{}", coloured(line)),
            None => eprintln!("{line}"),
        }
        drawn = curses && is_progress(line);
    }
    if let Some(tail) = s.interrupt_tail {
        wait_for_interrupt(curses);
        if curses {
            eprint!("\r\x1b[1A\x1b[K");
        }
        thread::sleep(INTERRUPT_CLEANUP);
        eprintln!("{tail}");
        process::exit(8);
    }

    if let Some(path) = flag("--experimental_workspace_rules_log_file") {
        if s.workspace_corrupt {
            fs::write(&path, [4, 0xff, 0xff, 0xff, 0xff])
                .unwrap_or_else(|e| panic!("basil: writing workspace log {path:?}: {e}"));
        } else if s.workspace_events > 0 {
            write_workspace_log(&path, s.workspace_events);
        }
    }

    let log = || match &execlog_path {
        Some(path) if s.execlog_entries > 0 => write_execlog(path, s.execlog_entries),
        _ => {}
    };

    // Only the invocation that asked for a JSON log gets one, which is how a
    // scenario can answer a task's loading-only selection pass without also
    // speaking for the spawns that follow it.
    if let Some(path) = flag("--build_event_json_file")
        && !s.json_bep_tests.is_empty()
    {
        let exit_code = match s.exit {
            ExitBehavior::Code(c) => c,
            ExitBehavior::Signal(_) => 0,
        };
        write_json_bep(&path, s.json_bep_tests, exit_code);
    }

    // Written before the BES file: Bazel writes the execution log as it
    // executes, and the reader tails it. This is the closest a one-shot fake
    // gets to that, and the entries are on disk while the AXL side drains BES.
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

/// How long an interrupted basil "cleans up" before its last line: long
/// enough that a CLI which stopped reading on cancel misses it.
const INTERRUPT_CLEANUP: Duration = Duration::from_millis(500);

static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_sigint(_sig: i32) {
    INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Announce on stderr, then block until SIGINT (at most 60s).
/// Announce the wait and wait for a SIGINT. Under `curses`, the progress
/// area Bazel draws under its last message follows, as it would while a
/// build waits, and is erased before the next line.
fn wait_for_interrupt(curses: bool) {
    // Declared directly, like `raise` below: one symbol is not worth the libc
    // crate. SIGINT is 2 on every Unix.
    unsafe extern "C" {
        fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
    }
    // SAFETY: the handler only stores to an atomic, which is
    // async-signal-safe.
    unsafe {
        signal(2, on_sigint);
    }
    eprintln!("basil: waiting for interrupt");
    if curses {
        eprintln!("\x1b[32m[0 / 1]\x1b[0m waiting");
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while !INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
}

const ASYNC_BEP_TAIL_DELAY: Duration = Duration::from_millis(500);
const ASYNC_BEP_CLIENT_EXIT_DELAY: Duration = Duration::from_millis(500);
const ASYNC_BEP_TAIL_TIMEOUT: Duration = Duration::from_secs(5);

fn wait_for_file(path: &Path, description: &str) {
    let deadline = Instant::now() + ASYNC_BEP_TAIL_TIMEOUT;
    while !path.exists() {
        if Instant::now() >= deadline {
            panic!(
                "basil: timed out waiting for {description} at {}",
                path.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn publish_control_file(path: &Path, contents: &[u8]) {
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, contents)
        .unwrap_or_else(|e| panic!("basil: writing {}: {e}", tmp_path.display()));
    fs::rename(&tmp_path, path)
        .unwrap_or_else(|e| panic!("basil: publishing {}: {e}", path.display()));
}

fn run_async_bep_tail_writer(args: &[String]) {
    let control_dir = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("basil: async BEP tail writer requires a control directory"));
    let bep_path_file = control_dir.join("bep_path");
    wait_for_file(&bep_path_file, "the client's BEP path");
    let bep_path = fs::read_to_string(&bep_path_file)
        .unwrap_or_else(|e| panic!("basil: reading BEP path: {e}"));
    let mut writer = OpenOptions::new()
        .write(true)
        .create(true)
        .append(true)
        .open(&bep_path)
        .unwrap_or_else(|e| panic!("basil: opening async BEP path {bep_path:?}: {e}"));

    write_bep_event(&mut writer, &build_started());
    fs::write(control_dir.join("ready"), b"")
        .unwrap_or_else(|e| panic!("basil: publishing BEP prefix: {e}"));
    let upload_mode_path = control_dir.join("upload_mode");
    wait_for_file(&upload_mode_path, "the client's upload mode");
    let upload_mode = fs::read_to_string(&upload_mode_path)
        .unwrap_or_else(|e| panic!("basil: reading upload mode: {e}"));
    if upload_mode == "fully_async" {
        wait_for_file(&control_dir.join("client_exiting"), "the client's exit");
    }
    thread::sleep(ASYNC_BEP_TAIL_DELAY);
    write_bep_event(&mut writer, &build_finished(0, true));
    drop(writer);
    fs::write(control_dir.join("done"), b"")
        .unwrap_or_else(|e| panic!("basil: publishing BEP tail: {e}"));
}

fn write_bep_event(writer: &mut impl Write, event: &BuildEvent) {
    let mut buf = Vec::new();
    event
        .encode_length_delimited(&mut buf)
        .expect("basil: encode BuildEvent");
    writer
        .write_all(&buf)
        .unwrap_or_else(|e| panic!("basil: writing BEP event: {e}"));
}

/// How basil terminates after writing the BES event stream. `Code(n)`
/// shells out to `process::exit(n)`; `Signal(n)` raises Unix signal `n`
/// on itself so the parent's `ExitStatus::code()` is `None`, modeling
/// Bazel being killed by a signal rather than exiting cleanly.
enum ExitBehavior {
    Code(i32),
    Signal(i32),
}

/// One full BES interaction. Each attempt creates the file over, writes its
/// events and closes it (see [`write_scenario`]).
///
/// `exit` controls how basil terminates after the event sequence is
/// flushed. Defaults to `Code(0)`; set explicitly to model nonzero exits
/// or signal kills.
struct Scenario {
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
    /// Test labels to report through `--build_event_json_file`, when the runtime
    /// asked for one. Empty writes no JSON log at all.
    ///
    /// `aspect cache diff` establishes its scope from a loading-only `test
    /// --noanalyze` invocation and reads the *JSON* event log for it, not the
    /// binary stream — so a scenario that has to get past that selection names
    /// the labels it comes back with. See [`write_json_bep`].
    json_bep_tests: &'static [&'static str],
    /// How many `WorkspaceEvent` messages to write to
    /// `--experimental_workspace_rules_log_file`, when the runtime asked for one.
    workspace_events: u32,
    /// Write one event there that does not decode, instead.
    workspace_corrupt: bool,
    /// Console lines written to stderr before anything else, which is where
    /// real bazel writes its console. Drives the captured-output tests. A
    /// line starting `stdout:` goes to stdout instead, as real bazel writes
    /// action and test output.
    stderr_lines: &'static [&'static str],
    /// After `stderr_lines`, announce `basil: waiting for interrupt` and wait
    /// for a SIGINT, then write this line after a cleanup pause and exit 8 —
    /// what an interrupted bazel prints on its way out.
    interrupt_tail: Option<&'static str>,
}

/// Whether `line` is one of bazel's progress lines: `[1 / 2] …` or a phase.
fn is_progress(line: &str) -> bool {
    (line.starts_with('[') && line.contains(" / ") && line.contains(']'))
        || ["Loading:", "Analyzing:", "Computing main repo mapping:"]
            .iter()
            .any(|p| line.starts_with(p))
}

/// `line` with its leading severity (`WARNING: `), progress count
/// (`[1 / 2]`) or progress phase (`Loading:`) coloured, as bazel writes them
/// under `--color=yes`.
fn coloured(line: &str) -> String {
    if let Some(rest) = line.strip_prefix('[')
        && let Some((count, rest)) = rest.split_once(']')
        && count.contains(" / ")
    {
        return format!("\x1b[32m[{count}]\x1b[0m{rest}");
    }
    for phase in ["Loading:", "Analyzing:", "Computing main repo mapping:"] {
        if let Some(rest) = line.strip_prefix(phase) {
            return format!("\x1b[32m{phase}\x1b[0m{rest}");
        }
    }
    let label = line
        .split_once(": ")
        .filter(|(sev, _)| !sev.is_empty() && sev.chars().all(|c| c.is_ascii_uppercase()));
    let colour = |sev: &str| match sev {
        "INFO" => "\x1b[32m",
        "WARNING" => "\x1b[35m",
        "DEBUG" => "\x1b[33m",
        _ => "\x1b[31m\x1b[1m",
    };
    match label {
        Some((sev, rest)) => format!("{}{sev}: \x1b[0m{rest}", colour(sev)),
        None => line.to_string(),
    }
}

/// A clean single-attempt run with no pauses and no execution log — so each
/// scenario below names only the fields that make it the case it is.
impl Default for Scenario {
    fn default() -> Self {
        Self {
            attempts: vec![vec![build_started(), build_finished(0, true)]],
            exit: ExitBehavior::Code(0),
            execlog_entries: 0,
            execlog_delay: Duration::ZERO,
            json_bep_tests: &[],
            workspace_events: 0,
            workspace_corrupt: false,
            stderr_lines: &[],
            interrupt_tail: None,
        }
    }
}

/// Write the loading-only test selection `aspect cache diff` reads out of
/// `--build_event_json_file`.
///
/// `cache_selection.axl` accepts the selection only when the log is exactly the
/// shape a successful `test --noanalyze` leaves: one `expanded` event whose
/// children are the configured targets, one `NO_ANALYZE` `aborted` event per
/// target — analysis is what the flag prevented — and one `finished` event
/// carrying this invocation's own exit code, with `lastMessage` on it and nowhere
/// else. Anything short of that is a truncated log and is rejected, so there is
/// no partial form worth emitting.
fn write_json_bep(path: &str, tests: &[&str], exit_code: i32) {
    let mut out = String::new();
    let children: Vec<String> = tests
        .iter()
        .map(|label| format!(r#"{{"targetConfigured":{{"label":"{label}"}}}}"#))
        .collect();
    out.push_str(&format!(
        r#"{{"id":{{"pattern":{{"pattern":["//..."]}}}},"children":[{}],"expanded":{{}}}}"#,
        children.join(","),
    ));
    out.push('\n');
    for label in tests {
        out.push_str(&format!(
            r#"{{"id":{{"targetConfigured":{{"label":"{label}"}}}},"aborted":{{"reason":"NO_ANALYZE"}}}}"#,
        ));
        out.push('\n');
    }
    out.push_str(&format!(
        r#"{{"id":{{"buildFinished":{{}}}},"finished":{{"exitCode":{{"code":{exit_code}}}}},"lastMessage":true}}"#,
    ));
    out.push('\n');
    fs::write(path, out).unwrap_or_else(|e| panic!("basil: writing JSON BEP to {path:?}: {e}"));
}

/// Write each attempt as Bazel does: create the file over (`O_TRUNC`), write
/// the attempt's events, close it. Attempts after the first wait until the
/// reader has moved the last one's file off the path (see [`await_moved`]).
fn write_scenario(path: &str, scenario: &Scenario, handshake: Option<&str>) {
    let last_attempt = scenario.attempts.len().saturating_sub(1);
    for (attempt, events) in scenario.attempts.iter().enumerate() {
        if attempt > 0 {
            await_moved(path);
        }
        let mut f = fs::File::create(path)
            .unwrap_or_else(|e| panic!("basil: creating BES path {path:?}: {e}"));
        for (i, ev) in events.iter().enumerate() {
            // Holding back the last event of the last attempt is what keeps the
            // consumer's drain loop alive for the handshake.
            if let Some(file) = handshake {
                if attempt == last_attempt && i + 1 == events.len() {
                    await_pump_handshake(file);
                }
            }
            write_bep_event(&mut f, ev);
        }
    }
}

/// Wait, for at most [`ATTEMPT_TIMEOUT`], until the reader has moved the
/// last attempt's file away from `path`, which it does as it opens it. The
/// reader starts after basil does, and a truncation before that would lose the
/// attempt; it need not have read any of it. Bazel's first attempt, from server
/// connect to its final event, is far longer than the reader's start.
fn await_moved(path: &str) {
    let deadline = Instant::now() + ATTEMPT_TIMEOUT;
    while Path::new(path).exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
}

/// Entries or events in each log scenario: past the decoded channel's
/// 1000-entry capacity, which is the only length that can tell a stream that
/// delivers everything apart from one that stops at the ring, and still a
/// fraction of a second to write.
const BEYOND_CAPACITY: u32 = 3000;

/// See [`await_moved`]: long enough for a loaded machine, short enough that a
/// reader that never comes does not hang the test.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);

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
/// Built under a sibling `.partial` name and renamed into place, so a reader
/// sees either no file yet or the whole thing, never a log cut off mid-write.
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

/// Write `count` workspace events to `path`, built under a sibling `.partial`
/// name and renamed into place as [`write_execlog`] does.
fn write_workspace_log(path: &str, count: u32) {
    let partial = format!("{path}.partial");
    let bytes: Vec<u8> = (1..=count)
        .flat_map(|n| {
            WorkspaceEvent {
                location: n.to_string(),
                ..Default::default()
            }
            .encode_length_delimited_to_vec()
        })
        .collect();
    fs::write(&partial, bytes)
        .unwrap_or_else(|e| panic!("basil: writing workspace log {partial:?}: {e}"));
    fs::rename(&partial, path)
        .unwrap_or_else(|e| panic!("basil: publishing workspace log to {path:?}: {e}"));
}

/// Where a scenario writes the files it reports (see `run_build`).
static SCRATCH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Resolve a scenario by name. Each scenario documents the behavior or bug
/// it targets. Add new ones here.
fn scenario(name: &str) -> Scenario {
    match name {
        // Clean run: one attempt, terminates with last_message=true.
        "success" => Scenario::default(),

        // Prints its last line only after the CLI cancels it, for the
        // captured console's drain on cancellation.
        "stderr_after_interrupt" => Scenario {
            interrupt_tail: Some("INFO: cleaned up after the interrupt"),
            ..Default::default()
        },

        // `success` with console lines the default stages act on, or leave
        // alone: similar warnings, a sandbox path, `canonical reproducible
        // form` advice (shown, since nothing is dropped by default), a
        // recognized failure, a progress line, and two test
        // actions' output, which must never collapse.
        "console_noise" => Scenario {
            stderr_lines: &[
                "WARNING: errors encountered while analyzing target '//:g0', it will not be built.",
                "Analysis failed",
                "DEBUG: Rule 'repo' indicated that a canonical reproducible form can be obtained by modifying arguments shallow_since = \"1 +0000\"",
                "DEBUG: Repo '+ext+bar' indicated that a canonical reproducible form can be obtained by modifying arguments integrity = \"sha256-x\"",
                "DEBUG: Repo +ext+bar defined by rule http_archive in @@bazel_tools//tools/build_defs/repo:http.bzl",
                "DEBUG: Repository _main~ext~foo instantiated at:",
                "  <builtin>: in <toplevel>",
                "Repository rule http_archive defined at:",
                "  /x/external/bazel_tools/tools/build_defs/repo/http.bzl:400:31: in <toplevel>",
                "WARNING: errors encountered while analyzing target '//:g1', it will not be built.",
                "Analysis failed",
                "WARNING: errors encountered while analyzing target '//:g2', it will not be built.",
                "Analysis failed",
                "INFO: From Compiling src/a.ts:",
                "src/a.ts:3:4 from /tmp/_bazel_u/h/sandbox/linux-sandbox/7/execroot/_main/src/a.ts:3:4",
                "Server terminated abruptly (error code: 14, error message: 'Socket closed')",
                "[1 / 2] Compiling src/a.ts; 0s linux-sandbox",
                "INFO: From Testing //pkg:a_test:",
                "==== Test output for //pkg:a_test:",
                "assertion a failed",
                "INFO: From Testing //pkg:b_test:",
                "==== Test output for //pkg:b_test:",
                "assertion b failed",
                "INFO: Build completed successfully",
            ],
            ..Default::default()
        },

        // A test's output on stdout between the stderr lines around it, as
        // bazel writes it: merged, they must stay in this order.
        "merged_console" => Scenario {
            stderr_lines: &[
                "INFO: From Testing //pkg:t:",
                "stdout:==== Test output for //pkg:t:",
                "stdout:failing output",
                "FAIL: //pkg:t (see /tmp/test.log)",
                "INFO: Build completed successfully",
            ],
            ..Default::default()
        },

        // `success` with console chatter on stderr, for the captured-output
        // pipeline: a warning, a repeated line and a credential to rewrite.
        "stderr_chatter" => Scenario {
            stderr_lines: &[
                "INFO: Analyzed 1 target",
                "WARNING: deprecated flag used",
                "WARNING: deprecated flag used",
                "password=hunter2 leaked",
                "INFO: Build completed successfully",
            ],
            ..Default::default()
        },

        // A single attempt with REMOTE_CACHE_EVICTED (exit code 39) and
        // last_message=true, then basil exits without a retry. After the retry
        // marker the reader waits for another attempt; it has to end once the
        // client is gone, so this scenario must end the AXL build promptly
        // (aspect-build/aspect-cli#1060).
        "cache_evicted_no_retry" => Scenario {
            attempts: vec![vec![build_started(), build_finished(39, true)]],
            ..Default::default()
        },

        // An eviction on the first invocation only, for a caller that retries
        // with Bazel's own retries off (the CLI's loops): the first exits 39,
        // and every later one succeeds. `BASIL_EVICTED_ONCE` names the file
        // that marks the first one done, so the state outlives each process.
        "cache_evicted_once" => {
            let marker = env::var_os("BASIL_EVICTED_ONCE")
                .unwrap_or_else(|| panic!("basil: cache_evicted_once needs BASIL_EVICTED_ONCE"));
            if Path::new(&marker).exists() {
                return Scenario::default();
            }
            fs::write(&marker, b"").unwrap_or_else(|e| panic!("basil: writing {marker:?}: {e}"));
            Scenario {
                attempts: vec![vec![build_started(), build_finished(39, true)]],
                exit: ExitBehavior::Code(39),
                ..Default::default()
            }
        }

        // Reference scenario: REMOTE_CACHE_EVICTED followed by a successful
        // retry, each attempt starting the file over as Bazel's do.
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
        // created the BEP file, so no attempts at all. The read side waits for
        // a file that cannot come, and ends when the client does.
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
        // `BEYOND_CAPACITY` is comfortably past it.
        "execlog_beyond_capacity" => Scenario {
            execlog_entries: BEYOND_CAPACITY,
            ..Default::default()
        },

        // A workspace event log longer than the decoded channel's capacity, for
        // the same reason.
        "workspace_events_beyond_capacity" => Scenario {
            workspace_events: BEYOND_CAPACITY,
            ..Default::default()
        },

        // A workspace log that fails to decode, beside an execution log longer
        // than the channel's capacity: the failed workspace log must not keep
        // `wait()` from releasing the execution log's handles.
        "corrupt_workspace_log_beside_execlog" => Scenario {
            workspace_corrupt: true,
            execlog_entries: BEYOND_CAPACITY,
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
            execlog_entries: BEYOND_CAPACITY,
            execlog_delay: Duration::from_secs(2),
            ..Default::default()
        },

        // A log of the same shape, on an invocation Bazel fails with
        // BLAZE_INTERNAL_ERROR (37) — which `BazelTrait.build_retry` retries by
        // default, so a task driving this runs its whole spawn → drain → wait
        // cycle once per attempt. Each attempt writes its own log, numbered from
        // 1 again, so a consumer that receives the whole log twice has proved that
        // every attempt got a live handle of its own.
        "execlog_retryable_failure" => Scenario {
            attempts: vec![vec![build_started(), build_finished(37, true)]],
            exit: ExitBehavior::Code(37),
            execlog_entries: BEYOND_CAPACITY,
            ..Default::default()
        },

        // `aspect cache diff --mode=precise`: three invocations, of which only
        // the precise pre-pass executes actions and so is the only one wired to
        // `exec_log_event`. Reaching it means getting past the loading-only
        // selection pass first, which is what `json_bep_tests` answers; the log
        // here is the pre-pass's own. The probe passes that follow need a
        // `--remote_grpc_log` no fake Bazel produces, so the task fails after
        // the part under test, as it does for `delivery`.
        "cache_diff_precise" => Scenario {
            execlog_entries: BEYOND_CAPACITY,
            json_bep_tests: &["//fixture:runs"],
            ..Default::default()
        },

        // An `ActionExecuted` — the payload whose accepted `kinds=` spelling
        // used to disagree with `type(event.payload)` — beside a
        // `NamedSetOfFiles` whose spellings always agreed, so a filter can be
        // asserted to deliver exactly the kind it names and nothing else.
        // Real bazel only publishes `ActionExecuted` for successful actions
        // under `--build_event_publish_all_actions`, which is part of why the
        // mismatch was hard to see in a live build.
        "action_and_named_set" => Scenario {
            attempts: vec![vec![
                build_started(),
                action_executed(),
                named_set_of_files(),
                build_finished(0, true),
            ]],
            exit: ExitBehavior::Code(0),
            ..Default::default()
        },

        // A failed action whose command line, 2000 arguments of 100 bytes,
        // is past any event size limit from 64 KiB to 128 KiB but within the
        // default 4 MiB, so how much of it survives says which limit the
        // stream was held to.
        "oversized_command_line" => Scenario {
            attempts: vec![vec![
                build_started(),
                failed_action_with_command_line(2000, 100),
                build_finished(1, true),
            ]],
            exit: ExitBehavior::Code(1),
            ..Default::default()
        },

        // Two actions and two tests whose logs are real files, as Bazel names
        // them: a failed action with stderr and a successful one with stdout
        // (Bazel reports that one only under `--build_event_publish_all_actions`),
        // a failed and a passing test attempt with a `test.log`. For the
        // history's log keeping.
        // `action_and_test_logs` with Bazel's console for it: the failed
        // action's error, its stderr relayed under it, Bazel's progress and
        // its closing line.
        "action_and_test_logs_console" => Scenario {
            stderr_lines: &[
                "INFO: Analyzed 4 targets.",
                "ERROR: /ws/pkg/BUILD:3:8: Executing genrule //pkg:broken failed: (Exit 1): bash failed: error executing Genrule command",
                "ERROR: broken",
                "[3 / 4] Testing //pkg:fails",
                "ERROR: Build did NOT complete successfully",
            ],
            ..scenario("action_and_test_logs")
        },

        "action_and_test_logs" => {
            let dir = SCRATCH
                .get()
                .cloned()
                .unwrap_or_else(|| env::temp_dir().join(format!("basil-logs-{}", process::id())));
            fs::create_dir_all(&dir).expect("basil: log dir");
            let log = |name: &str, text: &str| -> File {
                let path = dir.join(name);
                fs::write(&path, text).expect("basil: writing a log");
                File {
                    name: name.to_string(),
                    file: Some(file::File::Uri(format!("file://{}", path.display()))),
                    ..Default::default()
                }
            };
            Scenario {
                attempts: vec![vec![
                    build_started(),
                    action_with_logs(
                        "//pkg:broken",
                        false,
                        None,
                        Some(log("stderr-1", "ERROR: broken\n")),
                    ),
                    action_with_logs("//pkg:fine", true, Some(log("stdout-2", "fine\n")), None),
                    test_result_with_log(
                        "//pkg:fails",
                        TestStatus::Failed,
                        log("test.log", "FAIL\n"),
                    ),
                    test_result_with_log(
                        "//pkg:passes",
                        TestStatus::Passed,
                        log("pass.log", "PASS\n"),
                    ),
                    build_finished(1, true),
                ]],
                exit: ExitBehavior::Code(1),
                ..Default::default()
            }
        }

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

/// An `ActionExecuted` payload carried on an `action_completed` id — the
/// shape that makes `type(event.payload)` and `event.kind` disagree.
fn action_executed() -> BuildEvent {
    BuildEvent {
        id: Some(BuildEventId {
            id: Some(Id::ActionCompleted(ActionCompletedId {
                label: "//pkg:target".to_string(),
                ..Default::default()
            })),
        }),
        last_message: false,
        payload: Some(Payload::Action(ActionExecuted {
            success: true,
            r#type: "Genrule".to_string(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// A failed `Genrule` action whose command line is `args` arguments of
/// `arg_bytes` bytes each.
fn failed_action_with_command_line(args: usize, arg_bytes: usize) -> BuildEvent {
    let mut event = action_with_logs("//pkg:huge", false, None, None);
    if let Some(Payload::Action(action)) = &mut event.payload {
        action.command_line = (0..args).map(|_| "x".repeat(arg_bytes)).collect();
    }
    event
}

/// A failed or successful `Genrule` action for `label`, with its logs.
fn action_with_logs(
    label: &str,
    success: bool,
    stdout: Option<File>,
    stderr: Option<File>,
) -> BuildEvent {
    BuildEvent {
        id: Some(BuildEventId {
            id: Some(Id::ActionCompleted(ActionCompletedId {
                label: label.to_string(),
                primary_output: format!(
                    "bazel-out/bin/{}.out",
                    label.rsplit(':').next().unwrap_or("")
                ),
                ..Default::default()
            })),
        }),
        last_message: false,
        payload: Some(Payload::Action(ActionExecuted {
            success,
            r#type: "Genrule".to_string(),
            exit_code: if success { 0 } else { 1 },
            stdout,
            stderr,
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// One test attempt for `label`, with `log` as its only output.
fn test_result_with_log(label: &str, status: TestStatus, log: File) -> BuildEvent {
    BuildEvent {
        id: Some(BuildEventId {
            id: Some(Id::TestResult(TestResultId {
                label: label.to_string(),
                run: 1,
                shard: 1,
                attempt: 1,
                ..Default::default()
            })),
        }),
        last_message: false,
        payload: Some(Payload::TestResult(TestResult {
            status: status as i32,
            test_action_output: vec![File {
                name: "test.log".to_string(),
                ..log
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn named_set_of_files() -> BuildEvent {
    BuildEvent {
        id: Some(BuildEventId {
            id: Some(Id::NamedSet(NamedSetOfFilesId {
                id: "0".to_string(),
            })),
        }),
        last_message: false,
        payload: Some(Payload::NamedSetOfFiles(NamedSetOfFiles::default())),
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
