//! Children the runtime spawns, bound to a cancellation token.
//!
//! Every `std.process.Command.spawn` and every bazel client goes through
//! [`bind`]: the child gets a token of its own, a `child()` of the one the
//! caller passed (`ctx.cancellation.root` by default), and one runtime task
//! that waits for that token and then runs the child's stop sequence. The
//! sequence is what cancellation means for that kind of process, and the
//! only thing the runtime decides:
//!
//! - a plain process: the usual unix escalation, each to its whole process
//!   group when it leads one: interrupt (SIGINT; a console break on Windows),
//!   then terminate (SIGTERM) after [`CHILD_GRACE`], then kill (SIGKILL)
//!   after another;
//! - a bazel client: one interrupt (SIGINT), bazel's own graceful cancel, and
//!   nothing more. Draining its build events afterwards is AXL's job.
//!
//! The waits here are the safe points for the default ending: each runs under
//! [`Signals::block`], so a wait on a child ends the task once the root is
//! cancelled. The children themselves are already stopping by then.
//!
//! A child that shares our process group already has a signal the terminal
//! delivered (`Origin::Terminal`); the sequences skip the signal it already
//! has rather than send it twice.
//!
//! No registry of PIDs exists. [`Signals`] keeps the stop states only so that
//! process exit can wait for sequences in flight and the backstop can kill
//! what is left.

pub(crate) mod os;

use std::cell::RefCell;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::engine::cancellation::Signals;
use crate::eval::TaskExit;

pub use os::Target;

/// How long a signalled process gets to exit before the next rung: SIGINT,
/// then SIGTERM, then SIGKILL.
pub const CHILD_GRACE: Duration = Duration::from_millis(1500);

/// Bazel's own Ctrl+C sequence (https://bazel.build/run/cancellation): the
/// first SIGINT cancels the command gracefully, the second is still graceful,
/// the third makes the client kill its server and exit. The runtime replays
/// it with these timings: a tick between SIGINTs, a grace after the third
/// before a SIGKILL, and a beat after the kill for the kernel to settle.
///
/// On CI the sequence stops after the second SIGINT. Runners do not reap our
/// process tree, so the only thing that could hard-kill bazel mid-cleanup is
/// us, and a `KillServerProcess` or SIGKILL landing during sandbox setup
/// strands a sandbox tree that poisons the next command on the runner
/// (bazelbuild/bazel#23880). The graceful path lets `afterCommand` finish.
pub const BAZEL_SIGINT_TICK: Duration = Duration::from_millis(150);
pub const BAZEL_SIGINT_GRACE: Duration = Duration::from_secs(3);
pub const BAZEL_POST_KILL_GRACE: Duration = Duration::from_secs(1);

/// How often a wait checks on its child.
const POLL: Duration = Duration::from_millis(25);

/// Which stop sequence a bound child gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Process,
    Bazel,
}

/// The state a child's stop sequence reads and reports.
pub(crate) struct Stop {
    pid: u32,
    group: Option<i32>,
    kind: Kind,
    token: CancellationToken,
    signals: Arc<Signals>,
    /// The child was reaped by a wait, so its pid may be reused: never
    /// signal it again.
    exited: AtomicBool,
    /// The stop sequence ran to its end.
    finished: AtomicBool,
}

impl Stop {
    fn target(&self) -> Target {
        match self.group {
            Some(group) => Target::Group(group),
            None => Target::Pid(self.pid),
        }
    }

    pub(crate) fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// Cancelled, still alive as far as we know, and not yet done stopping.
    pub(crate) fn stopping(&self) -> bool {
        self.token.is_cancelled() && !self.finished.load(Ordering::SeqCst) && !self.gone()
    }

    /// Still alive as far as we know: the whole group for a group leader.
    pub(crate) fn alive(&self) -> bool {
        !self.gone()
    }

    pub(crate) fn is_bazel(&self) -> bool {
        self.kind == Kind::Bazel
    }

    /// The backstop's rung: kill outright, unless the child is already gone.
    pub(crate) fn kill(&self) {
        if !self.gone() {
            os::kill(self.target());
        }
    }

