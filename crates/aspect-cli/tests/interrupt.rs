//! Ctrl+C and SIGTERM reach the AXL task as cancellation, not as a force
//! exit: the task ends with the conventional code, its post-task hooks and
//! `ctx.defer` callbacks run, and every child it spawned is stopped. A second
//! signal is the backstop that ends the process whatever the task does.
//!
//! Each test starts the real CLI on a fixture task that announces itself on
//! stderr once its child is running, sends the signal to the CLI process
//! only (never to the child, which is in its own process group where needed),
//! and reads what the task printed on the way out.

#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use common::aspect_cli;

/// A fixture task running in a scratch workspace.
struct Run {
    _dir: tempfile::TempDir,
    child: Child,
    stderr: BufReader<std::process::ChildStderr>,
    /// What the task printed before the signal, including the `ready` line.
    seen: String,
}

impl Run {
    /// Start `task` from `source` and wait for its `ready` line. The line may
    /// carry `pid=<n>`, the child the task spawned.
    fn start(source: &str, task: &str) -> Self {
        Self::launch(source, task, Stdio::null())
    }

    /// [`Run::start`] with stdin an open pipe nothing ever writes to.
    fn start_with_stdin(source: &str, task: &str) -> Self {
        Self::launch(source, task, Stdio::piped())
    }

