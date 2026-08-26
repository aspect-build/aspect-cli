//! Bazel server health check and its recovery ladder.
//!
//! The probe is `bazel [startup_flags] --noblock_for_lock info server_pid`.
//! Bazel has two locks that can turn that probe into exit code 9
//! (`LOCK_HELD_NOBLOCK_FOR_LOCK`), and they need opposite treatment:
//!
//! * The **output base lock**, `<output_base>/lock`, an fcntl lock the bazel
//!   *client* holds for the whole command. Bazel prints `Another command holds
//!   the output base lock:` followed by the holder's `pid=`/`cwd=` lines, then
//!   `Exiting because the ... lock is held and --noblock_for_lock was given.`
//!   The kernel drops fcntl locks on process exit, so a held lock means a
//!   *live* client. On a single-tenant runner that is a leftover from a
//!   cancelled job that is still unwinding. Killing the server does not
//!   release its lock; killing the client does.
//! * The **server command lock**, held inside the JVM while a command runs.
//!   Bazel prints `Another command (pid=N) is running. Exiting immediately.`
//!   The client only reaches the server after taking the output base lock,
//!   so this case means the other client is already gone and its command is
//!   orphaned in the server. SIGKILLing the server is correct and the next
//!   probe starts a fresh one.
//!
//! [`run`] therefore climbs a ladder: wait for a client holder to finish on
//! its own, SIGINT it (Bazel's graceful cancel, as in `live.rs`), SIGKILL it,
//! and only then kill a wedged server and re-probe. A new holder taking the
//! lock mid-ladder gets a fresh ladder, a bounded number of times. Each rung
//! logs what it found and what it is about to do, so a job log explains a
//! slow or failed health check. The wait windows are in [`Timing`]; tests
//! shrink them.
//!
//! On success the check also removes stranded sandbox state from a prior
//! SIGKILL'd invocation (bazelbuild/bazel#23880).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, ValueLike};

/// Bazel's exit code when a lock is held and `--noblock_for_lock` was given.
const LOCK_HELD_NOBLOCK_FOR_LOCK: i32 = 9;

/// Exit codes, other than a held lock, that suggest a wedged server worth a
/// SIGKILL and one re-probe. Anything else is a configuration problem the
/// health check cannot fix.
const WEDGED_SERVER_EXIT_CODES: &[i32] = &[
    1,  // Build or parsing failure
    36, // Local environmental error
    37, // Blaze internal error
];

/// The wait budget for each rung of the client-lock ladder.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Interval between probes while waiting for a lock to clear.
    pub poll: Duration,
    /// How long a live client holder gets to finish on its own before SIGINT.
    pub graceful: Duration,
    /// How long after SIGINT before SIGKILL.
    pub after_sigint: Duration,
    /// How long after SIGKILL before giving up on the lock.
    pub after_sigkill: Duration,
}

impl Timing {
    pub const DEFAULT: Timing = Timing {
        poll: Duration::from_secs(1),
        graceful: Duration::from_secs(30),
        after_sigint: Duration::from_secs(10),
        after_sigkill: Duration::from_secs(5),
    };

    /// The ladder in order: wait, SIGINT and wait, SIGKILL and wait.
    fn rungs(&self) -> [Rung; 3] {
        [
            Rung {
                signal: None,
                wait: self.graceful,
            },
            Rung {
                signal: Some(Signal::Int),
                wait: self.after_sigint,
            },
            Rung {
                signal: Some(Signal::Kill),
                wait: self.after_sigkill,
            },
        ]
    }
}

/// One rung of the client-lock ladder: signal the holder, if the rung has a
/// signal, then wait up to `wait` for the lock to clear.
struct Rung {
    signal: Option<Signal>,
    wait: Duration,
}

#[derive(Debug, Clone, Copy)]
enum Signal {
    Int,
    Kill,
}

impl Signal {
    fn send(self, pid: u32) {
        match self {
            Signal::Int => {
                super::process::sigint(pid);
            }
            Signal::Kill => {
                super::process::sigkill(pid);
            }
        }
    }

    /// The log-line phrase for what the signal is meant to achieve.
    fn intent(self) -> &'static str {
        match self {
            Signal::Int => "SIGINT so it cancels its command",
            Signal::Kill => "SIGKILL",
        }
    }
}

/// Whole seconds, or milliseconds under a second, for log lines.
fn fmt_duration(d: Duration) -> String {
    if d >= Duration::from_secs(1) {
        format!("{}s", d.as_secs_f64().round() as u64)
    } else {
        format!("{}ms", d.as_millis())
    }
}

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.HealthCheckResult>")]
pub struct HealthCheckResult {
    /// One of "healthy", "unhealthy", or "inconclusive".
    outcome: String,
    message: Option<String>,
    exit_code: Option<i32>,
}

starlark_simple_value!(HealthCheckResult);

