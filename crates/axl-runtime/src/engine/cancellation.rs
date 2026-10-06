//! Cancellation: how an OS signal, or AXL itself, stops a run.
//!
//! Cancellation is a tree of tokens (tokio-util's [`CancellationToken`]).
//! The run has one root, [`Signals::root`], which the OS cancels when the
//! process receives Ctrl+C or SIGTERM, and which AXL reaches as
//! `ctx.cancellation.root`. Every process the runtime spawns is bound to a
//! token at its spawn (`engine/children.rs`) and is stopped when that token
//! is cancelled. The AXL body is left alone: its waits return as its children
//! stop, its loops read `root.cancelled`, and it ends the way it chooses. Only
//! a deadline the binary's signal task sets ([`Signals::force`]), or a second
//! signal, ends a body that has not returned: every blocking builtin runs
//! under [`Signals::block`], so the body then ends at its next blocking call
//! the way `ctx.std.process.exit(130, "interrupted")` would, and post-task
//! hooks, `ctx.defer` callbacks and the bookend still run. Nothing in the
//! runtime ever calls back into AXL on a signal.
//!
//! The run's [`Signals`] travels with its context: `Env.signals` for every
//! builtin that has an evaluator, the `ctx` values for the attribute getters,
//! and the iterators and handles that hold it from construction. The binary's
//! signal task records into [`Signals::global`], which is also what a fresh
//! `Env` starts with; the test harness gives each run a `Signals` of its own.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use allocative::Allocative;
use derive_more::Display;
use starlark::StarlarkResultExt;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::{self, NoSerialize, ProvidesStaticType, ValueLike, starlark_value};
use tokio_util::sync::CancellationToken;

use crate::engine::children::Stop;
use crate::eval::TaskExit;

/// What the operating system asked for. AXL never sees the signal's name;
/// the kind only picks the conventional exit code of the default ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// SIGINT, Ctrl+C or Ctrl+Break: someone at the console.
    Interrupt,
    /// SIGTERM, or a console close, logoff or shutdown: an orchestrator that
    /// expects a bounded wind-down.
    Terminate,
}

impl Kind {
    /// The shell convention: 128 + the signal's number.
    pub fn exit_code(self) -> u8 {
        match self {
            Kind::Interrupt => 130,
            Kind::Terminate => 143,
        }
    }

    /// The `ERROR:` line the default ending prints.
    pub fn message(self) -> &'static str {
        match self {
            Kind::Interrupt => "interrupted",
            Kind::Terminate => "terminated",
        }
    }

    /// How the one stderr line names the request.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Interrupt => "interrupt",
            Kind::Terminate => "terminate",
        }
    }
}

/// Who sent the signal, which decides who else already has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `kill(2)` from another process (a CI runner, a script): delivered to
    /// this process alone.
    Process,
    /// The terminal (Ctrl+C), which delivers to the whole foreground process
    /// group: every child sharing our group already has the signal, and the
    /// runtime must not send it a second one.
    Terminal,
}

/// The run's cancellation state: the root token, what the OS asked for, and
/// the children whose stop sequences are in flight.
pub struct Signals {
    root: CancellationToken,
    /// Cancelled when the body is to be ended: the deadline passed, or a
    /// second signal arrived. What the safe points and the evaluator's check
    /// watch.
    force: CancellationToken,
    kind: OnceLock<Kind>,
    count: AtomicU32,
    /// A signal reached our whole process group (it came from the terminal),
    /// so children in that group already have it.
    group_signalled: AtomicBool,
    /// Set once the task body has ended and the post-task hooks and defers
    /// are running: blocking builtins block normally again so cleanup can
    /// wait on things.
    unwinding: AtomicBool,
    stops: Mutex<Vec<Arc<Stop>>>,
}

impl std::fmt::Debug for Signals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signals")
            .field("root_cancelled", &self.root.is_cancelled())
            .field("kind", &self.kind())
            .field("count", &self.count.load(Ordering::SeqCst))
            .finish()
    }
}

