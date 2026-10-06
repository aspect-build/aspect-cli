//! Cancellation: how an OS signal, or AXL itself, stops a run.
//!
//! Cancellation is a tree of tokens (tokio-util's [`CancellationToken`]).
//! The run has one root, [`Signals::root`], which the OS cancels when the
//! process receives Ctrl+C or SIGTERM, and which AXL reaches as
//! `ctx.cancellation.root`. Every process the runtime spawns is bound to a
//! token at its spawn (`engine/children.rs`) and is stopped when that token
//! is cancelled. Every blocking builtin runs under [`Signals::block`], so a
//! task that never thinks about signals ends at its next blocking call the way
//! `ctx.std.process.exit(130, "interrupted")` would: post-task hooks,
//! `ctx.defer` callbacks and the bookend still run.
//!
//! A task that wants to own the ending calls `ctx.cancellation.notify()`
//! once. From then on the OS cancels the token `notify` returned instead of
//! the root, nothing stops by itself, and the task reads `sig.cancelled` on
//! its own tick. Nothing in the runtime ever calls back into AXL on a signal.
//!
//! One [`Signals`] lives for the whole process. The test harness installs a
//! private one per run with [`Signals::scoped`], so a test can raise a signal
//! without touching its neighbours. Builtins reach it through
//! [`Signals::current`], which needs no evaluator: attribute getters cannot
//! take one, and iterators have none.

use std::cell::RefCell;
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

/// The run's cancellation state: the root token, what the OS asked for, and
/// the children whose stop sequences are in flight.
pub struct Signals {
    root: CancellationToken,
    /// The token `ctx.cancellation.notify()` handed out, if it was called.
    /// While set, the OS cancels this instead of `root`.
    notify: OnceLock<CancellationToken>,
    kind: OnceLock<Kind>,
    count: AtomicU32,
    /// Set once the task body has ended and the post-task hooks and defers
    /// are running: blocking builtins block normally again so cleanup can
    /// wait on things.
    unwinding: AtomicBool,
    stops: Mutex<Vec<Arc<Stop>>>,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Signals>>> = const { RefCell::new(None) };
}

impl Signals {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            root: CancellationToken::new(),
            notify: OnceLock::new(),
            kind: OnceLock::new(),
            count: AtomicU32::new(0),
            unwinding: AtomicBool::new(false),
            stops: Mutex::new(Vec::new()),
        })
    }

    /// The process-wide instance, the one the binary's signal task records into.
    pub fn global() -> &'static Arc<Signals> {
        static GLOBAL: OnceLock<Arc<Signals>> = OnceLock::new();
        GLOBAL.get_or_init(Signals::new)
    }

    /// The instance this thread's run uses: the one [`Signals::scoped`]
    /// installed, else the global one.
    pub fn current() -> Arc<Signals> {
        CURRENT
            .with(|c| c.borrow().clone())
            .unwrap_or_else(|| Signals::global().clone())
    }

    /// Make `signals` this thread's [`Signals::current`] until the returned
    /// guard is dropped. For the test harness, so each run has a root of its
    /// own.
    pub fn enter(signals: Arc<Signals>) -> Scope {
        Scope(CURRENT.with(|c| c.replace(Some(signals))))
    }

    /// Run `f` with `signals` as this thread's [`Signals::current`].
    pub fn scoped<R>(signals: Arc<Signals>, f: impl FnOnce() -> R) -> R {
        let _scope = Signals::enter(signals);
        f()
    }

    /// The run's root token.
    pub fn root(&self) -> &CancellationToken {
        &self.root
    }

    /// The OS asked the process to stop. Cancels the `notify` token when a
    /// task took one, else the root. Returns how many requests have arrived,
    /// so the caller can tell a repeat from the first.
    pub fn record(&self, kind: Kind) -> u32 {
        let _ = self.kind.set(kind);
        let count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        match self.notify.get() {
            Some(token) => token.cancel(),
            None => self.root.cancel(),
        }
        count
    }

    /// `ctx.cancellation.notify()`: the token the OS cancels from now on.
    /// The same token on every call.
    pub fn notify(&self) -> CancellationToken {
        self.notify.get_or_init(CancellationToken::new).clone()
    }

    /// What the OS asked for, if it did.
    pub fn kind(&self) -> Option<Kind> {
        self.kind.get().copied()
    }

    /// The task body has ended; blocking builtins block normally from here.
    pub fn enter_unwind(&self) {
        self.unwinding.store(true, Ordering::SeqCst);
    }

    pub fn unwinding(&self) -> bool {
        self.unwinding.load(Ordering::SeqCst)
    }

    /// Whether the body should end now: the root is cancelled and the
    /// post-task hooks have not started yet. The starlark bytecode check and
    /// every blocking builtin ask this.
    pub fn should_unwind(&self) -> bool {
        self.root.is_cancelled() && !self.unwinding()
    }

    /// The exit the default ending reports: 130 or 143 after a signal, 130
    /// when AXL cancelled the root itself.
    pub fn exit_for(&self) -> TaskExit {
        match self.kind() {
            Some(kind) => TaskExit::new(kind.exit_code(), Some(kind.message().to_owned())),
            None => TaskExit::new(Kind::Interrupt.exit_code(), Some("cancelled".to_owned())),
        }
    }

    /// Drive `fut` to completion on the runtime, unless the root is cancelled
    /// first, in which case the exit the task should end with is returned
    /// instead. While unwinding, `fut` runs to completion regardless. Every
    /// blocking builtin goes through here.
    pub fn block<F: std::future::Future>(&self, fut: F) -> Result<F::Output, TaskExit> {
        let handle = tokio::runtime::Handle::current();
        if self.unwinding() {
            return Ok(handle.block_on(fut));
        }
        match handle.block_on(self.root.run_until_cancelled(fut)) {
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
            .map(|stops| stops.iter().filter(|s| !s.exited()).cloned().collect())
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

/// What [`Signals::enter`] returns: dropping it restores the previous
/// [`Signals::current`].
pub struct Scope(Option<Arc<Signals>>);

impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.0.take();
        CURRENT.with(|c| *c.borrow_mut() = previous);
    }
}