#[starlark_value(type = "bazel.HealthCheckResult")]
impl<'v> values::StarlarkValue<'v> for HealthCheckResult {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(health_check_result_methods)
    }
}

#[starlark_module]
pub(crate) fn health_check_result_methods(registry: &mut MethodsBuilder) {
    /// The server health state: `"healthy"`, `"unhealthy"`, or `"inconclusive"`.
    #[starlark(attribute)]
    fn outcome<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        Ok(this
            .downcast_ref::<HealthCheckResult>()
            .unwrap()
            .outcome
            .clone())
    }

    /// Diagnostic message, if any.
    #[starlark(attribute)]
    fn message<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<HealthCheckResult>()
                .unwrap()
                .message
                .clone(),
        ))
    }

    /// The original Bazel exit code, if available.
    #[starlark(attribute)]
    fn exit_code<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<HealthCheckResult>().unwrap().exit_code,
        ))
    }
}

impl HealthCheckResult {
    fn healthy() -> Self {
        HealthCheckResult {
            outcome: "healthy".to_string(),
            message: None,
            exit_code: Some(0),
        }
    }

    fn unhealthy(message: String, exit_code: Option<i32>) -> Self {
        HealthCheckResult {
            outcome: "unhealthy".to_string(),
            message: Some(message),
            exit_code,
        }
    }

    fn inconclusive(message: String, exit_code: Option<i32>) -> Self {
        HealthCheckResult {
            outcome: "inconclusive".to_string(),
            message: Some(message),
            exit_code,
        }
    }
}

/// Raw outcome of one probe. `exit_code` is `None` when bazel could not be
/// spawned at all, in which case `stderr` carries the spawn error.
struct CheckResult {
    success: bool,
    exit_code: Option<i32>,
    stderr: String,
}

/// One probe, interpreted. See the module docs for the two lock cases.
#[derive(Debug)]
enum Probe {
    Healthy,
    /// Another live bazel client holds `<output_base>/lock`. `holder` is its
    /// pid when bazel's stderr or the lock file named one.
    ClientLockHeld {
        holder: Option<u32>,
        stderr: String,
    },
    /// The server is wedged: busy with an orphaned command, or failing in a
    /// way a restart is likely to clear.
    ServerWedged {
        exit_code: i32,
        stderr: String,
    },
    /// Not a server problem; nothing here to repair.
    Fatal {
        exit_code: Option<i32>,
        stderr: String,
    },
}

/// Runs `bazel [startup_flags] --noblock_for_lock info server_pid` and returns the result.
fn check_bazel_server(startup_flags: &[String]) -> CheckResult {
    let mut cmd = super::bazel_command();
    cmd.args(startup_flags)
        .arg("--noblock_for_lock")
        .arg("info")
        .arg("server_pid")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let output = match super::live::spawn_registered(&mut cmd) {
        Ok((child, _guard)) => child.wait_with_output(),
        Err(e) => Err(e),
    };

    match output {
        Ok(output) => CheckResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        },
        Err(e) => CheckResult {
            success: false,
            exit_code: None,
            stderr: e.to_string(),
        },
    }
}