impl Signals {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            root: CancellationToken::new(),
            force: CancellationToken::new(),
            kind: OnceLock::new(),
            count: AtomicU32::new(0),
            group_signalled: AtomicBool::new(false),
            unwinding: AtomicBool::new(false),
            stops: Mutex::new(Vec::new()),
        })
    }

    /// The process-wide instance: what the binary's signal task records into,
    /// and what a fresh `Env` starts with.
    pub fn global() -> &'static Arc<Signals> {
        static GLOBAL: OnceLock<Arc<Signals>> = OnceLock::new();
        GLOBAL.get_or_init(Signals::new)
    }

    /// The run's root token.
    pub fn root(&self) -> &CancellationToken {
        &self.root
    }

    /// What a spawn with no explicit `cancellation =` is bound to: the root,
    /// unless it is already cancelled. A process started after the cancel is
    /// cleanup, not part of what was cancelled: it gets a token of its own,
    /// and the backstop still covers it.
    pub fn default_binding(&self) -> CancellationToken {
        if self.root.is_cancelled() {
            CancellationToken::new()
        } else {
            self.root.clone()
        }
    }

    /// The OS asked the process to stop: cancel the root. Returns how many
    /// requests have arrived, so the caller can tell a repeat from the first.
    pub fn record(&self, kind: Kind, origin: Origin) -> u32 {
        let _ = self.kind.set(kind);
        if origin == Origin::Terminal {
            self.group_signalled.store(true, Ordering::SeqCst);
        }
        let count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        self.root.cancel();
        count
    }

    /// End the body: the deadline for returning on its own has passed, or a
    /// second signal arrived. From here every blocking builtin and the
    /// evaluator's check end the task with [`Signals::exit_for`].
    pub fn force(&self) {
        self.force.cancel();
    }

    /// Whether any child bound to the root is still alive: what decides how
    /// long the body gets after a signal before it is ended.
    pub fn has_live_children(&self) -> bool {
        !self.alive().is_empty()
    }

    /// What the OS asked for, if it did.
    pub fn kind(&self) -> Option<Kind> {
        self.kind.get().copied()
    }

    /// Whether a signal already reached every child in our process group.
    pub fn group_signalled(&self) -> bool {
        self.group_signalled.load(Ordering::SeqCst)
    }

    /// The task body has ended; blocking builtins block normally from here.
    pub fn enter_unwind(&self) {
        self.unwinding.store(true, Ordering::SeqCst);
    }

    pub fn unwinding(&self) -> bool {
        self.unwinding.load(Ordering::SeqCst)
    }

    /// Whether the body should end now: it was forced and the post-task hooks
    /// have not started yet. The starlark bytecode check and every blocking
    /// builtin ask this.
    pub fn should_unwind(&self) -> bool {
        self.force.is_cancelled() && !self.unwinding()
    }

    /// The exit the default ending reports: 130 or 143 after a signal, 130
    /// when AXL cancelled the root itself.
    pub fn exit_for(&self) -> TaskExit {
        match self.kind() {
            Some(kind) => TaskExit::new(kind.exit_code(), Some(kind.message().to_owned())),
            None => TaskExit::new(Kind::Interrupt.exit_code(), Some("cancelled".to_owned())),
        }
    }

    /// Drive `fut` to completion on the runtime, unless the body is forced to
    /// end first, in which case the exit the task should end with is returned
    /// instead. While unwinding, `fut` runs to completion regardless. Every
    /// blocking builtin goes through here.
    pub fn block<F: std::future::Future>(&self, fut: F) -> Result<F::Output, TaskExit> {
        let handle = tokio::runtime::Handle::current();
        if self.unwinding() {
            return Ok(handle.block_on(fut));
        }
        match handle.block_on(self.force.run_until_cancelled(fut)) {
            Some(out) => Ok(out),
            None => Err(self.exit_for()),
        }
    }

    pub(crate) fn track(&self, stop: Arc<Stop>) {
        if let Ok(mut stops) = self.stops.lock() {
            stops.push(stop);
        }
    }

    pub(crate) fn forget(&self, stop: &Arc<Stop>) {
        if let Ok(mut stops) = self.stops.lock() {
            stops.retain(|s| !Arc::ptr_eq(s, stop));
        }
    }

    /// The bound children whose stop sequence has begun and not finished:
    /// what process exit waits for.
    pub(crate) fn stopping(&self) -> Vec<Arc<Stop>> {
        self.stops
            .lock()
            .map(|stops| stops.iter().filter(|s| s.stopping()).cloned().collect())
            .unwrap_or_default()
    }

    /// Every bound child still alive, for the backstop.
    pub(crate) fn alive(&self) -> Vec<Arc<Stop>> {
        self.stops
            .lock()
            .map(|stops| stops.iter().filter(|s| s.alive()).cloned().collect())
            .unwrap_or_default()
    }

    /// Wait until every stop sequence in flight has finished, or `timeout`
    /// passes. Returns whether they all finished.
    pub async fn drain(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.stopping().is_empty() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// The backstop: kill every bound child still alive. With `spare_bazel`,
    /// bazel clients are left to the host's own escalation (a SIGKILL during
    /// sandbox cleanup is bazelbuild/bazel#23880).
    pub fn kill_all(&self, spare_bazel: bool) -> usize {
        let mut killed = 0;
        for stop in self.alive() {
            if spare_bazel && stop.is_bazel() {
                continue;
            }
            stop.kill();
            killed += 1;
        }
        killed
    }
}