    /// The child has gone, as far as we can tell without reaping it. For a
    /// process group that means every member: a shell exits on SIGINT while
    /// the jobs it started with `&` ignore it, so the leader going is not
    /// the group going. (An unreaped leader keeps its group non-empty, so the
    /// remaining rungs still run; they are harmless on an empty group.)
    fn gone(&self) -> bool {
        match self.group {
            Some(group) => !os::is_running(Target::Group(group)),
            None => self.exited() || os::has_exited(self.pid),
        }
    }

    /// Wait up to `grace` for the child to go, checking every [`POLL`].
    async fn linger(&self, grace: Duration) -> bool {
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if self.gone() {
                return true;
            }
            tokio::time::sleep(POLL).await;
        }
        self.gone()
    }

    /// Whether a terminal already delivered the signal to this child: it
    /// shares our process group, and the signal came from the terminal.
    fn already_signalled(&self) -> bool {
        self.group.is_none() && self.signals.group_signalled()
    }

    /// What cancellation means for this child. Runs once, on its runtime
    /// task, after the token is cancelled.
    async fn run(&self) {
        if self.gone() {
            self.finished.store(true, Ordering::SeqCst);
            return;
        }
        match self.kind {
            Kind::Bazel => self.stop_bazel().await,
            Kind::Process => self.stop_process().await,
        }
        self.finished.store(true, Ordering::SeqCst);
    }

    /// Interrupt, terminate, kill, with [`CHILD_GRACE`] between each. A child
    /// the terminal already interrupted starts at the grace.
    async fn stop_process(&self) {
        let target = self.target();
        if !self.already_signalled() {
            os::interrupt(target);
        }
        if self.linger(CHILD_GRACE).await {
            return;
        }
        os::terminate(target);
        if self.linger(CHILD_GRACE).await {
            return;
        }
        os::kill(target);
    }

    /// Bazel's Ctrl+C sequence (see [`BAZEL_SIGINT_TICK`]). A terminal's
    /// SIGINT counts as the first rung.
    async fn stop_bazel(&self) {
        let target = Target::Pid(self.pid);
        let mut sent = if self.already_signalled() { 1 } else { 0 };
        while sent < 2 {
            if sent > 0 {
                tokio::time::sleep(BAZEL_SIGINT_TICK).await;
            }
            if self.gone() {
                return;
            }
            os::interrupt(target);
            sent += 1;
        }
        if crate::ci::on_recognized_ci() {
            return;
        }
        tokio::time::sleep(BAZEL_SIGINT_TICK).await;
        if self.gone() {
            return;
        }
        os::interrupt(target);
        if self.linger(BAZEL_SIGINT_GRACE).await {
            return;
        }
        os::kill(target);
        tokio::time::sleep(BAZEL_POST_KILL_GRACE).await;
    }
}

/// A spawned child's binding: its own token, and the stop state behind it.
/// Held by the `std.process.Child` / `bazel.Build` value that owns the child.
pub struct Bound {
    token: CancellationToken,
    stop: Arc<Stop>,
}

impl std::fmt::Debug for Bound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bound")
            .field("pid", &self.stop.pid)
            .field("group", &self.stop.group)
            .field("kind", &self.stop.kind)
            .field("cancelled", &self.token.is_cancelled())
            .field("exited", &self.stop.exited())
            .finish()
    }
}

impl Bound {
    /// The child's token, `handle.cancellation` in AXL.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// What a signal sent now is aimed at: the group the child leads, else
    /// the child.
    pub fn target(&self) -> Target {
        self.stop.target()
    }

    /// The run's cancellation state this child belongs to.
    pub fn signals(&self) -> &Arc<Signals> {
        &self.stop.signals
    }

    /// Whether the child has been reaped. Its pid may belong to someone else
    /// from then on, so nothing may be sent to it.
    pub fn exited(&self) -> bool {
        self.stop.exited()
    }

    /// The child has been reaped. Its pid may be reused from here, so nothing
    /// is sent to it alone again; a group it led is still stopped as a whole.
    pub fn mark_exited(&self) {
        self.stop.exited.store(true, Ordering::SeqCst);
        if self.stop.group.is_none() {
            self.stop.signals.forget(&self.stop);
        }
    }
}