/// First `pid=<digits>` in `text`, whether on its own line (the lock file and
/// the client-lock stderr) or inside parentheses (older bazel messages).
fn find_pid(text: &str) -> Option<u32> {
    text.match_indices("pid=").find_map(|(i, _)| {
        let digits: String = text[i + 4..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().ok()
    })
}

/// The pid recorded in `<output_base>/lock` by the last client to take it.
/// The file is truncated and rewritten on acquire but never cleared on
/// release, so a pid read here is only meaningful while bazel says the lock
/// is held.
fn lock_file_holder(output_base: Option<&Path>) -> Option<u32> {
    let content = std::fs::read(output_base?.join("lock")).ok()?;
    find_pid(&String::from_utf8_lossy(&content))
}

fn classify(result: CheckResult, output_base: Option<&Path>) -> Probe {
    if result.success {
        return Probe::Healthy;
    }
    let CheckResult {
        exit_code, stderr, ..
    } = result;
    match exit_code {
        Some(LOCK_HELD_NOBLOCK_FOR_LOCK) if stderr.contains("--noblock_for_lock was given") => {
            let holder = find_pid(&stderr).or_else(|| lock_file_holder(output_base));
            Probe::ClientLockHeld { holder, stderr }
        }
        Some(LOCK_HELD_NOBLOCK_FOR_LOCK) => Probe::ServerWedged {
            exit_code: LOCK_HELD_NOBLOCK_FOR_LOCK,
            stderr,
        },
        Some(code) if WEDGED_SERVER_EXIT_CODES.contains(&code) => Probe::ServerWedged {
            exit_code: code,
            stderr,
        },
        other => Probe::Fatal {
            exit_code: other,
            stderr,
        },
    }
}

/// Collapse bazel's stderr to one line for a diagnostic message.
fn one_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

fn describe_holder(pid: Option<u32>) -> String {
    match pid {
        Some(pid) => match super::process::describe_process(pid) {
            Some(desc) => format!("pid {pid} ({desc})"),
            None => format!("pid {pid}"),
        },
        None => "an unknown process".to_string(),
    }
}

/// Reads the PID from a server PID file on disk.
///
/// Returns `None` if the path is not absolute, does not exist, cannot be read,
/// or does not contain a valid integer. The file is read as latin1 and trimmed.
fn extract_server_pid(server_pid_file: Option<&Path>) -> Option<u32> {
    let path = server_pid_file?;

    if !path.is_absolute() {
        return None;
    }

    let content = std::fs::read(path).ok()?;
    // latin1: each byte maps directly to a unicode codepoint
    let text: String = content.iter().map(|&b| b as char).collect();
    text.trim().parse::<u32>().ok()
}

/// Tries to determine the Bazel output base by running `bazel [startup_flags] info output_base`.
fn get_output_base(startup_flags: &[String]) -> Option<PathBuf> {
    let mut cmd = super::bazel_command();
    cmd.args(startup_flags)
        .arg("info")
        .arg("output_base")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    let (child, _guard) = super::live::spawn_registered(&mut cmd).ok()?;
    let output = child.wait_with_output().ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// Directory `<output_base>/sandbox/_moved_trash_dir` is the rename
/// target that `SandboxBase.tidyUp` moves the live sandbox base into
/// before async-deletion. If a prior invocation was SIGKILL'd between
/// the rename and the deletion, the directory is left behind, and the
/// next command on the same output_base aborts in `SandboxBase.tidyUp`
/// with `"... is supposed to be moved, but file exists"`. See
/// bazelbuild/bazel#23880.
///
/// Deliberately excludes `sandbox_stash` — that is the persistent
/// cross-invocation cache used by `--reuse_sandbox_directories` (on by
/// default in this repo's `bazel/defaults.bazelrc`), and its presence is
/// expected on a healthy runner. Wiping it on every health check would
/// silently undo sandbox reuse on every job.
const STRANDED_MOVED_TRASH_DIR: &str = "_moved_trash_dir";

/// Removes stranded sandbox state left by a previous invocation that
/// was SIGKILL'd before sandbox cleanup could finish. Without this, the
/// next bazel command on the same output_base aborts with the bug
/// described in bazelbuild/bazel#23880.
///
/// `aspect-cli/src/main.rs` already gives bazel a 5s SIGINT grace
/// window before escalating to SIGKILL, but on a heavily-loaded runner
/// cleanup can still time out — this is the safety net that lets the
/// next job on the runner proceed instead of hard-failing in
/// `afterCommand`.
///
/// Logs the removal so it is visible in CI output. Returns `true` iff
/// the entry was present and removed.
fn cleanup_stranded_sandbox_state(output_base: &Path) -> bool {
    let path = output_base.join("sandbox").join(STRANDED_MOVED_TRASH_DIR);
    // symlink_metadata so symlinks are inspected, not followed.
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return false;
    };
    tracing::warn!(
        path = %path.display(),
        "Removing stranded sandbox state from a previous SIGKILL'd \
         invocation (bazelbuild/bazel#23880)"
    );
    let res = if meta.file_type().is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };
    match res {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to remove stranded sandbox state — next invocation may still hit bazelbuild/bazel#23880",
            );
            false
        }
    }
}