/// `"Evaluation cancelled"` is what the evaluator raises when the bytecode
/// cancel check fires. Its error type is private to starlark, so the text is
/// matched, and only once the body really was forced: nothing else makes the
/// check return true.
pub fn is_cancelled_error(err: &starlark::Error, signals: &Signals) -> bool {
    match err.kind() {
        starlark::ErrorKind::Other(inner) => {
            inner.to_string() == "Evaluation cancelled" && signals.force.is_cancelled()
        }
        _ => false,
    }
}

/// A cancellation token: `ctx.cancellation.root`, what `ctx.cancellation.new()`
/// returns, a token's `child()`, and `handle.cancellation` on
/// every process the runtime spawned.
///
/// Cancelling a token cancels every token derived from it with `child()`,
/// never its parent. A process bound to a token (`cancellation = tok` at its
/// spawn) is stopped when the token is cancelled, the way its kind needs: a
/// `std.process.Child` gets SIGINT, then SIGTERM, then SIGKILL, a pause
/// between each; a `bazel.Build` gets bazel's own Ctrl+C sequence.
///
/// Libraries take a token as a plain argument and default it to
/// `ctx.cancellation.root`; their loops check `cancellation.cancelled`.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<cancellation.Token>")]
pub struct Token {
    #[allocative(skip)]
    inner: CancellationToken,
    #[allocative(skip)]
    signals: Arc<Signals>,
}

starlark_simple_value!(Token);

impl Token {
    pub fn new(inner: CancellationToken, signals: Arc<Signals>) -> Self {
        Self { inner, signals }
    }

    pub fn inner(&self) -> &CancellationToken {
        &self.inner
    }
}

#[starlark_value(type = "cancellation.Token")]
impl<'v> values::StarlarkValue<'v> for Token {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("token_methods", token_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn token_methods(registry: &mut MethodsBuilder) {
    /// A token cancelled whenever this one is. Cancelling the child never
    /// affects this token. Bind a piece of work to a child to be able to stop
    /// just that piece: `sub = ctx.cancellation.root.child()`.
    fn child<'v>(this: values::Value<'v>) -> anyhow::Result<Token> {
        let token = this.downcast_ref_err::<Token>().into_anyhow_result()?;
        Ok(Token::new(token.inner.child_token(), token.signals.clone()))
    }

    /// Cancel this token and every token derived from it, and wake every
    /// `wait()`. Every process bound to one of them starts its stop sequence.
    /// Cancelling `ctx.cancellation.root` ends the task as Ctrl+C would.
    fn cancel<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let token = this.downcast_ref_err::<Token>().into_anyhow_result()?;
        token.inner.cancel();
        Ok(NoneType)
    }