    fn launch(source: &str, task: &str, stdin: Stdio) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("MODULE.bazel"), "").expect("MODULE.bazel");
        let aspect = dir.path().join(".aspect");
        std::fs::create_dir(&aspect).expect(".aspect");
        std::fs::write(aspect.join("fixture.axl"), source).expect("fixture");
        let mut child = Command::new(aspect_cli())
            .arg(task)
            .current_dir(dir.path())
            .env(
                "ASPECT_CREDENTIALS_FILE",
                dir.path().join("credentials.json"),
            )
            .env("DO_NOT_TRACK", "1")
            .env_remove("ASPECT_DEBUG")
            .env_remove("CI")
            .env_remove("BUILDKITE")
            .env_remove("GITHUB_ACTIONS")
            .stdin(stdin)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn aspect-cli");
        let mut stderr = BufReader::new(child.stderr.take().expect("piped stderr"));
        let mut seen = String::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let mut line = String::new();
            let n = stderr.read_line(&mut line).expect("reading stderr");
            seen.push_str(&line);
            if n == 0 || line.contains("ready") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the fixture never became ready:\n{seen}"
            );
        }
        assert!(seen.contains("ready"), "the fixture exited early:\n{seen}");
        Self {
            _dir: dir,
            child,
            stderr,
            seen,
        }
    }

    /// The pid the `ready` line announced.
    fn announced_pid(&self) -> i32 {
        let line = self
            .seen
            .lines()
            .find(|l| l.contains("ready"))
            .expect("a ready line");
        let pid = line.split("pid=").nth(1).expect("ready line carries pid=");
        pid.trim().parse().expect("a numeric pid")
    }

    fn signal(&self, signal: libc::c_int) {
        let rc = unsafe { libc::kill(self.child.id() as libc::pid_t, signal) };
        assert_eq!(rc, 0, "signalling aspect-cli");
    }

    /// Read the rest of stderr and reap the CLI.
    fn finish(mut self) -> (ExitStatus, String) {
        let mut rest = String::new();
        self.stderr
            .read_to_string(&mut rest)
            .expect("draining stderr");
        let status = self.child.wait().expect("waiting for aspect-cli");
        (status, format!("{}{rest}", self.seen))
    }
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Wait up to `timeout` for `pid` to go away.
fn wait_gone(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while alive(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

/// A task waiting on a `sleep 30`, with a post-task hook and a defer that
/// each leave a line on stderr. The signal stops the child, the wait returns,
/// and the task ends on its own terms: 128 + the signal its child died of.
const WAITING: &str = r#"
def _post(ctx, outcome):
    print("post-task hook saw exit code %d" % outcome.exit_code)

def _impl(ctx):
    ctx.hooks.post_task(_post)
    ctx.defer(print, "deferred cleanup ran")
    child = ctx.std.process.command("sleep").arg("30").spawn()
    print("ready pid=%d" % child.id)
    status = child.wait()
    print("child ended with signal %s" % status.signal)
    return 128 + status.signal if status.signal != None else 0

waiting = task(summary = "Test fixture.", implementation = _impl)
"#;

fn assert_default_ending(stderr: &str, code: i32) {
    assert!(
        stderr.contains(&format!("post-task hook saw exit code {code}")),
        "the post-task hook did not run with the exit code:\n{stderr}"
    );
    assert!(
        stderr.contains("deferred cleanup ran"),
        "the defer did not run:\n{stderr}"
    );
    assert!(
        !stderr.contains("Traceback"),
        "a cancellation must not render a traceback:\n{stderr}"
    );
}

#[test]
fn ctrl_c_stops_the_child_and_the_task_ends_on_its_own() {
    let run = Run::start(WAITING, "waiting");
    let child_pid = run.announced_pid();
    run.signal(libc::SIGINT);
    let (status, stderr) = run.finish();
    assert_eq!(
        status.code(),
        Some(130),
        "exit code\n--- stderr ---\n{stderr}"
    );
    assert!(stderr.contains("child ended with signal 2"), "{stderr}");
    assert!(
        !stderr.contains("ERROR: interrupted"),
        "the body returned on its own:\n{stderr}"
    );
    assert_default_ending(&stderr, 130);
    assert!(
        wait_gone(child_pid, Duration::from_secs(5)),
        "the task's child outlived aspect-cli"
    );
}

/// SIGTERM to aspect-cli alone (a CI cancel): the child is still stopped
/// through its own sequence, SIGINT first.
#[test]
fn sigterm_stops_the_child_the_same_way() {
    let run = Run::start(WAITING, "waiting");
    let child_pid = run.announced_pid();
    run.signal(libc::SIGTERM);
    let (status, stderr) = run.finish();
    assert_eq!(
        status.code(),
        Some(130),
        "exit code\n--- stderr ---\n{stderr}"
    );
    assert!(stderr.contains("child ended with signal 2"), "{stderr}");
    assert_default_ending(&stderr, 130);
    assert!(wait_gone(child_pid, Duration::from_secs(5)));
}

/// A task that ignores the cancel and has nothing bound is ended a second
/// after the signal, with the conventional code, its hooks and defers intact.
#[test]
fn a_task_that_ignores_the_cancel_is_ended_after_the_grace() {
    const IGNORES: &str = r#"
load("@std//time.axl", "sleep_iter")

def _post(ctx, outcome):
    print("post-task hook saw exit code %d" % outcome.exit_code)

def _impl(ctx):
    ctx.hooks.post_task(_post)
    ctx.defer(print, "deferred cleanup ran")
    print("ready")
    for _tick in sleep_iter(50):
        pass
    return 0

ignores = task(summary = "Test fixture.", implementation = _impl)
"#;
    let run = Run::start(IGNORES, "ignores");
    run.signal(libc::SIGTERM);
    let (status, stderr) = run.finish();
    assert_eq!(
        status.code(),
        Some(143),
        "exit code\n--- stderr ---\n{stderr}"
    );
    assert!(stderr.contains("ERROR: terminated"), "{stderr}");
    assert_default_ending(&stderr, 143);
}

/// The `run --watch` shape (PR #1540): the child leads its own process group,
/// so the terminal's Ctrl+C would never reach it; the runtime does, with a
/// terminate signal first so the child's own cleanup runs.
#[test]
fn a_child_in_its_own_process_group_is_terminated_gracefully() {
    let scratch = tempfile::tempdir().expect("scratch");
    let marker_path = scratch
        .path()
        .join("cleaned")
        .to_string_lossy()
        .into_owned();
    let ready_path = scratch.path().join("ready").to_string_lossy().into_owned();
    let source = format!(
        r#"
def _impl(ctx):
    child = ctx.std.process.command("sh").args([
        "-c",
        "trap 'echo cleaned > \"$MARKER\"; exit 0' INT TERM; touch \"$READY\"; sleep 30 & wait",
    ]).env("MARKER", "{marker_path}").env("READY", "{ready_path}").process_group(0).spawn()
    print("ready pid=%d" % child.id)
    child.wait()
    return 0

grouped = task(summary = "Test fixture.", implementation = _impl)
"#
    );
    let run = Run::start(&source, "grouped");
    let child_pid = run.announced_pid();
    // The shell has installed its trap once it touches the ready file.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::path::Path::new(&ready_path).exists() {
        assert!(Instant::now() < deadline, "the shell never became ready");
        std::thread::sleep(Duration::from_millis(10));
    }
    run.signal(libc::SIGINT);
    let (status, stderr) = run.finish();
    // The child's trap exits 0, so the task's own `return 0` stands.
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(wait_gone(child_pid, Duration::from_secs(5)));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !std::path::Path::new(&marker_path).exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        std::fs::read_to_string(&marker_path).is_ok_and(|s| s.contains("cleaned")),
        "the child's TERM handler did not run:\n{stderr}"
    );
}