/// Extract `--output_base=<path>` from the passed startup flags without
/// invoking bazel. Returns `None` when no `--output_base` flag is present
/// or its value is empty.
///
/// The recovery paths need the output base while the server may be wedged
/// holding the lock, and `bazel info output_base` (without
/// `--noblock_for_lock`) would queue behind it.
fn output_base_from_flags(startup_flags: &[String]) -> Option<PathBuf> {
    startup_flags
        .iter()
        .filter_map(|flag| flag.strip_prefix("--output_base="))
        .find(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Probe the Bazel server and recover from a held lock or a wedged server
/// when possible. `log` receives one human-readable line per step taken, for
/// the job log.
///
/// The `--noblock_for_lock` probe is intentionally the FIRST bazel call.
/// Any other invocation (in particular `bazel info output_base`) lacks the
/// flag and would queue behind a wedged server holding the lock, defeating
/// the purpose of the health check. The output base comes from the startup
/// flags for the same reason.
pub fn run(startup_flags: &[String], log: &mut dyn FnMut(&str)) -> HealthCheckResult {
    let output_base = output_base_from_flags(startup_flags);
    let mut probe = || check_bazel_server(startup_flags);
    let result = run_with(&mut probe, output_base.as_deref(), &Timing::DEFAULT, log);
    if result.outcome == "healthy"
        && let Some(base) = output_base.or_else(|| get_output_base(startup_flags))
    {
        let _ = cleanup_stranded_sandbox_state(&base);
    }
    result
}

/// The ladder, with the probe and clock injected so tests can drive it with
/// a fake bazel and millisecond windows.
///
/// Outcomes:
///   - `healthy`: a probe returned 0, on the first try or after recovery.
///   - `inconclusive`: an exit code no restart would fix, or bazel would not
///     spawn. Likely a configuration issue.
///   - `unhealthy`: the lock stayed held through SIGKILL of its holder, the
///     server pid could not be found, or the re-probe after killing the
///     server still failed.
fn run_with(
    probe: &mut dyn FnMut() -> CheckResult,
    output_base: Option<&Path>,
    timing: &Timing,
    log: &mut dyn FnMut(&str),
) -> HealthCheckResult {
    let mut last = classify(probe(), output_base);
    if matches!(last, Probe::ClientLockHeld { .. }) {
        last = clear_client_lock(probe, output_base, timing, last, log);
    }
    match last {
        Probe::Healthy => HealthCheckResult::healthy(),
        Probe::ClientLockHeld { holder, stderr } => {
            let message = match holder {
                Some(pid) => format!(
                    "another bazel client ({}) still holds the output base lock after waiting, SIGINT and SIGKILL: {}",
                    describe_holder(Some(pid)),
                    one_line(&stderr)
                ),
                None => format!(
                    "another bazel client holds the output base lock and its pid could not be determined from bazel's output or {}: {}",
                    output_base.map_or("the lock file".to_string(), |b| b
                        .join("lock")
                        .display()
                        .to_string()),
                    one_line(&stderr)
                ),
            };
            HealthCheckResult::unhealthy(message, Some(LOCK_HELD_NOBLOCK_FOR_LOCK))
        }
        Probe::ServerWedged { exit_code, stderr } => {
            kill_server_and_retry(probe, output_base, exit_code, &stderr, log)
        }
        Probe::Fatal { exit_code, stderr } => HealthCheckResult::inconclusive(
            format!(
                "Unable to health check bazel server due to potential configuration issues: {}",
                one_line(&stderr)
            ),
            exit_code,
        ),
    }
}

/// How many times a new holder taking the lock restarts the ladder before
/// the current ladder runs on regardless. Bounds the total wait under
/// pathological lock churn.
const MAX_LADDER_RESTARTS: u32 = 3;

/// What a polling window ended with.
enum Poll {
    /// The lock is no longer held by a client; here is that probe.
    Released(Probe),
    /// The same holder still has the lock after the whole budget.
    StillHeld(Probe),
    /// A different holder (by pid) has the lock now.
    HolderChanged(Probe, Option<u32>),
}

/// Re-probe every `poll` until the lock is released, its holder changes, or
/// `budget` elapses. Returns what happened and the time spent.
fn poll_while_client_holds(
    probe: &mut dyn FnMut() -> CheckResult,
    output_base: Option<&Path>,
    poll: Duration,
    budget: Duration,
    holder: Option<u32>,
) -> (Poll, Duration) {
    let start = Instant::now();
    loop {
        std::thread::sleep(poll);
        let p = classify(probe(), output_base);
        let Probe::ClientLockHeld {
            holder: now_holding,
            ..
        } = &p
        else {
            return (Poll::Released(p), start.elapsed());
        };
        if *now_holding != holder {
            let now_holding = *now_holding;
            return (Poll::HolderChanged(p, now_holding), start.elapsed());
        }
        if start.elapsed() >= budget {
            return (Poll::StillHeld(p), start.elapsed());
        }
    }
}

/// Climb the client-lock rungs of `timing` starting from `first`, a
/// `ClientLockHeld` probe. A new holder gets a fresh ladder, up to
/// `MAX_LADDER_RESTARTS` times; after that the ladder runs on and signals
/// whoever holds the lock at each rung. Returns the first probe that is not
/// `ClientLockHeld`, or the last one if the lock never cleared.
fn clear_client_lock(
    probe: &mut dyn FnMut() -> CheckResult,
    output_base: Option<&Path>,
    timing: &Timing,
    first: Probe,
    log: &mut dyn FnMut(&str),
) -> Probe {
    let mut holder = match &first {
        Probe::ClientLockHeld { holder, .. } => *holder,
        _ => return first,
    };
    let mut last = first;
    let mut restarts = 0;
    'ladder: loop {
        log(&format!(
            "output base lock is held by {}; waiting up to {} for it to finish",
            describe_holder(holder),
            fmt_duration(timing.graceful)
        ));
        let mut waited = Duration::ZERO;
        for rung in timing.rungs() {
            if let Some(signal) = rung.signal {
                let Some(pid) = holder else {
                    log(
                        "output base lock is still held and the holder's pid is unknown; nothing to signal",
                    );
                    break;
                };
                log(&format!(
                    "output base lock is still held by pid {pid} after {}; sending {}",
                    fmt_duration(waited),
                    signal.intent()
                ));
                signal.send(pid);
            }
            let (poll, elapsed) =
                poll_while_client_holds(probe, output_base, timing.poll, rung.wait, holder);
            waited += elapsed;
            match poll {
                Poll::Released(p) => {
                    log(&format!(
                        "output base lock released after {}",
                        fmt_duration(waited)
                    ));
                    return p;
                }
                Poll::StillHeld(p) => last = p,
                Poll::HolderChanged(p, new_holder) => {
                    log(&format!(
                        "output base lock holder changed from {} to {}",
                        describe_holder(holder),
                        describe_holder(new_holder)
                    ));
                    holder = new_holder;
                    last = p;
                    if restarts < MAX_LADDER_RESTARTS {
                        restarts += 1;
                        continue 'ladder;
                    }
                    log(
                        "holder has changed too many times; continuing without restarting the wait",
                    );
                }
            }
        }
        log(&format!(
            "output base lock is still held after {}; giving up",
            fmt_duration(waited)
        ));
        return last;
    }
}

/// The server rung: SIGKILL the pid in `<output_base>/server/server.pid.txt`
/// and re-probe once. The re-probe starts a fresh server, so a single retry
/// is enough when the old one was the problem.
fn kill_server_and_retry(
    probe: &mut dyn FnMut() -> CheckResult,
    output_base: Option<&Path>,
    exit_code: i32,
    stderr: &str,
    log: &mut dyn FnMut(&str),
) -> HealthCheckResult {
    let diagnostic = format!(
        "Bazel server returned exit code {exit_code}: {}",
        one_line(stderr)
    );
    log(&format!(
        "bazel server is not responding: {}",
        one_line(stderr)
    ));

    let Some(output_base) = output_base else {
        log("cannot restart the server: no --output_base in the startup flags");
        return HealthCheckResult::unhealthy(diagnostic, Some(exit_code));
    };
    let server_pid_file = output_base.join("server").join("server.pid.txt");
    let Some(pid) = extract_server_pid(Some(&server_pid_file)) else {
        log(&format!(
            "cannot restart the server: no server pid in {}",
            server_pid_file.display()
        ));
        return HealthCheckResult::unhealthy(diagnostic, Some(exit_code));
    };

    if super::process::is_pid_running(pid) {
        log(&format!(
            "sending SIGKILL to bazel server pid {pid} so the next command starts a fresh server"
        ));
        super::process::sigkill(pid);
    } else {
        log(&format!(
            "bazel server pid {pid} from {} is not running",
            server_pid_file.display()
        ));
    }

    log("re-probing the bazel server");
    match classify(probe(), Some(output_base)) {
        Probe::Healthy => {
            log("bazel server responded after restart");
            HealthCheckResult::healthy()
        }
        Probe::ClientLockHeld { stderr, .. }
        | Probe::ServerWedged { stderr, .. }
        | Probe::Fatal { stderr, .. } => HealthCheckResult::unhealthy(
            format!(
                "{diagnostic}; after killing server pid {pid} the probe still failed: {}",
                one_line(&stderr)
            ),
            Some(exit_code),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_output_base() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("sandbox")).expect("create sandbox dir");
        dir
    }

    #[test]
    fn cleanup_noop_when_sandbox_clean() {
        let base = make_output_base();
        assert!(!cleanup_stranded_sandbox_state(base.path()));
    }

    #[test]
    fn cleanup_removes_moved_trash_dir() {
        let base = make_output_base();
        let moved_trash = base.path().join("sandbox").join("_moved_trash_dir");
        std::fs::create_dir(&moved_trash).unwrap();
        // Non-empty dir — exercise remove_dir_all.
        std::fs::write(moved_trash.join("leftover"), b"junk").unwrap();

        assert!(cleanup_stranded_sandbox_state(base.path()));
        assert!(!moved_trash.exists());
    }

    #[test]
    fn cleanup_preserves_sandbox_stash() {
        // `sandbox_stash` is the persistent --reuse_sandbox_directories
        // cache; the health check must NOT touch it.
        let base = make_output_base();
        let stash = base.path().join("sandbox").join("sandbox_stash");
        std::fs::create_dir(&stash).unwrap();
        std::fs::write(stash.join("cached_action"), b"reuse-me").unwrap();

        assert!(!cleanup_stranded_sandbox_state(base.path()));
        assert!(
            stash.exists(),
            "sandbox_stash must survive the health check"
        );
    }

    #[test]
    fn cleanup_ignores_unrelated_entries() {
        let base = make_output_base();
        let other = base.path().join("sandbox").join("linux-sandbox");
        std::fs::create_dir(&other).unwrap();

        assert!(!cleanup_stranded_sandbox_state(base.path()));
        assert!(
            other.exists(),
            "must not touch the per-strategy sandbox dirs"
        );
    }

    #[test]
    fn cleanup_handles_missing_sandbox_dir() {
        // No sandbox subdirectory at all — e.g. fresh output_base.
        let base = tempfile::tempdir().expect("tempdir");
        assert!(!cleanup_stranded_sandbox_state(base.path()));
    }

    #[test]
    fn output_base_from_flags_finds_explicit_flag() {
        let flags = vec![
            "--nohome_rc".to_string(),
            "--output_base=/mnt/ephemeral/output/repo".to_string(),
            "--nosystem_rc".to_string(),
        ];
        assert_eq!(
            output_base_from_flags(&flags),
            Some(PathBuf::from("/mnt/ephemeral/output/repo"))
        );
    }

    #[test]
    fn output_base_from_flags_absent_returns_none() {
        let flags = vec![
            "--nohome_rc".to_string(),
            "--output_user_root=/mnt/ephemeral/bazel".to_string(),
        ];
        assert_eq!(output_base_from_flags(&flags), None);
    }

    #[test]
    fn output_base_from_flags_empty_value_returns_none() {
        // Defensive: malformed `--output_base=` should not silently
        // produce PathBuf::from("") which would join into nonsense.
        let flags = vec!["--output_base=".to_string()];
        assert_eq!(output_base_from_flags(&flags), None);
    }

    #[test]
    fn output_base_from_flags_prefix_match_only() {
        // `--output_user_root` shares a `--output_` prefix; must not
        // accidentally match.
        let flags = vec!["--output_user_root=/mnt/foo".to_string()];
        assert_eq!(output_base_from_flags(&flags), None);
    }

    // --- probe classification ---

    /// Bazel's stderr when another live client holds `<output_base>/lock`.
    fn client_lock_stderr(pid: u32) -> String {
        format!(
            "Another command holds the output base lock: \npid={pid}\nowner=client\ncwd=/work\n\
             Exiting because the output base lock is held and --noblock_for_lock was given.\n"
        )
    }

    /// Bazel's stderr when the server's command lock is held.
    const SERVER_BUSY_STDERR: &str =
        "Another command (pid=4242) is running. Exiting immediately.\n";

    fn failed(code: i32, stderr: &str) -> CheckResult {
        CheckResult {
            success: false,
            exit_code: Some(code),
            stderr: stderr.to_string(),
        }
    }

    fn ok() -> CheckResult {
        CheckResult {
            success: true,
            exit_code: Some(0),
            stderr: String::new(),
        }
    }

    #[test]
    fn fmt_duration_rounds_seconds_and_keeps_millis_under_a_second() {
        assert_eq!(fmt_duration(Duration::from_millis(150)), "150ms");
        assert_eq!(fmt_duration(Duration::from_millis(999)), "999ms");
        assert_eq!(fmt_duration(Duration::from_secs(30)), "30s");
        assert_eq!(fmt_duration(Duration::from_millis(30_400)), "30s");
        assert_eq!(fmt_duration(Duration::from_millis(30_600)), "31s");
    }

    #[test]
    fn one_line_collapses_stderr() {
        assert_eq!(
            one_line("Another command holds the lock: \npid=1\n\n  owner=client \n"),
            "Another command holds the lock:; pid=1; owner=client"
        );
    }

    #[test]
    fn find_pid_reads_own_line_and_parenthesised_forms() {
        assert_eq!(find_pid("pid=123\nowner=client\n"), Some(123));
        assert_eq!(find_pid("Another command (pid=77) is running."), Some(77));
        assert_eq!(find_pid("pid=\nowner=client\npid=9\n"), Some(9));
        assert_eq!(find_pid("no pid here"), None);
    }

    #[test]
    fn classify_client_lock_takes_pid_from_stderr() {
        let p = classify(failed(9, &client_lock_stderr(555)), None);
        assert!(
            matches!(
                p,
                Probe::ClientLockHeld {
                    holder: Some(555),
                    ..
                }
            ),
            "{p:?}"
        );
    }

    #[test]
    fn classify_client_lock_falls_back_to_lock_file() {
        let base = tempfile::tempdir().unwrap();
        std::fs::write(base.path().join("lock"), "pid=808\nowner=client\ncwd=/x\n").unwrap();
        let stderr =
            "Exiting because the output base lock is held and --noblock_for_lock was given.\n";
        let p = classify(failed(9, stderr), Some(base.path()));
        assert!(
            matches!(
                p,
                Probe::ClientLockHeld {
                    holder: Some(808),
                    ..
                }
            ),
            "{p:?}"
        );
    }

    #[test]
    fn classify_server_busy_is_wedged_server() {
        let p = classify(failed(9, SERVER_BUSY_STDERR), None);
        assert!(
            matches!(p, Probe::ServerWedged { exit_code: 9, .. }),
            "{p:?}"
        );
        // Exit 9 with output we do not recognise still goes to the server rung.
        let p = classify(failed(9, "something new"), None);
        assert!(
            matches!(p, Probe::ServerWedged { exit_code: 9, .. }),
            "{p:?}"
        );
    }

    #[test]
    fn classify_other_codes() {
        assert!(matches!(classify(ok(), None), Probe::Healthy));
        assert!(matches!(
            classify(failed(37, "internal"), None),
            Probe::ServerWedged { exit_code: 37, .. }
        ));
        assert!(matches!(
            classify(failed(2, "bad flag"), None),
            Probe::Fatal {
                exit_code: Some(2),
                ..
            }
        ));
        let spawn_failed = CheckResult {
            success: false,
            exit_code: None,
            stderr: "No such file".into(),
        };
        assert!(matches!(
            classify(spawn_failed, None),
            Probe::Fatal {
                exit_code: None,
                ..
            }
        ));
    }

    // --- the ladder, driven against a real child process standing in for
    // the lock holder ---

    #[cfg(unix)]
    mod ladder {
        use super::*;
        use std::process::{Child, Command};

        /// Millisecond windows so a test runs in well under a second per rung.
        const FAST: Timing = Timing {
            poll: Duration::from_millis(20),
            graceful: Duration::from_millis(150),
            after_sigint: Duration::from_millis(300),
            after_sigkill: Duration::from_millis(1500),
        };

        /// A child standing in for a lock holder or a server. Scripts that
        /// only sleep `exec` so the sleep is the signalled process: a shell
        /// waiting on a foreground child defers a SIGINT sent to it alone.
        fn spawn(script: &str) -> Child {
            Command::new("sh")
                .arg("-c")
                .arg(script)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn sh")
        }

        /// `try_wait` reaps the child so a killed holder stops looking alive;
        /// `kill(pid, 0)` succeeds on a zombie.
        fn alive(child: &mut Child) -> bool {
            child.try_wait().expect("try_wait").is_none()
        }

        /// Drive the ladder against a fake bazel that reports the client lock
        /// held while `holder` lives, with its pid in stderr when
        /// `report_pid`, then healthy. The borrow of `holder` ends on return
        /// so the test can inspect it.
        fn run_lock_ladder(
            holder: &mut Child,
            report_pid: bool,
        ) -> (HealthCheckResult, Vec<String>) {
            let pid = holder.id();
            let mut probe = move || {
                if alive(holder) {
                    let stderr = if report_pid {
                        client_lock_stderr(pid)
                    } else {
                        "Exiting because the output base lock is held and --noblock_for_lock was given.\n".to_string()
                    };
                    failed(9, &stderr)
                } else {
                    ok()
                }
            };
            run_ladder(&mut probe, None)
        }

        fn run_ladder(
            probe: &mut dyn FnMut() -> CheckResult,
            output_base: Option<&Path>,
        ) -> (HealthCheckResult, Vec<String>) {
            let mut lines = Vec::new();
            let result = run_with(probe, output_base, &FAST, &mut |l| {
                lines.push(l.to_string())
            });
            (result, lines)
        }

        fn joined(lines: &[String]) -> String {
            lines.join("\n")
        }

        #[test]
        fn holder_that_exits_on_its_own_is_left_alone() {
            let mut holder = spawn("exec sleep 0.05");
            let (result, lines) = run_lock_ladder(&mut holder, true);
            assert_eq!(result.outcome, "healthy", "{}", joined(&lines));
            let log = joined(&lines);
            assert!(log.contains("waiting up to 150ms"), "{log}");
            assert!(log.contains("lock released"), "{log}");
            assert!(!log.contains("SIGINT"), "{log}");
        }

        #[test]
        fn holder_that_overstays_gets_sigint() {
            let mut holder = spawn("exec sleep 30");
            let (result, lines) = run_lock_ladder(&mut holder, true);
            assert_eq!(result.outcome, "healthy", "{}", joined(&lines));
            let log = joined(&lines);
            assert!(log.contains("sending SIGINT"), "{log}");
            assert!(!log.contains("SIGKILL"), "{log}");
            assert!(!alive(&mut holder));
        }

        #[test]
        fn holder_that_ignores_sigint_gets_sigkill() {
            let mut holder = spawn("trap '' INT; sleep 30");
            let (result, lines) = run_lock_ladder(&mut holder, true);
            assert_eq!(result.outcome, "healthy", "{}", joined(&lines));
            let log = joined(&lines);
            assert!(log.contains("sending SIGINT"), "{log}");
            assert!(log.contains("sending SIGKILL"), "{log}");
            assert!(!alive(&mut holder));
        }

        #[test]
        fn a_new_holder_gets_a_fresh_ladder() {
            // A exits during the graceful wait and B takes the lock. B must
            // get its own graceful window before any signal, and then the
            // SIGINT that A never needed.
            let mut a = spawn("exec sleep 0.05");
            let mut b = spawn("exec sleep 30");
            let (a_pid, b_pid) = (a.id(), b.id());
            let mut probe = || {
                if alive(&mut a) {
                    failed(9, &client_lock_stderr(a_pid))
                } else if alive(&mut b) {
                    failed(9, &client_lock_stderr(b_pid))
                } else {
                    ok()
                }
            };
            let (result, lines) = run_ladder(&mut probe, None);
            assert_eq!(result.outcome, "healthy", "{}", joined(&lines));
            let log = joined(&lines);
            assert!(log.contains("holder changed"), "{log}");
            assert_eq!(log.matches("waiting up to 150ms").count(), 2, "{log}");
            assert!(log.contains(&format!("held by pid {b_pid} after")), "{log}");
            assert!(
                !log.contains(&format!("held by pid {a_pid} after")),
                "{log}"
            );
            assert!(!alive(&mut b));
        }

        #[test]
        fn unknown_holder_is_unhealthy_without_signalling() {
            let mut holder = spawn("exec sleep 30");
            let (result, lines) = run_lock_ladder(&mut holder, false);
            assert_eq!(result.outcome, "unhealthy", "{}", joined(&lines));
            let message = result.message.unwrap();
            assert!(message.contains("pid could not be determined"), "{message}");
            let log = joined(&lines);
            assert!(!log.contains("SIGINT"), "{log}");
            assert!(alive(&mut holder), "must not guess at a pid to kill");
            let _ = holder.kill();
        }

        #[test]
        fn orphaned_server_is_killed_once_the_client_is_gone() {
            // The leftover client dies during the graceful wait; the server
            // is still running its command, so the next probe hits the
            // server's command lock. The ladder kills the server pid from
            // the pid file and the probe after that succeeds.
            let base = tempfile::tempdir().unwrap();
            std::fs::create_dir(base.path().join("server")).unwrap();
            let mut server = spawn("exec sleep 30");
            std::fs::write(
                base.path().join("server/server.pid.txt"),
                server.id().to_string(),
            )
            .unwrap();
            let mut client = spawn("exec sleep 0.05");
            let client_pid = client.id();
            let mut server_busy_reported = false;
            let mut probe = || {
                if alive(&mut client) {
                    failed(9, &client_lock_stderr(client_pid))
                } else if !server_busy_reported {
                    server_busy_reported = true;
                    failed(9, SERVER_BUSY_STDERR)
                } else {
                    ok()
                }
            };
            let (result, lines) = run_ladder(&mut probe, Some(base.path()));
            assert_eq!(result.outcome, "healthy", "{}", joined(&lines));
            let log = joined(&lines);
            assert!(log.contains("lock released"), "{log}");
            assert!(
                log.contains(&format!("SIGKILL to bazel server pid {}", server.id())),
                "{log}"
            );
            assert!(!log.contains("sending SIGINT"), "{log}");
            let status = server.wait().unwrap();
            assert!(!status.success());
        }

        #[test]
        fn server_that_stays_wedged_after_kill_is_unhealthy() {
            let base = tempfile::tempdir().unwrap();
            std::fs::create_dir(base.path().join("server")).unwrap();
            let mut server = spawn("exec sleep 30");
            std::fs::write(
                base.path().join("server/server.pid.txt"),
                server.id().to_string(),
            )
            .unwrap();
            let mut probe = || failed(37, "Blaze internal error");
            let (result, lines) = run_ladder(&mut probe, Some(base.path()));
            assert_eq!(result.outcome, "unhealthy", "{}", joined(&lines));
            assert_eq!(result.exit_code, Some(37));
            let message = result.message.unwrap();
            assert!(message.contains("exit code 37"), "{message}");
            assert!(message.contains("still failed"), "{message}");
            assert!(!server.wait().unwrap().success());
        }

        #[test]
        fn wedged_server_without_output_base_is_unhealthy() {
            let mut probe = || failed(9, SERVER_BUSY_STDERR);
            let (result, lines) = run_ladder(&mut probe, None);
            assert_eq!(result.outcome, "unhealthy");
            assert!(
                joined(&lines).contains("no --output_base"),
                "{}",
                joined(&lines)
            );
        }

        #[test]
        fn configuration_errors_are_inconclusive() {
            let mut probe = || failed(2, "Unrecognized option: --bogus");
            let (result, lines) = run_ladder(&mut probe, None);
            assert_eq!(result.outcome, "inconclusive", "{}", joined(&lines));
            assert!(result.message.unwrap().contains("--bogus"));
            assert!(lines.is_empty(), "{}", joined(&lines));
        }
    }
}