    /// Whether this token has been cancelled, by `cancel()` on it or on an
    /// ancestor, or by the operating system for the root. What a loop checks
    /// on its tick.
    #[starlark(attribute)]
    fn cancelled<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let token = this.downcast_ref_err::<Token>().into_anyhow_result()?;
        Ok(token.inner.is_cancelled())
    }

    /// Block until this token is cancelled, or `timeout_ms` passes. Returns
    /// `True` when it was cancelled, `False` on the timeout.
    fn wait<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<i32>,
    ) -> anyhow::Result<bool> {
        let this = this.downcast_ref_err::<Token>().into_anyhow_result()?;
        let token = this.inner.clone();
        let timeout = match timeout_ms.into_option() {
            Some(ms) if ms < 0 => anyhow::bail!("timeout_ms must not be negative: {ms}"),
            Some(ms) => Some(Duration::from_millis(ms as u64)),
            None => None,
        };
        Ok(this.signals.block(async move {
            match timeout {
                Some(timeout) => tokio::time::timeout(timeout, token.cancelled())
                    .await
                    .is_ok(),
                None => {
                    token.cancelled().await;
                    true
                }
            }
        })?)
    }
}

/// `ctx.cancellation`: where a task gets its tokens.
///
/// - `root` is cancelled by the operating system (Ctrl+C, SIGTERM) or by
///   you, and is what every spawn is bound to unless told otherwise.
/// - `new()` is a token nothing cancels but you.
///
/// Cancelling the root stops what is bound to it and nothing else: the task
/// keeps running, its waits return as the children stop, and it ends the way
/// it chooses. A task that did not return within the deadline is ended with
/// exit 130 (143 for SIGTERM).
///
/// ```python
/// def _impl(ctx):
///     child = ctx.std.process.command("dev-server").spawn()
///     for _tick in sleep_iter(100):
///         if ctx.cancellation.root.cancelled:
///             child.wait()          # already stopping
///             return 0
/// ```
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<cancellation.Cancellation>")]
pub struct Cancellation {
    #[allocative(skip)]
    signals: Arc<Signals>,
}

starlark_simple_value!(Cancellation);

impl Cancellation {
    pub fn new(signals: Arc<Signals>) -> Self {
        Self { signals }
    }
}

#[starlark_value(type = "cancellation.Cancellation")]
impl<'v> values::StarlarkValue<'v> for Cancellation {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("cancellation_methods", cancellation_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn cancellation_methods(registry: &mut MethodsBuilder) {
    /// The run's root token. The operating system cancels it on Ctrl+C or
    /// SIGTERM; `root.cancel()` does the same from AXL. Every process spawned
    /// without an explicit `cancellation =` is bound to it and stops when it
    /// is cancelled. The task itself keeps running: read `root.cancelled` on
    /// a tick to end on your own terms, or let your waits return as the
    /// children stop. A task still running at the deadline is ended with exit
    /// 130 (143 for SIGTERM), its post-task hooks, `ctx.defer` callbacks and
    /// bookend intact.
    #[starlark(attribute)]
    fn root<'v>(this: values::Value<'v>) -> anyhow::Result<Token> {
        let this = this
            .downcast_ref_err::<Cancellation>()
            .into_anyhow_result()?;
        Ok(Token::new(
            this.signals.root().clone(),
            this.signals.clone(),
        ))
    }

    /// A token nothing cancels but you. Bind a process to it to keep it out
    /// of Ctrl+C and stop it yourself, with `cancel()` on the token or
    /// `interrupt()`, `terminate()` and `kill()` on the handle.
    fn new<'v>(this: values::Value<'v>) -> anyhow::Result<Token> {
        let this = this
            .downcast_ref_err::<Cancellation>()
            .into_anyhow_result()?;
        Ok(Token::new(CancellationToken::new(), this.signals.clone()))
    }
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Cancellation: StarlarkValueAsType<Cancellation> = StarlarkValueAsType::new();
    const Token: StarlarkValueAsType<Token> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    globals.namespace("cancellation", register_types);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_cancels_the_root_and_nothing_more() {
        let s = Signals::new();
        assert!(!s.root().is_cancelled());
        assert_eq!(s.record(Kind::Interrupt, Origin::Process), 1);
        assert!(s.root().is_cancelled());
        // The body is not ended by the root, only by the deadline.
        assert!(!s.should_unwind());
        s.force();
        assert!(s.should_unwind());
        assert_eq!(s.exit_for(), TaskExit::new(130, Some("interrupted".into())));
        assert_eq!(s.record(Kind::Terminate, Origin::Process), 2);
    }