/// A child that would otherwise sleep for ten minutes is stopped with the task.
#[test]
fn a_long_sleeping_child_is_stopped() {
    const SLEEPER: &str = r#"
def _impl(ctx):
    child = ctx.std.process.command("sleep").arg("600").spawn()
    print("ready pid=%d" % child.id)
    child.wait()
    return 0

sleeper = task(summary = "Test fixture.", implementation = _impl)
"#;
    let run = Run::start(SLEEPER, "sleeper");
    let sleep_pid = run.announced_pid();
    run.signal(libc::SIGINT);
    let (status, stderr) = run.finish();
    // `sleep` died of SIGINT, the wait returned, the task's `return 0` stands.
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(
        wait_gone(sleep_pid, Duration::from_secs(5)),
        "`sleep 600` outlived aspect-cli"
    );
}

/// A shell whose own child sleeps for ten minutes, in a process group of its
/// own: both the shell and the sleeper are gone afterwards, not just the shell.
#[test]
fn a_shell_and_its_long_sleeping_child_are_both_stopped() {
    let scratch = tempfile::tempdir().expect("scratch");
    let pid_path = scratch
        .path()
        .join("sleep.pid")
        .to_string_lossy()
        .into_owned();
    let source = format!(
        r#"
def _impl(ctx):
    child = ctx.std.process.command("bash").args([
        "-c",
        "sleep 600 & echo $! > \"$PIDFILE\"; wait",
    ]).env("PIDFILE", "{pid_path}").process_group(0).spawn()
    print("ready pid=%d" % child.id)
    child.wait()
    return 0

shell = task(summary = "Test fixture.", implementation = _impl)
"#
    );
    let run = Run::start(&source, "shell");
    let shell_pid = run.announced_pid();
    let deadline = Instant::now() + Duration::from_secs(10);
    let sleep_pid: i32 = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_path)
            && let Ok(pid) = text.trim().parse()
        {
            break pid;
        }
        assert!(
            Instant::now() < deadline,
            "the shell never started its sleeper"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        alive(sleep_pid),
        "the sleeper should be running before the signal"
    );
    run.signal(libc::SIGINT);
    let (status, stderr) = run.finish();
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(
        wait_gone(shell_pid, Duration::from_secs(5)),
        "the shell survived"
    );
    assert!(
        wait_gone(sleep_pid, Duration::from_secs(5)),
        "`sleep 600` survived"
    );
}