/// `"Evaluation cancelled"` is what the evaluator raises when the bytecode
/// cancel check fires. Its error type is private to starlark, so the text is
/// matched, and only once the root really is cancelled: nothing else makes
/// the check return true.
pub fn is_cancelled_error(err: &starlark::Error) -> bool {
    match err.kind() {
        starlark::ErrorKind::Other(inner) => {
            inner.to_string() == "Evaluation cancelled" && Signals::current().root().is_cancelled()
        }
        _ => false,
    }
}

/// A cancellation token: `ctx.cancellation.root`, what `ctx.cancellation.new()`
/// and `notify()` return, a token's `child()`, and `handle.cancellation` on
/// every process the runtime spawned.
///
/// Cancelling a token cancels every token derived from it with `child()`,
/// never its parent. A process bound to a token (`cancellation = tok` at its
/// spawn) is stopped when the token is cancelled, the way its kind needs: a
/// `std.process.Child` gets a terminate signal and, three seconds later, a
/// kill; a `bazel.Build` gets one interrupt, bazel's own graceful cancel.
///
/// Libraries take a token as a plain argument and default it to
/// `ctx.cancellation.root`; their loops check `cancellation.cancelled`. Only
/// a task decides how the run ends (`ctx.cancellation.notify()`).
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<cancellation.Token>")]
pub struct Token {
    #[allocative(skip)]
    inner: CancellationToken,
}

starlark_simple_value!(Token);