/// Bind a just-spawned child to `parent`: a `child()` token of its own and
/// the runtime task that stops it when that token is cancelled. `group` is
/// the process group the child leads, when it was spawned into one.
pub fn bind(
    signals: &Arc<Signals>,
    parent: &CancellationToken,
    pid: u32,
    group: Option<i32>,
    kind: Kind,
) -> Bound {
    let token = parent.child_token();
    let stop = Arc::new(Stop {
        pid,
        group,
        kind,
        token: token.clone(),
        signals: signals.clone(),
        exited: AtomicBool::new(false),
        finished: AtomicBool::new(false),
    });
    signals.track(stop.clone());
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let waiting = stop.clone();
        // The binding stays tracked after its sequence: a bazel client the
        // ladder left running on CI, or anything that ignored the kill, must
        // still be found by the backstop. Only a reap lets go of it.
        handle.spawn(async move {
            waiting.token.cancelled().await;
            waiting.run().await;
            if waiting.gone() {
                waiting.signals.forget(&waiting);
            }
        });
    }
    Bound { token, stop }
}

/// Spawn `cmd` bound to `parent`: the runtime's spawn-time configuration,
/// then [`bind`]. `group` is the process group the command was configured
/// to lead, if any.
pub fn spawn(
    cmd: &mut Command,
    signals: &Arc<Signals>,
    parent: &CancellationToken,
    group: Option<i32>,
    kind: Kind,
) -> io::Result<(Child, Bound)> {
    os::creation_flags(cmd);
    let child = cmd.spawn()?;
    let group = group.map(|g| if g == 0 { child.id() as i32 } else { g });
    let bound = bind(signals, parent, child.id(), group, kind);
    Ok((child, bound))
}

/// Spawn a bazel client bound to the root.
pub fn spawn_bazel(cmd: &mut Command, signals: &Arc<Signals>) -> io::Result<Spawned> {
    let (child, bound) = spawn(cmd, signals, signals.root(), None, Kind::Bazel)?;
    Ok(Spawned {
        child: RefCell::new(Some(child)),
        bound,
    })
}

/// A child spawned by the runtime for its own purposes (a `bazel info`, a
/// query): the handle and its binding, with the cancellation-aware waits.
pub struct Spawned {
    child: RefCell<Option<Child>>,
    bound: Bound,
}

impl Spawned {
    pub fn wait(&self) -> io::Result<ExitStatus> {
        wait(&self.child, &self.bound, None)?
            .ok_or_else(|| io::Error::other("wait without a timeout returned nothing"))
    }

    pub fn wait_with_output(self) -> io::Result<Output> {
        let child = self
            .child
            .take()
            .ok_or_else(|| io::Error::other("child is no longer active"))?;
        wait_with_output(child, &self.bound)
    }
}

/// What a cancellable receive produced.
pub enum Recv<T> {
    Item(T),
    /// `tick` passed with nothing to deliver.
    Tick,
    /// The sender is gone.
    Closed,
    /// The root token was cancelled while waiting.
    Cancelled,
}

/// One bounded receive on a channel, whichever crate's.
pub enum Slice<T> {
    Item(T),
    Empty,
    Closed,
}

/// A channel receiver [`recv_cancellable`] can poll.
pub trait RecvSlice<T> {
    fn recv_slice(&self, slice: Duration) -> Slice<T>;
}

impl<T> RecvSlice<T> for std::sync::mpsc::Receiver<T> {
    fn recv_slice(&self, slice: Duration) -> Slice<T> {
        use std::sync::mpsc::RecvTimeoutError;
        match self.recv_timeout(slice) {
            Ok(item) => Slice::Item(item),
            Err(RecvTimeoutError::Timeout) => Slice::Empty,
            Err(RecvTimeoutError::Disconnected) => Slice::Closed,
        }
    }
}

impl<T: Send + Clone> RecvSlice<T> for fibre::spmc::Receiver<T> {
    fn recv_slice(&self, slice: Duration) -> Slice<T> {
        use fibre::RecvErrorTimeout;
        match self.recv_timeout(slice) {
            Ok(item) => Slice::Item(item),
            Err(RecvErrorTimeout::Timeout) => Slice::Empty,
            Err(RecvErrorTimeout::Disconnected) => Slice::Closed,
        }
    }
}