/// A prompt nobody will answer: a task blocked reading stdin still ends on the
/// first Ctrl+C.
#[test]
fn a_task_blocked_on_stdin_ends_on_the_first_ctrl_c() {
    const PROMPT: &str = r#"
def _impl(ctx):
    print("ready")
    answer = ctx.std.io.stdin.read_to_string()
    print("got %r" % answer)
    return 0

prompt = task(summary = "Test fixture.", implementation = _impl)
"#;
    let run = Run::start_with_stdin(PROMPT, "prompt");
    run.signal(libc::SIGINT);
    let (status, stderr) = run.finish();
    assert_eq!(status.code(), Some(130), "{stderr}");
    assert!(stderr.contains("ERROR: interrupted"), "{stderr}");
}

/// A task that ignores the cancel: the second Ctrl+C is the backstop and ends
/// the process at once, before the grace would have.
#[test]
fn a_second_ctrl_c_ends_a_task_that_never_stops() {
    const STUBBORN: &str = r#"
load("@std//time.axl", "sleep_iter")

def _impl(ctx):
    print("ready")
    for _tick in sleep_iter(50):
        pass
    return 0

stubborn = task(summary = "Test fixture.", implementation = _impl)
"#;
    let run = Run::start(STUBBORN, "stubborn");
    run.signal(libc::SIGINT);
    std::thread::sleep(Duration::from_millis(500));
    run.signal(libc::SIGINT);
    let (status, stderr) = run.finish();
    assert_eq!(status.code(), Some(130), "{stderr}");
    assert!(stderr.contains("exiting with code 130"), "{stderr}");
}

/// The watch shape: a loop that reads the root on its tick. The child is
/// already stopping when it notices; it waits for it and returns 0.
#[test]
fn a_loop_that_watches_the_root_returns_on_its_own_terms() {
    const OWNER: &str = r#"
load("@std//time.axl", "sleep_iter")

def _impl(ctx):
    child = ctx.std.process.command("sleep").arg("30").spawn()
    print("ready pid=%d" % child.id)
    for _tick in sleep_iter(50):
        if ctx.cancellation.root.cancelled:
            status = child.wait()
            print("child ended with signal %s" % status.signal)
            return 0
    return 1

owner = task(summary = "Test fixture.", implementation = _impl)
"#;
    let run = Run::start(OWNER, "owner");
    run.signal(libc::SIGINT);
    let (status, stderr) = run.finish();
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("child ended with signal 2"), "{stderr}");
}

/// The `run --watch` ending: Ctrl+C ends the supervisor's loop, which stops
/// the watched target (its own process group, as `run --watch` starts it)
/// gracefully, and the run exits 0.
#[test]
fn ctrl_c_ends_a_watch_session_and_stops_its_target() {
    const WATCH: &str = r#"
load("@aspect//private/lib/watch.axl", "watch")
load("@aspect//private/lib/watch_supervisor.axl", "watch_supervisor")

def _spawn(ctx):
    return ctx.std.process.command("sh").args([
        "-c",
        "trap 'echo target handled SIGINT >&2; exit 0' INT; echo ready pid=$$ >&2; while :; do sleep 0.1; done",
    ]).process_group(0).spawn()

def _impl(ctx):
    supervisor = watch_supervisor.new(ctx, lambda environment, stdin: _spawn(ctx))
    supervisor.restart()
    return supervisor.run(watch.new(ctx), lambda batch: None)

watching = task(summary = "Test fixture.", implementation = _impl)
"#;
    let run = Run::start(WATCH, "watching");
    let target_pid = run.announced_pid();
    run.signal(libc::SIGINT);
    let (status, stderr) = run.finish();
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("target handled SIGINT"), "{stderr}");
    assert!(!stderr.contains("Traceback"), "{stderr}");
    assert!(
        wait_gone(target_pid, Duration::from_secs(5)),
        "the watched target outlived aspect-cli"
    );
}