    #[test]
    fn unwinding_turns_the_check_off() {
        let s = Signals::new();
        s.record(Kind::Terminate, Origin::Process);
        s.force();
        s.enter_unwind();
        assert!(!s.should_unwind());
        assert!(s.root().is_cancelled());
    }

    #[test]
    fn block_returns_the_exit_once_the_body_is_forced() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let s = Signals::new();
        assert_eq!(s.block(async { 7 }), Ok(7));
        s.record(Kind::Interrupt, Origin::Process);
        // Cancelled, not forced: a blocking call still blocks.
        assert_eq!(s.block(async { 9 }), Ok(9));
        s.force();
        assert_eq!(
            s.block(std::future::pending::<()>()),
            Err(TaskExit::new(130, Some("interrupted".into())))
        );
        s.enter_unwind();
        assert_eq!(s.block(async { 8 }), Ok(8));
    }

    #[test]
    fn a_spawn_after_the_cancel_gets_a_token_of_its_own() {
        let s = Signals::new();
        assert!(!s.default_binding().is_cancelled());
        s.record(Kind::Interrupt, Origin::Process);
        assert!(!s.default_binding().is_cancelled());
    }

    /// A file the AXL under test appends one line per event to.
    struct Trace {
        _dir: tempfile::TempDir,
        path: String,
    }

    impl Trace {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("trace").to_string_lossy().into_owned();
            Self { _dir: dir, path }
        }

        fn read(&self) -> String {
            std::fs::read_to_string(&self.path).unwrap_or_default()
        }
    }

    /// Run `body` as a task with a post-task hook and a defer that each
    /// append a line, under a private `Signals`. Another thread records
    /// `kind` after `after`, and forces the body after `force_after` when
    /// given, the way the binary's deadline would.
    fn run_signalled(
        body: &str,
        kind: Kind,
        after: Duration,
        force_after: Option<Duration>,
    ) -> (anyhow::Result<Option<u8>>, String, Arc<Signals>) {
        let trace = Trace::new();
        let signals = Signals::new();
        let raiser = signals.clone();
        std::thread::spawn(move || {
            std::thread::sleep(after);
            raiser.record(kind, Origin::Process);
            if let Some(force_after) = force_after {
                std::thread::sleep(force_after);
                raiser.force();
            }
        });
        let code = format!(
            r#"
load("@std//time.axl", "sleep", "sleep_iter")

def _post(ctx, outcome):
    ctx.std.fs.try_append("{path}", "post:" + str(outcome.exit_code) + "\n")

def _impl(ctx):
    ctx.hooks.post_task(_post)
    ctx.defer(ctx.std.fs.try_append, "{path}", "defer\n")
{body}

t = task(implementation = _impl)
"#,
            path = trace.path,
            body = body
                .lines()
                .map(|l| format!("    {l}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let result = crate::test::eval(&code)
            .with_loader()
            .with_signals(signals.clone())
            .run_task(0);
        (result, trace.read(), signals)
    }

    const SOON: Duration = Duration::from_millis(200);
    const DEADLINE: Option<Duration> = Some(Duration::from_millis(300));

    #[test]
    fn a_loop_that_ignores_the_cancel_is_ended_at_the_deadline() {
        let (result, trace, _) = run_signalled(
            "for _tick in sleep_iter(20):\n    pass\nreturn 0",
            Kind::Interrupt,
            SOON,
            DEADLINE,
        );
        assert_eq!(result.expect("run_task"), Some(130));
        assert_eq!(trace, "post:130\ndefer\n");
    }

    #[test]
    fn a_loop_that_watches_the_root_ends_on_its_own_terms() {
        let (result, trace, signals) = run_signalled(
            r#"for _tick in sleep_iter(20):
    if ctx.cancellation.root.cancelled:
        return 7
return 1"#,
            Kind::Interrupt,
            SOON,
            None,
        );
        assert_eq!(result.expect("run_task"), Some(7));
        assert_eq!(trace, "post:7\ndefer\n");
        assert!(signals.root().is_cancelled());
    }

    #[test]
    fn a_terminate_at_the_deadline_reports_143() {
        let (result, trace, _) =
            run_signalled("sleep(30000)\nreturn 0", Kind::Terminate, SOON, DEADLINE);
        assert_eq!(result.expect("run_task"), Some(143));
        assert_eq!(trace, "post:143\ndefer\n");
    }

    /// The signal stops the child; the wait returns; the body goes on and
    /// decides. No deadline is needed.
    #[cfg(unix)]
    #[test]
    fn a_wait_returns_as_the_child_is_stopped_and_the_body_goes_on() {
        let (result, trace, _) = run_signalled(
            r#"c = ctx.std.process.command("sleep").arg("30").spawn()
status = c.wait()
return 3 if status.signal == 2 else 4"#,
            Kind::Interrupt,
            SOON,
            None,
        );
        assert_eq!(
            result.expect("run_task"),
            Some(3),
            "the child got SIGINT first"
        );
        assert_eq!(trace, "post:3\ndefer\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_long_sleeping_shell_and_its_sleeper_are_gone_once_the_task_ended() {
        let pidfile = Trace::new();
        let (result, _, _) = run_signalled(
            &format!(
                r#"c = ctx.std.process.command("bash").args(["-c", "sleep 600; echo never"]).process_group(0).spawn()
ctx.std.fs.try_append("{}", str(c.id))
c.wait()
return 0"#,
                pidfile.path
            ),
            Kind::Interrupt,
            SOON,
            None,
        );
        assert_eq!(result.expect("run_task"), Some(0));
        let pid: u32 = pidfile.read().trim().parse().expect("pid");
        assert!(
            crate::engine::children::os::has_exited(pid),
            "bash outlived the task"
        );
        assert!(
            !crate::engine::children::os::is_running(crate::engine::children::Target::Group(
                pid as i32
            )),
            "a member of bash's process group outlived the task"
        );
    }

    /// A process started by cleanup after the cancel is cleanup, not part of
    /// what was cancelled: it runs to completion.
    #[cfg(unix)]
    #[test]
    fn a_process_spawned_by_a_defer_after_the_signal_is_not_cancelled() {
        let out = Trace::new();
        let (result, trace, _) = run_signalled(
            &format!(
                r#"def _cleanup():
    status = ctx.std.process.command("sh").args(["-c", "sleep 0.3; echo done"]).spawn().wait()
    ctx.std.fs.try_append("{out}", "cleanup:" + str(status.code) + ":" + str(status.signal) + "\n")
ctx.defer(_cleanup)
for _tick in sleep_iter(20):
    pass"#,
                out = out.path
            ),
            Kind::Interrupt,
            SOON,
            DEADLINE,
        );
        assert_eq!(result.expect("run_task"), Some(130));
        assert_eq!(trace, "post:130\ndefer\n");
        assert_eq!(out.read(), "cleanup:0:None\n");
    }

    /// After `wait()` the pid may belong to someone else: a signal goes nowhere.
    #[cfg(unix)]
    #[test]
    fn a_signal_after_the_child_was_reaped_is_a_no_op() {
        let result = crate::test::eval(
            r#"
def _impl(ctx):
    c = ctx.std.process.command("true").spawn()
    c.wait()
    c.interrupt()
    c.terminate()
    c.kill()
    return 0

t = task(implementation = _impl)
"#,
        )
        .run_task(0);
        assert_eq!(result.expect("run_task"), Some(0));
    }

    #[test]
    fn a_pure_loop_is_ended_at_the_deadline() {
        let (result, trace, _) = run_signalled(
            "n = 0\nfor _ in range(1000000000):\n    n += 1\nreturn 0",
            Kind::Interrupt,
            SOON,
            DEADLINE,
        );
        assert_eq!(result.expect("run_task"), Some(130));
        assert_eq!(trace, "post:130\ndefer\n");
    }

    #[test]
    fn a_post_hook_can_still_block_after_the_deadline() {
        let (result, trace, _) = run_signalled(
            r#"ctx.hooks.post_task(lambda c, o: sleep(100))
for _tick in sleep_iter(20):
    pass"#,
            Kind::Interrupt,
            SOON,
            DEADLINE,
        );
        assert_eq!(result.expect("run_task"), Some(130));
        assert_eq!(trace, "post:130\ndefer\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_child_token_stops_only_what_is_bound_to_it() {
        let result = crate::test::eval(
            r#"
def _impl(ctx):
    sub = ctx.cancellation.root.child()
    c = ctx.std.process.command("sleep").arg("30").spawn(cancellation = sub)
    sub.cancel()
    status = c.wait()
    if status.signal != 2:
        return 2
    if ctx.cancellation.root.cancelled:
        return 3
    return 0

t = task(implementation = _impl)
"#,
        )
        .run_task(0);
        assert_eq!(result.expect("run_task"), Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn a_handle_token_stops_that_child_alone() {
        let result = crate::test::eval(
            r#"
def _impl(ctx):
    a = ctx.std.process.command("sleep").arg("30").spawn()
    b = ctx.std.process.command("sleep").arg("30").spawn()
    a.cancellation.cancel()
    if a.wait().signal != 2:
        return 2
    if b.try_wait() != None:
        return 3
    b.kill()
    b.wait()
    return 0

t = task(implementation = _impl)
"#,
        )
        .run_task(0);
        assert_eq!(result.expect("run_task"), Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn a_child_bound_to_a_new_token_survives_the_cancel() {
        let pidfile = Trace::new();
        let (result, _, _) = run_signalled(
            &format!(
                r#"tok = ctx.cancellation.new()
c = ctx.std.process.command("sleep").arg("30").spawn(cancellation = tok)
ctx.std.fs.try_append("{}", str(c.id))
for _tick in sleep_iter(20):
    if ctx.cancellation.root.cancelled:
        return 0"#,
                pidfile.path
            ),
            Kind::Interrupt,
            SOON,
            None,
        );
        assert_eq!(result.expect("run_task"), Some(0));
        let pid: i32 = pidfile.read().trim().parse().expect("pid");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            unsafe { nix::libc::kill(pid, 0) },
            0,
            "the detached child was stopped"
        );
        unsafe { nix::libc::kill(pid, nix::libc::SIGKILL) };
    }

    #[test]
    fn cancelled_evaluation_maps_to_the_exit_only_once_forced() {
        let signals = Signals::new();
        let err = starlark::Error::new_other(anyhow::anyhow!("Evaluation cancelled"));
        signals.record(Kind::Terminate, Origin::Process);
        assert!(!is_cancelled_error(&err, &signals), "not forced yet");
        signals.force();
        assert!(is_cancelled_error(&err, &signals));
        assert_eq!(
            TaskExit::from_starlark(&err, &signals),
            Some(TaskExit::new(143, Some("terminated".into())))
        );
    }
}