/// Receive from `recv`, checking the root token every [`POLL`], for at most
/// `tick` when given. For iterators, which cannot raise: on `Cancelled` they
/// end, and the loop's next call raises the task's exit.
pub fn recv_cancellable<T>(
    signals: &Signals,
    recv: &impl RecvSlice<T>,
    tick: Option<Duration>,
) -> Recv<T> {
    let deadline = tick.map(|t| Instant::now() + t);
    loop {
        if signals.should_unwind() {
            return Recv::Cancelled;
        }
        let slice = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()).min(POLL),
            None => POLL,
        };
        match recv.recv_slice(slice) {
            Slice::Item(item) => return Recv::Item(item),
            Slice::Closed => return Recv::Closed,
            Slice::Empty => {
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return Recv::Tick;
                }
            }
        }
    }
}

/// The exit a cancelled wait reports, as the `io::Error` the std-shaped
/// helpers return. `TaskExit::from_anyhow` looks inside an `io::Error` for it.
fn cancelled(exit: TaskExit) -> io::Error {
    io::Error::other(exit)
}

/// Wait for a child to exit, polling `poll` so the wait is a safe point:
/// once the root is cancelled it returns the task's exit instead. With a
/// `timeout`, `Ok(None)` when it passes first. Reaping marks the binding
/// exited. `poll` is the child's `try_wait`, however the caller stores it.
pub fn wait_with(
    bound: &Bound,
    timeout: Option<Duration>,
    mut poll: impl FnMut() -> io::Result<Option<ExitStatus>>,
) -> io::Result<Option<ExitStatus>> {
    let deadline = timeout.map(|t| Instant::now() + t);
    let status = bound
        .signals()
        .block(async {
            loop {
                if let Some(status) = poll()? {
                    return Ok::<_, io::Error>(Some(status));
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return Ok(None);
                }
                tokio::time::sleep(POLL).await;
            }
        })
        .map_err(cancelled)??;
    if status.is_some() {
        bound.mark_exited();
    }
    Ok(status)
}

/// [`wait_with`] over a `std.process.Child`'s slot. Like `Child::wait`, the
/// child's stdin is closed first so it cannot block on us.
pub fn wait(
    cell: &RefCell<Option<Child>>,
    bound: &Bound,
    timeout: Option<Duration>,
) -> io::Result<Option<ExitStatus>> {
    {
        let mut guard = cell.borrow_mut();
        let child = guard
            .as_mut()
            .ok_or_else(|| io::Error::other("child is no longer active"))?;
        drop(child.stdin.take());
    }
    wait_with(bound, timeout, || {
        let mut guard = cell.borrow_mut();
        let child = guard
            .as_mut()
            .ok_or_else(|| io::Error::other("child is no longer active"))?;
        child.try_wait()
    })
}

/// `Child::try_wait`, marking the binding exited when the child is reaped.
pub fn try_wait(child: &mut Child, bound: &Bound) -> io::Result<Option<ExitStatus>> {
    let status = child.try_wait()?;
    if status.is_some() {
        bound.mark_exited();
    }
    Ok(status)
}

/// `Child::wait_with_output` as a safe point: the pipes drain on threads of
/// their own while the exit is polled under [`Signals::block`].
pub fn wait_with_output(mut child: Child, bound: &Bound) -> io::Result<Output> {
    drop(child.stdin.take());
    let stdout = child.stdout.take().map(drain_on_thread);
    let stderr = child.stderr.take().map(drain_on_thread);
    let cell = RefCell::new(Some(child));
    let status = wait(&cell, bound, None)?
        .ok_or_else(|| io::Error::other("wait without a timeout returned nothing"))?;
    Ok(Output {
        status,
        stdout: join_drained(stdout)?,
        stderr: join_drained(stderr)?,
    })
}

fn drain_on_thread<R: Read + Send + 'static>(
    mut reader: R,
) -> std::thread::JoinHandle<io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