impl Token {
    pub fn new(inner: CancellationToken) -> Self {
        Self { inner }
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
        Ok(Token::new(token.inner.child_token()))
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
    /// `True` when it was cancelled, `False` on the timeout. Like every
    /// blocking call, it ends the task instead if the root is cancelled while
    /// it waits and no `notify()` is in effect.
    fn wait<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<i32>,
    ) -> anyhow::Result<bool> {
        let token = this
            .downcast_ref_err::<Token>()
            .into_anyhow_result()?
            .inner
            .clone();
        let timeout = match timeout_ms.into_option() {
            Some(ms) if ms < 0 => anyhow::bail!("timeout_ms must not be negative: {ms}"),
            Some(ms) => Some(Duration::from_millis(ms as u64)),
            None => None,
        };
        Ok(Signals::current().block(async move {
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
///   you, and is what every spawn and every blocking call is bound to unless
///   told otherwise.
/// - `new()` is a token nothing cancels but you.
/// - `notify()` hands the OS request to the task instead of the root.
///
/// ```python
/// def _impl(ctx):
///     sig = ctx.cancellation.notify()
///     sub = ctx.cancellation.root.child()
///     child = ctx.std.process.command("dev-server", cancellation = sub).spawn()
///     for _tick in sleep_iter(100):
///         if sig.cancelled:
///             sub.cancel()
///             child.wait()
///             return 0
/// ```
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<cancellation.Cancellation>")]
pub struct Cancellation {}

starlark_simple_value!(Cancellation);

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
    /// SIGTERM, unless `notify()` was called; `root.cancel()` does the same
    /// from AXL. Every process spawned without an explicit `cancellation =`
    /// is bound to it, and every blocking call ends the task once it is
    /// cancelled, as `ctx.std.process.exit(130, "interrupted")` would: the
    /// post-task hooks, `ctx.defer` callbacks and the bookend still run.
    #[starlark(attribute)]
    fn root<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Token> {
        Ok(Token::new(Signals::current().root().clone()))
    }

    /// A token nothing cancels but you. Bind a process to it to keep it out
    /// of Ctrl+C and stop it yourself, with `cancel()` on the token or
    /// `interrupt()`, `terminate()` and `kill()` on the handle.
    fn new<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Token> {
        Ok(Token::new(CancellationToken::new()))
    }

    /// Own the ending. Returns a token the operating system cancels instead
    /// of `root` from now on, so nothing stops by itself when Ctrl+C arrives:
    /// the task reads `sig.cancelled` on its tick, stops what it owns, and
    /// returns the code it wants. For a task, never a library; calling it
    /// again returns the same token.
    fn notify<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Token> {
        Ok(Token::new(Signals::current().notify()))
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
    fn a_signal_cancels_the_root_unless_notified() {
        let s = Signals::new();
        assert!(!s.root().is_cancelled());
        assert_eq!(s.record(Kind::Interrupt), 1);
        assert!(s.root().is_cancelled());
        assert!(s.should_unwind());
        assert_eq!(s.exit_for(), TaskExit::new(130, Some("interrupted".into())));

        let s = Signals::new();
        let sig = s.notify();
        assert_eq!(s.record(Kind::Terminate), 1);
        assert!(sig.is_cancelled());
        assert!(!s.root().is_cancelled());
        assert_eq!(s.record(Kind::Terminate), 2);
    }

    #[test]
    fn unwinding_turns_the_check_off() {
        let s = Signals::new();
        s.record(Kind::Terminate);
        s.enter_unwind();
        assert!(!s.should_unwind());
        assert!(s.root().is_cancelled());
    }

    #[test]
    fn block_returns_the_exit_once_the_root_is_cancelled() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        let s = Signals::new();
        assert_eq!(s.block(async { 7 }), Ok(7));
        s.record(Kind::Interrupt);
        assert_eq!(
            s.block(std::future::pending::<()>()),
            Err(TaskExit::new(130, Some("interrupted".into())))
        );
        s.enter_unwind();
        assert_eq!(s.block(async { 8 }), Ok(8));
    }

    #[test]
    fn scoped_isolates_the_current_instance() {
        let s = Signals::new();
        let inner = Signals::scoped(s.clone(), || Signals::current());
        assert!(Arc::ptr_eq(&inner, &s));
        assert!(Arc::ptr_eq(&Signals::current(), Signals::global()));
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
    /// append a line, under a private `Signals`, and `record(kind)` from
    /// another thread after `after`.
    fn run_signalled(
        body: &str,
        kind: Kind,
        after: Duration,
    ) -> (anyhow::Result<Option<u8>>, String, Arc<Signals>) {
        let trace = Trace::new();
        let signals = Signals::new();
        let raiser = signals.clone();
        std::thread::spawn(move || {
            std::thread::sleep(after);
            raiser.record(kind);
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

    #[test]
    fn a_signal_during_a_tick_loop_ends_the_task_like_an_exit() {
        let (result, trace, _) = run_signalled(
            "for _tick in sleep_iter(20):\n    pass\nreturn 0",
            Kind::Interrupt,
            SOON,
        );
        assert_eq!(result.expect("run_task"), Some(130));
        assert_eq!(trace, "post:130\ndefer\n");
    }

    #[test]
    fn a_terminate_reports_143() {
        let (result, trace, _) = run_signalled("sleep(30000)\nreturn 0", Kind::Terminate, SOON);
        assert_eq!(result.expect("run_task"), Some(143));
        assert_eq!(trace, "post:143\ndefer\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_signal_during_a_child_wait_ends_the_task_and_the_child() {
        let pidfile = Trace::new();
        let (result, trace, _) = run_signalled(
            &format!(
                r#"c = ctx.std.process.command("sleep").arg("30").spawn()
ctx.std.fs.try_append("{}", str(c.id))
c.wait()
return 0"#,
                pidfile.path
            ),
            Kind::Interrupt,
            SOON,
        );
        assert_eq!(result.expect("run_task"), Some(130));
        assert_eq!(trace, "post:130\ndefer\n");
        // The child is ours and nobody reaps it once the task is gone, so
        // ask the kernel whether it has exited rather than whether it exists.
        let pid: u32 = pidfile.read().trim().parse().expect("pid");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !crate::engine::children::os::has_exited(pid) && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            crate::engine::children::os::has_exited(pid),
            "the child outlived the task"
        );
    }

    #[test]
    fn a_signal_during_a_pure_loop_ends_the_task() {
        let (result, trace, _) = run_signalled(
            "n = 0\nfor _ in range(1000000000):\n    n += 1\nreturn 0",
            Kind::Interrupt,
            SOON,
        );
        assert_eq!(result.expect("run_task"), Some(130));
        assert_eq!(trace, "post:130\ndefer\n");
    }

    #[test]
    fn a_body_that_returns_after_the_signal_still_ends_with_130() {
        // `sleep_iter` ends on the signal and the body goes on to `return 0`;
        // the evaluator's check after the call turns that into the exit.
        let (result, trace, _) = run_signalled(
            "for _tick in sleep_iter(20):\n    pass\nreturn 0",
            Kind::Interrupt,
            SOON,
        );
        assert_eq!(result.expect("run_task"), Some(130));
        assert!(trace.starts_with("post:130\n"), "{trace}");
    }

    #[test]
    fn a_notified_task_sees_the_signal_and_owns_the_ending() {
        let (result, trace, signals) = run_signalled(
            r#"sig = ctx.cancellation.notify()
for _tick in sleep_iter(20):
    if sig.cancelled:
        return 7
return 1"#,
            Kind::Interrupt,
            SOON,
        );
        assert_eq!(result.expect("run_task"), Some(7));
        assert_eq!(trace, "post:7\ndefer\n");
        assert!(!signals.root().is_cancelled());
    }

    #[test]
    fn a_post_hook_can_still_block_after_the_signal() {
        let (result, trace, _) = run_signalled(
            r#"ctx.hooks.post_task(lambda c, o: sleep(100))
for _tick in sleep_iter(20):
    pass"#,
            Kind::Interrupt,
            SOON,
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
    if status.signal != 15:
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
    if a.wait().signal != 15:
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
    fn a_child_bound_to_a_new_token_survives_the_default_ending() {
        let pidfile = Trace::new();
        let (result, _, _) = run_signalled(
            &format!(
                r#"tok = ctx.cancellation.new()
c = ctx.std.process.command("sleep").arg("30").spawn(cancellation = tok)
ctx.std.fs.try_append("{}", str(c.id))
for _tick in sleep_iter(20):
    pass"#,
                pidfile.path
            ),
            Kind::Interrupt,
            SOON,
        );
        assert_eq!(result.expect("run_task"), Some(130));
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
    fn cancelled_evaluation_maps_to_the_exit() {
        let signals = Signals::new();
        let err = starlark::Error::new_other(anyhow::anyhow!("Evaluation cancelled"));
        Signals::scoped(signals.clone(), || {
            assert!(!is_cancelled_error(&err), "not cancelled until the root is");
            signals.record(Kind::Terminate);
            assert!(is_cancelled_error(&err));
            assert_eq!(
                TaskExit::from_starlark(&err),
                Some(TaskExit::new(143, Some("terminated".into())))
            );
        });
    }
}