fn join_drained(
    handle: Option<std::thread::JoinHandle<io::Result<Vec<u8>>>>,
) -> io::Result<Vec<u8>> {
    match handle {
        Some(handle) => handle
            .join()
            .map_err(|_| io::Error::other("output reader thread panicked"))?,
        None => Ok(Vec::new()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::engine::cancellation::{Kind as SignalKind, Origin};
    use std::process::Stdio;

    fn sleeper(group: bool) -> Command {
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new("sleep");
        cmd.arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if group {
            cmd.process_group(0);
        }
        cmd
    }

    fn with_runtime<R>(f: impl FnOnce(Arc<Signals>) -> R) -> R {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        f(Signals::new())
    }

    #[test]
    fn cancelling_the_token_interrupts_the_child() {
        with_runtime(|signals| {
            let token = CancellationToken::new();
            let (child, bound) =
                spawn(&mut sleeper(false), &signals, &token, None, Kind::Process).unwrap();
            let cell = RefCell::new(Some(child));
            token.cancel();
            let status = wait(&cell, &bound, Some(Duration::from_secs(5)))
                .unwrap()
                .expect("child exits after the interrupt");
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                status.signal(),
                Some(nix::sys::signal::Signal::SIGINT as i32)
            );
            assert!(!bound.stop.stopping());
        });
    }

    #[test]
    fn a_child_ignoring_interrupt_and_terminate_is_killed_after_the_graces() {
        with_runtime(|signals| {
            let token = CancellationToken::new();
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "trap '' INT TERM; sleep 30"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let (child, bound) = spawn(&mut cmd, &signals, &token, None, Kind::Process).unwrap();
            let cell = RefCell::new(Some(child));
            std::thread::sleep(Duration::from_millis(200));
            let started = Instant::now();
            token.cancel();
            let status = wait(
                &cell,
                &bound,
                Some(CHILD_GRACE * 2 + Duration::from_secs(5)),
            )
            .unwrap()
            .expect("child is killed after the graces");
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                status.signal(),
                Some(nix::sys::signal::Signal::SIGKILL as i32)
            );
            assert!(started.elapsed() >= CHILD_GRACE * 2);
        });
    }

    #[test]
    fn a_group_leader_takes_its_descendants_with_it() {
        with_runtime(|signals| {
            let dir = tempfile::tempdir().unwrap();
            let pid_file = dir.path().join("descendant.pid");
            let token = CancellationToken::new();
            let mut cmd = Command::new("sh");
            cmd.arg("-c")
                .arg("sleep 30 & echo $! > \"$PID_FILE\"; wait")
                .env("PID_FILE", &pid_file)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            {
                use std::os::unix::process::CommandExt;
                cmd.process_group(0);
            }
            let (child, bound) = spawn(&mut cmd, &signals, &token, Some(0), Kind::Process).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !pid_file.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let descendant: u32 = std::fs::read_to_string(&pid_file)
                .expect("descendant pid file")
                .trim()
                .parse()
                .unwrap();
            let cell = RefCell::new(Some(child));
            token.cancel();
            wait(&cell, &bound, Some(Duration::from_secs(5)))
                .unwrap()
                .expect("leader exits");
            let deadline = Instant::now() + Duration::from_secs(5);
            while os::is_running(Target::Pid(descendant)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                !os::is_running(Target::Pid(descendant)),
                "descendant survived"
            );
        });
    }

    /// A stand-in for the bazel client: counts the SIGINTs it receives and
    /// exits on the third, as bazel's `KillServerProcess` rung would.
    #[test]
    fn a_bazel_child_gets_bazels_sigint_ladder() {
        with_runtime(|signals| {
            let dir = tempfile::tempdir().unwrap();
            let count = dir.path().join("count");
            let token = CancellationToken::new();
            let mut cmd = Command::new("sh");
            cmd.arg("-c")
                .arg("n=0; trap 'n=$((n+1)); echo $n > \"$COUNT\"; [ $n -ge 3 ] && exit 0' INT; while :; do sleep 0.05; done")
                .env("COUNT", &count)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let (child, bound) = spawn(&mut cmd, &signals, &token, None, Kind::Bazel).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            token.cancel();
            let cell = RefCell::new(Some(child));
            let status = wait(&cell, &bound, Some(Duration::from_secs(5))).unwrap();
            let sent: u32 = std::fs::read_to_string(&count)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            if crate::ci::on_recognized_ci() {
                // Two graceful SIGINTs, then nothing: the client is left to finish.
                assert_eq!(sent, 2);
                assert!(status.is_none());
                let mut guard = cell.borrow_mut();
                let child = guard.as_mut().unwrap();
                child.kill().unwrap();
                child.wait().unwrap();
            } else {
                assert_eq!(sent, 3);
                assert_eq!(status.expect("exits on the third rung").code(), Some(0));
            }
        });
    }

    /// A binding stays known to the backstop for as long as its child is
    /// alive, through the whole sequence, and only drops out once the child
    /// is gone.
    #[test]
    fn a_binding_stays_tracked_while_the_child_is_alive() {
        with_runtime(|signals| {
            let token = CancellationToken::new();
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "trap '' INT TERM; while :; do sleep 0.05; done"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let (child, bound) = spawn(&mut cmd, &signals, &token, None, Kind::Process).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            token.cancel();
            // Interrupted and ignoring it: still alive, still tracked.
            std::thread::sleep(Duration::from_millis(500));
            assert_eq!(
                signals.alive().len(),
                1,
                "tracked while it ignores the signal"
            );
            let cell = RefCell::new(Some(child));
            wait(
                &cell,
                &bound,
                Some(CHILD_GRACE * 2 + Duration::from_secs(5)),
            )
            .unwrap()
            .expect("killed at the end of the sequence");
            assert!(signals.alive().is_empty(), "let go once gone");
        });
    }

    /// The terminal already interrupted a child sharing our process group:
    /// the runtime does not send that interrupt again, only the rungs after
    /// the grace.
    #[test]
    fn a_child_the_terminal_already_signalled_is_not_signalled_again() {
        with_runtime(|signals| {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("int");
            let mut cmd = Command::new("sh");
            cmd.arg("-c")
                .arg("trap 'touch \"$MARKER\"' INT; trap 'exit 0' TERM; while :; do sleep 0.05; done")
                .env("MARKER", &marker)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let (child, bound) =
                spawn(&mut cmd, &signals, signals.root(), None, Kind::Process).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            signals.record(SignalKind::Interrupt, Origin::Terminal);
            let cell = RefCell::new(Some(child));
            let status = wait(
                &cell,
                &bound,
                Some(CHILD_GRACE * 2 + Duration::from_secs(5)),
            )
            .unwrap()
            .expect("the child exits on the terminate rung");
            assert_eq!(status.code(), Some(0));
            assert!(!marker.exists(), "the child received a second interrupt");
        });
    }

    #[test]
    fn a_wait_ends_with_the_exit_once_the_body_is_forced() {
        with_runtime(|signals| {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "trap '' INT TERM; sleep 30"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let (child, bound) =
                spawn(&mut cmd, &signals, signals.root(), None, Kind::Process).unwrap();
            let cell = RefCell::new(Some(child));
            let raiser = signals.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                raiser.record(SignalKind::Interrupt, Origin::Process);
                raiser.force();
            });
            let err = wait(&cell, &bound, None).expect_err("the wait ends with the exit");
            let exit = err
                .get_ref()
                .and_then(|e| e.downcast_ref::<TaskExit>())
                .expect("an io::Error carrying the TaskExit");
            assert_eq!(exit.code, 130);
            // The child is stopping on its own; wait for it so the test leaves nothing behind.
            signals.enter_unwind();
            wait(
                &cell,
                &bound,
                Some(CHILD_GRACE * 2 + Duration::from_secs(5)),
            )
            .unwrap()
            .expect("child exits");
        });
    }

    #[test]
    fn wait_with_output_collects_both_pipes() {
        with_runtime(|signals| {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "echo out; echo err >&2; exit 3"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let (child, bound) =
                spawn(&mut cmd, &signals, signals.root(), None, Kind::Process).unwrap();
            let out = wait_with_output(child, &bound).unwrap();
            assert_eq!(out.status.code(), Some(3));
            assert_eq!(out.stdout, b"out\n");
            assert_eq!(out.stderr, b"err\n");
            assert!(bound.stop.exited());
        });
    }
}
