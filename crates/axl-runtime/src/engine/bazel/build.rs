use crate::errln;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use allocative::Allocative;
use axl_types::stream::Writable;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;

use starlark::StarlarkResultExt;
use starlark::starlark_module;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;

use crate::engine::history::History;
use crate::engine::task_record::BazelSlot;
use axl_proto::build_event_stream::BuildEvent;
use bazel::stream::Gate;
use bazel::stream::file::Reservation;
use bazel::stream::pid_pair::PidPair;

use crate::engine::r#async::rt::AsyncRuntime;
use crate::engine::cancellation::Signals;
use crate::engine::children::{self, Bound, Recv, recv_cancellable};
use crate::engine::telemetry::TelemetrySources;
use tokio_util::sync::CancellationToken;

use super::iter::ExecLogIter;
use super::iter::Handle;
use super::iter::WorkspaceEventIter;
use super::output::TraitConsole;
use super::wrappers::ExecLogSink;
use bazel::bes::BuildEventEnvelope;
use bazel::bes::feed::{BuildEventStream, FileSignal, FileSink};
use bazel::bes::kind::{self, event_kind_help, event_kind_in};
use bazel::bes::sink::grpc;
use bazel::bes::sink::retry::{RetryConfig, SinkStats};
use bazel::bes::sink::tracing as tracing_sink;
use bazel::console::forward::Forwarder;
use bazel::console::processors::{HeldLine, Observations};
use bazel::execlog::feed::{ExecLogStream, ReaderOutcome};
use bazel::fanout::SinkOutcome;
use bazel::fanout::Subscriber;
use bazel::fanout::SubscriberFilter;
use bazel::runner::Runner as _;
use bazel::workspace_event::feed::WorkspaceEventStream;
use fibre::spmc::Receiver;

/// Subscribe each iterator handle to a subscriber cloned from `receiver`, all
/// or nothing.
///
/// A bound handle is a subscriber the producer blocks for, so one left behind
/// by a half-finished bind would park the reader on a consumer that is never
/// coming. A handle passed twice, or still bound to an earlier build, fails
/// its bind; the handles this call bound before it are released again, and
/// only those: the failing one may belong to a build that is still running.
fn bind_handles<T: Send + Clone, H: Handle<T>>(
    handles: &[H],
    receiver: impl Fn() -> Option<Receiver<T>>,
    log: &str,
) -> io::Result<()> {
    for (bound, handle) in handles.iter().enumerate() {
        let bind = || -> io::Result<()> {
            handle
                .bind(subscriber(receiver(), log)?)
                .map_err(io::Error::other)
        };
        if let Err(err) = bind() {
            handles[..bound].iter().for_each(Handle::release);
            return Err(err);
        }
    }
    Ok(())
}

/// A further subscriber to the `log` stream, or an error once it is sealed.
fn subscriber<T: Send + Clone>(
    receiver: Option<Receiver<T>>,
    log: &str,
) -> io::Result<Receiver<T>> {
    receiver.ok_or_else(|| io::Error::other(format!("{log} stream has no subscriber to clone")))
}

/// Convert a Starlark `Writable` handle to a `std::process::Stdio` for use
/// as a child's stdio slot.
///
/// Parent stdio handles (`Writable::Stdout`/`Stderr`/`ChildStdin`) get their
/// underlying fd duplicated so cross-wiring (e.g. `stdout = ctx.std.io.stderr`)
/// works and the original handle stays usable from Starlark. `Writable::File`
/// is `try_clone`d for the same reason.
pub fn writable_to_stdio(w: &Writable) -> io::Result<Stdio> {
    let closed = || io::Error::other("writable stream is closed");
    match w {
        Writable::Stdout(arc, _) => {
            let guard = arc.lock().unwrap();
            let borrowed = guard.borrow();
            let s = borrowed.as_ref().ok_or_else(closed)?;
            dup_fd(s)
        }
        Writable::Stderr(arc, _) => {
            let guard = arc.lock().unwrap();
            let borrowed = guard.borrow();
            let s = borrowed.as_ref().ok_or_else(closed)?;
            dup_fd(s)
        }
        Writable::ChildStdin(arc) => {
            let guard = arc.lock().unwrap();
            let borrowed = guard.borrow();
            let s = borrowed.as_ref().ok_or_else(closed)?;
            dup_fd(s)
        }
        Writable::File(arc) => {
            let guard = arc.lock().unwrap();
            let file = guard.as_ref().ok_or_else(closed)?;
            Ok(Stdio::from(file.try_clone()?))
        }
    }
}

#[cfg(unix)]
fn dup_fd<H: std::os::fd::AsFd>(h: &H) -> io::Result<Stdio> {
    Ok(Stdio::from(h.as_fd().try_clone_to_owned()?))
}

#[cfg(windows)]
fn dup_fd<H: std::os::windows::io::AsHandle>(h: &H) -> io::Result<Stdio> {
    Ok(Stdio::from(h.as_handle().try_clone_to_owned()?))
}

#[derive(Debug, Clone, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.BuildStatus>")]
pub struct BuildStatus {
    success: bool,
    code: Option<i32>,
    signal: Option<i32>,
}

impl BuildStatus {
    /// The client's exit: its code, or, when a signal ended it, which.
    fn of(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal = None;
        BuildStatus {
            success: status.success(),
            code: status.code(),
            signal,
        }
    }
}

impl<'v> AllocValue<'v> for BuildStatus {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "bazel.build.BuildStatus")]
impl<'v> values::StarlarkValue<'v> for BuildStatus {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("build_status_methods", build_status_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
pub(crate) fn build_status_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn success<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        Ok(this.downcast_ref::<BuildStatus>().unwrap().success)
    }
    /// The client's exit code; None when a signal ended it (see `signal`).
    #[starlark(attribute)]
    fn code<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<BuildStatus>().unwrap().code,
        ))
    }

    /// The signal that ended the client, such as 9 when it was SIGKILLed;
    /// None when it exited with a code.
    #[starlark(attribute)]
    fn signal<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<BuildStatus>().unwrap().signal,
        ))
    }
}

#[derive(Clone)]
enum SinkConfig {
    Grpc {
        uri: String,
        metadata: HashMap<String, String>,
        retry: RetryConfig,
        /// The largest message the backend accepts; None for
        /// [`bazel::bes::cap::DEFAULT_MAX_MESSAGE_BYTES`].
        max_message_bytes: Option<usize>,
    },
    File {
        path: String,
    },
}

/// Handles cycle Idle → Live → Idle across multiple `ctx.bazel.build(...)`
/// calls so the retry loop in `bazel_runner.axl` can reuse them.
enum SinkPhase {
    Idle,
    Live(SinkLive),
}

enum SinkLive {
    Grpc {
        join: JoinHandle<(SinkStats, SinkOutcome)>,
    },
    File {
        signal: Arc<FileSignal>,
    },
}

#[derive(Debug, Default)]
struct SinkOutcomeState {
    failed: bool,
    error: Option<String>,
    /// `times_bound > 0` distinguishes "never bound" from "freshly bound
    /// but already waited" — needed by the `done` attribute.
    times_bound: usize,
    /// Distinct build events streamed to the backend on the most recent bind
    /// (gRPC sinks only; 0 for file sinks). See `SinkStats`.
    events_sent: u64,
    /// Build events the backend confirmed on the most recent bind (gRPC sinks
    /// only; 0 for file sinks). See `SinkStats`.
    events_acked: u64,
}

#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.BuildEventSink>")]
pub struct BuildEventSink {
    #[allocative(skip)]
    config: Arc<SinkConfig>,
    #[allocative(skip)]
    phase: Arc<Mutex<SinkPhase>>,
    #[allocative(skip)]
    outcome: Arc<Mutex<SinkOutcomeState>>,
}

impl std::fmt::Debug for SinkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SinkConfig::Grpc { uri, .. } => f.debug_struct("Grpc").field("uri", uri).finish(),
            SinkConfig::File { path } => f.debug_struct("File").field("path", path).finish(),
        }
    }
}

impl std::fmt::Debug for SinkPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SinkPhase::Idle => write!(f, "Idle"),
            SinkPhase::Live(_) => write!(f, "Live"),
        }
    }
}

impl BuildEventSink {
    pub fn new_grpc(
        uri: String,
        metadata: HashMap<String, String>,
        retry: RetryConfig,
        max_message_bytes: Option<usize>,
    ) -> Self {
        Self::new(SinkConfig::Grpc {
            uri,
            metadata,
            retry,
            max_message_bytes,
        })
    }

    pub fn new_file(path: String) -> Self {
        Self::new(SinkConfig::File { path })
    }

    fn new(config: SinkConfig) -> Self {
        Self {
            config: Arc::new(config),
            phase: Arc::new(Mutex::new(SinkPhase::Idle)),
            outcome: Arc::new(Mutex::new(SinkOutcomeState::default())),
        }
    }

    fn file_path(&self) -> Option<String> {
        match &*self.config {
            SinkConfig::File { path } => Some(path.clone()),
            _ => None,
        }
    }

    /// The largest message a gRPC sink's backend accepts, None for a file
    /// sink.
    fn grpc_max_message_bytes(&self) -> Option<usize> {
        match &*self.config {
            SinkConfig::Grpc {
                max_message_bytes, ..
            } => Some(max_message_bytes.unwrap_or(bazel::bes::cap::DEFAULT_MAX_MESSAGE_BYTES)),
            _ => None,
        }
    }

    fn grpc_uri(&self) -> Option<String> {
        match &*self.config {
            SinkConfig::Grpc { uri, .. } => Some(uri.clone()),
            _ => None,
        }
    }

    fn bind_grpc(
        &self,
        rt: AsyncRuntime,
        stream: &BuildEventStream,
        invocation_id: String,
        trim: bool,
    ) -> anyhow::Result<()> {
        let SinkConfig::Grpc {
            uri,
            metadata,
            retry,
            ..
        } = &*self.config
        else {
            anyhow::bail!("BUG: bind_grpc called on a non-gRPC sink");
        };
        let mut phase = self.phase.lock().unwrap();
        if matches!(*phase, SinkPhase::Live(_)) {
            anyhow::bail!(
                "this `bazel.build_events.grpc(...)` handle is still Live from a previous bind; \
                 call `sink.wait()` before passing it to another `ctx.bazel.build(...)` call",
            );
        }
        let mut outcome = self.outcome.lock().unwrap();
        outcome.failed = false;
        outcome.error = None;
        outcome.times_bound += 1;
        drop(outcome);
        let join = grpc::Grpc::spawn(
            rt.0.clone(),
            stream.subscribe(),
            uri.clone(),
            metadata.clone(),
            invocation_id,
            retry.clone(),
            trim,
        );
        *phase = SinkPhase::Live(SinkLive::Grpc { join });
        Ok(())
    }

    fn bind_file(&self) -> anyhow::Result<Arc<FileSignal>> {
        if !matches!(&*self.config, SinkConfig::File { .. }) {
            anyhow::bail!("BUG: bind_file called on a non-file sink");
        }
        let mut phase = self.phase.lock().unwrap();
        if matches!(*phase, SinkPhase::Live(_)) {
            anyhow::bail!(
                "this `bazel.build_events.file(...)` handle is still Live from a previous bind; \
                 call `sink.wait()` before re-binding",
            );
        }
        let mut outcome = self.outcome.lock().unwrap();
        outcome.failed = false;
        outcome.error = None;
        outcome.times_bound += 1;
        drop(outcome);
        let signal = Arc::new(FileSignal::new());
        *phase = SinkPhase::Live(SinkLive::File {
            signal: signal.clone(),
        });
        Ok(signal)
    }
}

impl<'v> AllocValue<'v> for BuildEventSink {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for BuildEventSink {
    type Error = anyhow::Error;

    // `Ok(None)` (not `Err`) on type mismatch so Either's UnpackValue can
    // fall through to the next branch.
    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<BuildEventSink>().cloned())
    }
}

#[starlark_value(type = "bazel.build.BuildEventSink")]
impl<'v> values::StarlarkValue<'v> for BuildEventSink {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("build_event_sink_methods", build_event_sink_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let phase = self.phase.lock().unwrap();
        let outcome = self.outcome.lock().unwrap();
        match attribute {
            "done" => {
                Some(heap.alloc(matches!(*phase, SinkPhase::Idle) && outcome.times_bound > 0))
            }
            "failed" => Some(heap.alloc(outcome.failed)),
            "error" => Some(match &outcome.error {
                Some(e) => heap.alloc_str(e).to_value(),
                None => Value::new_none(),
            }),
            // Build events streamed to a gRPC backend and those the server acked
            // (its sequence-number acks are the only delivery confirmation), so a
            // caller can report how many events reached the backend. Both are 0
            // for a file sink or a stream that never bound. See `wait`.
            "events_sent" => Some(heap.alloc(outcome.events_sent)),
            "events_acked" => Some(heap.alloc(outcome.events_acked)),
            // The gRPC backend URI (None for a file sink), so a summary can name
            // the backend without the caller tracking it separately.
            "uri" => Some(match self.grpc_uri() {
                Some(uri) => heap.alloc_str(&uri).to_value(),
                None => Value::new_none(),
            }),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(
            attribute,
            "done" | "failed" | "error" | "events_sent" | "events_acked" | "uri"
        )
    }
}

#[starlark_module]
pub(crate) fn build_event_sink_methods(registry: &mut MethodsBuilder) {
    /// Block until this sink finishes flushing. Idempotent: `None` once it
    /// has nothing left to wait for.
    ///
    /// With `timeout_ms`, gives up after that long and returns `False`,
    /// leaving the sink flushing in the background for a later `wait()`.
    /// This is how AXL drains build events after a cancel: the forwarders
    /// get a bounded window to deliver bazel's `BuildFinished` before the
    /// process exits. Returns `True` when the sink finished within the wait.
    fn wait<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<i32>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneOr<bool>> {
        let sink = this
            .downcast_ref_err::<BuildEventSink>()
            .into_anyhow_result()?;
        let signals = crate::engine::store::Env::from_eval(eval)?.signals.clone();
        let timeout = match timeout_ms.into_option() {
            Some(ms) if ms < 0 => anyhow::bail!("timeout_ms must not be negative: {ms}"),
            Some(ms) => Some(Duration::from_millis(ms as u64)),
            None => None,
        };
        // Take Live out of the Mutex so we don't hold the lock across the
        // blocking join.
        let live = {
            let mut phase = sink.phase.lock().unwrap();
            match std::mem::replace(&mut *phase, SinkPhase::Idle) {
                SinkPhase::Live(live) => live,
                SinkPhase::Idle => return Ok(NoneOr::None),
            }
        };
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        let finished = |done: &dyn Fn() -> bool| -> Result<bool, crate::eval::TaskExit> {
            signals.block(async {
                loop {
                    if done() {
                        return true;
                    }
                    if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
        };
        let (outcome, stats): (Result<(), String>, SinkStats) = match live {
            SinkLive::Grpc { join } => {
                if !finished(&|| join.is_finished())? {
                    *sink.phase.lock().unwrap() = SinkPhase::Live(SinkLive::Grpc { join });
                    return Ok(NoneOr::Other(false));
                }
                match join.join() {
                    Ok((stats, Ok(()))) => (Ok(()), stats),
                    Ok((stats, Err(e))) => (Err(e.last_error), stats),
                    Err(_) => (
                        Err("sink worker thread panicked".to_string()),
                        SinkStats::default(),
                    ),
                }
            }
            SinkLive::File { signal } => {
                if !finished(&|| signal.is_complete())? {
                    *sink.phase.lock().unwrap() = SinkPhase::Live(SinkLive::File { signal });
                    return Ok(NoneOr::Other(false));
                }
                (signal.wait(), SinkStats::default())
            }
        };
        let (failed, error) = match outcome {
            Ok(()) => (false, None),
            Err(e) => (true, Some(e)),
        };
        let mut out = sink.outcome.lock().unwrap();
        out.failed = failed;
        out.error = error;
        out.events_sent = stats.sent;
        out.events_acked = stats.acked;
        Ok(NoneOr::Other(true))
    }
}

#[derive(Clone)]
struct IterConfig {
    /// `None` means no filter — every event yields. `Some(set)` keeps only
    /// events whose payload tag is in the set. Applied SEND-side at `bind`
    /// time (see `BuildEventStream::subscribe_filtered`), so a filtered
    /// iterator's buffer never holds events of other kinds.
    kinds: Option<Arc<HashSet<i32>>>,
    /// When `Some(ms)`, blocking iteration (`for event in iter`) yields a
    /// Starlark `None` after `ms` of silence instead of blocking forever, so
    /// callers get a heartbeat tick even while Bazel is quiet. `None` blocks
    /// until the next event or stream close (the historical behavior).
    tick_ms: Option<u64>,
}

enum IterState {
    /// Created but not yet bound to a build.
    Pending,
    /// `Build::spawn` subscribed us; iteration reads from `recv`.
    Live {
        recv: Subscriber<Arc<BuildEventEnvelope>>,
    },
    /// Stream ended (clean close or caller drained).
    Done,
}

#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.BuildEventIter>")]
pub struct BuildEventIter {
    #[allocative(skip)]
    config: IterConfig,
    #[allocative(skip)]
    state: Arc<Mutex<IterState>>,
    #[allocative(skip)]
    signals: Arc<Signals>,
}

impl std::fmt::Debug for IterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IterConfig")
            .field("has_kinds_filter", &self.kinds.is_some())
            .finish()
    }
}

impl std::fmt::Debug for IterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IterState::Pending => write!(f, "Pending"),
            IterState::Live { .. } => write!(f, "Live"),
            IterState::Done => write!(f, "Done"),
        }
    }
}

impl BuildEventIter {
    pub fn new(signals: Arc<Signals>, kinds: Option<HashSet<i32>>, tick_ms: Option<u64>) -> Self {
        Self {
            config: IterConfig {
                kinds: kinds.map(Arc::new),
                tick_ms,
            },
            state: Arc::new(Mutex::new(IterState::Pending)),
            signals,
        }
    }

    /// Take the bound receiver out of this handle, for a Rust consumer that
    /// wants the subscription `Build::spawn` made for it without going
    /// through Starlark iteration. The handle reads as `done` afterwards.
    /// `None` when the handle was never bound or was already taken.
    pub(super) fn take_recv(&self) -> Option<Subscriber<Arc<BuildEventEnvelope>>> {
        let mut state = self.state.lock().unwrap();
        match std::mem::replace(&mut *state, IterState::Done) {
            IterState::Live { recv } => Some(recv),
            other => {
                *state = other;
                None
            }
        }
    }

    /// Subscribe the iterator's receiver. Must run before the stream is
    /// started so the early burst is buffered. The `kinds=` filter is
    /// installed here as a send-side predicate, so the reader thread drops
    /// unwanted kinds before they ever enter this iterator's buffer.
    fn bind(&self, stream: &BuildEventStream) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap();
        match *state {
            IterState::Pending => {
                let filter: Option<SubscriberFilter<Arc<BuildEventEnvelope>>> =
                    self.config.kinds.clone().map(|kinds| {
                        let f: SubscriberFilter<Arc<BuildEventEnvelope>> =
                            Arc::new(move |envelope: &Arc<BuildEventEnvelope>| {
                                event_kind_in(&envelope.event, &kinds)
                            });
                        f
                    });
                let recv = stream.subscribe_filtered(filter);
                *state = IterState::Live { recv };
                Ok(())
            }
            _ => anyhow::bail!(
                "this `bazel.build_events.iterator()` handle was already bound to a build; \
                 create a fresh one per build",
            ),
        }
    }
}

impl<'v> AllocValue<'v> for BuildEventIter {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for BuildEventIter {
    type Error = anyhow::Error;
    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        let v = value
            .downcast_ref_err::<BuildEventIter>()
            .into_anyhow_result()?;
        Ok(Some(v.clone()))
    }
}

#[starlark_value(type = "bazel.build.BuildEventIter")]
impl<'v> values::StarlarkValue<'v> for BuildEventIter {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("build_event_iter_methods", build_event_iter_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "done" => {
                let state = self.state.lock().unwrap();
                Some(heap.alloc(matches!(*state, IterState::Done)))
            }
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "done")
    }

    unsafe fn iterate(&self, me: Value<'v>, _heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(me)
    }

    unsafe fn iter_next(&self, _i: usize, heap: Heap<'v>) -> Option<Value<'v>> {
        // Take recv out of the Mutex so we don't hold the lock across the
        // blocking call. Events are pre-filtered send-side (see `bind`), so
        // whatever arrives is already a kind the caller asked for.
        let recv = {
            let mut state = self.state.lock().unwrap();
            match std::mem::replace(&mut *state, IterState::Pending) {
                IterState::Live { recv } => recv,
                other => {
                    *state = other;
                    return None;
                }
            }
        };

        // Heartbeat mode yields a Starlark `None` when the stream goes quiet
        // for `tick_ms`, so the caller's loop keeps ticking; blocking mode
        // yields events until the stream closes. Neither ends on a cancel:
        // the loop decides when to leave. A cancelled root only stops the
        // heartbeat's wait, so the evaluator's own check ends a loop that is
        // not watching within a few ticks rather than after that many.
        let received = match self.config.tick_ms {
            Some(ms) => recv_cancellable(&self.signals, &*recv, Some(Duration::from_millis(ms))),
            None => match recv.recv() {
                Ok(envelope) => Recv::Item(envelope),
                Err(_) => Recv::Closed,
            },
        };
        match received {
            Recv::Item(envelope) => {
                *self.state.lock().unwrap() = IterState::Live { recv };
                Some(BuildEventEnvelope::into_event(envelope).alloc_value(heap))
            }
            Recv::Tick | Recv::Cancelled => {
                *self.state.lock().unwrap() = IterState::Live { recv };
                Some(Value::new_none())
            }
            Recv::Closed => {
                *self.state.lock().unwrap() = IterState::Done;
                None
            }
        }
    }

    unsafe fn iter_stop(&self) {}
}

#[starlark_module]
pub(crate) fn build_event_iter_methods(registry: &mut MethodsBuilder) {
    /// Stop iterating: unsubscribe, drop buffered events. Idempotent.
    fn drain<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
        let iter = this
            .downcast_ref_err::<BuildEventIter>()
            .into_anyhow_result()?;
        let mut state = iter.state.lock().unwrap();
        if !matches!(*state, IterState::Done) {
            *state = IterState::Done;
        }
        Ok(NoneOr::None)
    }

    /// Non-blocking pop. Returns `None` when empty or disconnected. Events are
    /// already `kinds=`-filtered send-side, so whatever is buffered is wanted.
    fn try_pop<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<BuildEvent>> {
        let iter = this
            .downcast_ref_err::<BuildEventIter>()
            .into_anyhow_result()?;
        let mut state = iter.state.lock().unwrap();
        let recv = match &*state {
            IterState::Live { recv } => recv,
            _ => return Ok(NoneOr::None),
        };
        match recv.try_recv() {
            Ok(envelope) => Ok(NoneOr::Other(BuildEventEnvelope::into_event(envelope))),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(NoneOr::None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *state = IterState::Done;
                Ok(NoneOr::None)
            }
        }
    }
}

/// Resolve one `kinds=` list element to a payload tag. Accepts the payload's
/// `type(event.payload)` name, any legacy alias, or the raw tag integer; see
/// [`kind::tag_from_int`] and [`kind::tag_from_name`] for what each is.
pub(super) fn parse_event_kind(value: values::Value) -> anyhow::Result<i32> {
    if let Some(n) = value.unpack_i32() {
        return kind::tag_from_int(n).map_err(anyhow::Error::msg);
    }
    if let Some(s) = value.unpack_str() {
        return kind::tag_from_name(s).map_err(anyhow::Error::msg);
    }
    // `unpack_i32` rejects an int too large for i32, which would otherwise reach
    // the wrong-type arm below and tell an int it is not a tag.
    if value.get_type() == "int" {
        anyhow::bail!(
            "build_event payload tag out of range: {value} does not fit a 32-bit \
             int; {}",
            event_kind_help()
        );
    }
    anyhow::bail!(
        "kinds entry must be a build event payload name or its payload tag; got \
         {}; {}",
        value.get_type(),
        event_kind_help(),
    )
}

/// Optionally print the detected Bazel version and/or the exact command being
/// spawned, one `INFO:` line each, before a `bazel build`/`test`/`run`
/// invocation. Gated by the `--announce-bazel-version` / `--announce-bazel-command`
/// task flags, resolved in AXL and passed through as `announce`.
///
/// Its `INFO` label is the CLI's own (`label.info`), its text `muted`, so the
/// long `INFO: Spawning:` line reads as background context next to bazel's
/// own `INFO:` output. Falls back to plain text when stderr takes no color
/// ([`crate::color::console_supports_color`], the gate every runtime line on
/// stderr uses).
pub(super) fn announce_spawn(
    announce: AnnounceSpawn,
    version: Option<&semver::Version>,
    cmd: &Command,
) {
    let palette = crate::color::console_supports_color().then(crate::theme::of_process);
    let line = |text: String| match &palette {
        Some(p) => format!(
            "{}: {}",
            p.paint("label.info", "INFO"),
            p.paint("muted", &text)
        ),
        None => format!("INFO: {text}"),
    };
    if announce.version {
        errln!("{}", line(bazel::command::version_line(version)));
    }
    if announce.command {
        errln!(
            "{}",
            line(format!("Spawning: {}", bazel::command::render(cmd)))
        );
    }
}

/// A captured console, started before the client so nothing can fail
/// between the client's spawn and its stderr having a reader.
struct Console {
    forwarder: Forwarder,
    /// The URLs the console's links wrote out beside them.
    links_written: bazel::console::links::WrittenCount,
    /// The client's stdout when it is a file of its own: captured too, and
    /// forwarded as written (colour the capture asked for aside) after what
    /// the console's forwarder has pending,
    /// so it stays in order with the stderr lines above it.
    stdout: Option<Forwarder>,
    /// The feed `output_feed()` drains, with `bazel::console::Output::Hidden`.
    feed: Option<Arc<bazel::console::feed::Feed>>,
    /// Lines the console writes between Bazel's: the history's note of where
    /// it kept each failed action's log.
    notes: bazel::console::forward::Notes,
    /// The failed actions' kept logs, which the console's recap names.
    kept_logs: bazel::console::forward::KeptLogs,
    observations: Arc<Observations>,
    events: Option<std::sync::mpsc::Receiver<HeldLine>>,
    log: Option<std::path::PathBuf>,
    /// The client's pid once spawned, 0 before. The console's stream ends
    /// when every writer has closed or, for a writer the client left behind,
    /// a quiet window after the client exits, and at most
    /// [`bazel::stream::fd::LINGER`] after the reader has caught up with it.
    client: Arc<std::sync::atomic::AtomicU32>,
    /// Set once the client is reaped (any wait that returns its status), or
    /// was never spawned: gone for the
    /// console's gate whatever a probe of its pid, which may by then name
    /// another process, would say.
    gone: Arc<std::sync::atomic::AtomicBool>,
    /// Sets `gone` if this console is dropped before the client spawned.
    abandon: Abandon,
}

/// Sets its flag when dropped, unless [`Abandon::disarm`]ed first.
struct Abandon(Option<Arc<std::sync::atomic::AtomicBool>>);

impl Abandon {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for Abandon {
    fn drop(&mut self) {
        if let Some(gone) = &self.0 {
            gone.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

impl Console {
    /// The client's stderr and, when it is inherited, its stdout (see
    /// [`bazel::console::capture`]: the same capture when stdout is stderr's
    /// file, a pipe of its own otherwise), and the forwarders reading them
    /// into this process's from their own threads, so a task's own lines
    /// interleave with them by whole line.
    fn start(
        config: &bazel::console::Config,
        stdout_inherited: bool,
    ) -> io::Result<(Stdio, Option<Stdio>, Console)> {
        use bazel::console::capture;
        let share_stdout = stdout_inherited && capture::stdout_is_stderrs_file();
        let capture::Capture {
            stderr: child_stderr,
            stdout: shared_stdout,
            reader,
            hold,
        } = capture::open_with_stdout(
            if config.pty {
                bazel::console::Mode::Pty
            } else {
                bazel::console::Mode::Pipe
            },
            share_stdout,
        )?;
        let probe = reader.try_clone()?;
        let client = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Armed before anything below can fail and drop this console.
        let abandon = Abandon(Some(gone.clone()));
        let gate = || {
            let (pid, gone) = (client.clone(), gone.clone());
            let mut gate = bazel::stream::Gate::new(move || {
                if gone.load(std::sync::atomic::Ordering::SeqCst) {
                    return true;
                }
                let pid = pid.load(std::sync::atomic::Ordering::SeqCst);
                pid != 0 && !bazel::process::is_pid_alive(pid)
            });
            gate.linger = Some(bazel::stream::fd::LINGER);
            gate
        };
        // The reader reports its own waits, so a stdout write ordered after
        // this forwarder never sees it waiting while a read is taking bytes.
        let activity = bazel::stream::Activity::default();
        let mut reader = bazel::stream::fd::Fd::follow(reader, gate())?.reporting(activity.clone());
        if let Some(hold) = hold {
            reader = reader.holding(hold);
        }
        let mut pipeline = config.pipeline();
        pipeline.shape.copy = Some(crate::out::copy_console_output);
        // The copy and the write under one turn of stderr's target.
        pipeline.shape.hold = Some(axl_types::terminal::hold_stderr);
        let log = pipeline.log.as_ref().map(|l| l.path().to_path_buf());
        let feed = pipeline.feed.clone();
        let notes = pipeline.notes.clone();
        let kept_logs = pipeline.kept_logs.clone();
        let forwarder = Forwarder::spawn_reported(
            Box::new(reader),
            Box::new(axl_types::terminal::Routed::new(std::io::stderr(), 2)),
            pipeline.chain,
            pipeline.log,
            pipeline.shape,
            Some(activity),
        )?;
        let (child_stdout, stdout) = match shared_stdout {
            Some(shared) => (Some(shared), None),
            None if stdout_inherited => {
                let (child, out) = capture::open(bazel::console::Mode::Pipe)?;
                let out = bazel::stream::fd::Fd::follow(out, gate())?;
                // The wait for stderr's forwarder runs before stdout's
                // target is held (`gate`), the write and its copy under it.
                let gate = std::sync::Arc::new(forwarder.gate(probe.into()));
                let after = axl_types::terminal::Routed::new(std::io::stdout(), 1);
                // Colour asked for the stages' sake is shown as the user
                // asked, else as the CLI's own stdout would carry it, as it
                // is for stderr.
                let shape = bazel::console::forward::Shape {
                    strip_colour: !config
                        .user_colour
                        .unwrap_or_else(|| crate::color::stdout_supports_color()),
                    // A stdout of its own is the program's output, not the
                    // console UI: shown as written even when nothing else is.
                    copy: Some(crate::out::copy_only),
                    hold: Some(axl_types::terminal::hold_stdout),
                    gate: Some(gate),
                    ..Default::default()
                };
                (
                    Some(child),
                    Some(Forwarder::spawn_logged(
                        Box::new(out),
                        Box::new(after),
                        vec![],
                        None,
                        shape,
                    )?),
                )
            }
            None => (None, None),
        };
        Ok((
            child_stderr,
            child_stdout,
            Console {
                forwarder,
                links_written: pipeline.links_written,
                stdout,
                feed,
                notes,
                kept_logs,
                observations: pipeline.observations,
                events: pipeline.events,
                log,
                client,
                abandon,
                gone,
            },
        ))
    }
}

/// Which pre-spawn `INFO:` lines to emit. Resolved from task flags in AXL
/// (`auto` → on under CI) and threaded down through `ctx.bazel.build` / `.test`.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnnounceSpawn {
    pub version: bool,
    pub command: bool,
}

#[derive(Debug, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.Build>")]
pub struct Build {
    #[allocative(skip)]
    build_event_stream: RefCell<Option<BuildEventStream>>,
    #[allocative(skip)]
    workspace_event_stream: RefCell<Option<WorkspaceEventStream>>,
    #[allocative(skip)]
    execlog_stream: RefCell<Option<ExecLogStream>>,

    /// The `bazel.execution_log.iterator()` handles subscribed to
    /// `execlog_stream`. Held so `wait()` can release them before joining the
    /// stream: each one is a subscriber the producer blocks for, so a task that
    /// never drained its handle would otherwise park the reader thread forever
    /// on a full channel. Holds no Starlark values — an `ExecLogIter` clone is
    /// its `kinds=` filter, a `Subscription` (an `Arc` to the shared state), and
    /// an `Arc<Signals>`.
    #[allocative(skip)]
    execlog_iters: Vec<ExecLogIter>,

    /// The `bazel.workspace_events.iterator()` handles subscribed to
    /// `workspace_event_stream`, held so `wait()` can release them before
    /// joining it, as for `execlog_iters`.
    #[allocative(skip)]
    workspace_event_iters: Vec<WorkspaceEventIter>,

    /// Whether something reads the execution log, rather than only recording
    /// it: a log that never appears is worth a warning only then.
    #[allocative(skip)]
    execlog_read: bool,

    /// The profile Bazel writes into the history, and the path the caller
    /// asked for it at, which [`Build::reaped`] copies it to.
    #[allocative(skip)]
    profile_copy: RefCell<Option<(PathBuf, PathBuf)>>,

    /// The captured console's forwarder, when the build was given `output =`.
    /// `wait()` joins it as soon as the child exits, so bazel's last lines are
    /// out before anything the task prints next.
    #[allocative(skip)]
    console: RefCell<Option<Forwarder>>,

    /// The forwarder of the client's own stdout, when the console took it
    /// as a file of its own; joined right after `console`.
    #[allocative(skip)]
    console_stdout: RefCell<Option<Forwarder>>,

    /// What the console's matchers saw: `output_fatal`, `output_fatal_line`
    /// and `output_matches()`.
    #[allocative(skip)]
    console_observations: Option<Arc<Observations>>,

    /// The URLs the console's links wrote out beside them:
    /// `output_links_written`.
    #[allocative(skip)]
    console_links_written: Option<bazel::console::links::WrittenCount>,

    /// Bazel's raw log, when the `output` processor changes what is shown.
    #[allocative(skip)]
    console_log: Option<std::path::PathBuf>,

    /// The history's copy of the build event stream, when it keeps one.
    #[allocative(skip)]
    kept_build_events: Option<std::path::PathBuf>,

    /// Tells the console's gate the client is gone once `wait()` reaps it.
    #[allocative(skip)]
    console_gone: Option<Arc<std::sync::atomic::AtomicBool>>,

    /// The lines `intercept` stages hold, for the `BazelTrait.output_intercept`
    /// hooks: answered by `run_output_hooks()` and by `wait()`, which answers
    /// the rest until the console ends.
    #[allocative(skip)]
    console_events: RefCell<Option<std::sync::mpsc::Receiver<HeldLine>>>,

    /// The first error a console hook raised, for `wait()` to raise once
    /// the build is joined: raised at once, it would leave the client
    /// running and the build's streams unjoined.
    #[allocative(skip)]
    hook_error: RefCell<Option<anyhow::Error>>,

    /// The `output_end` hooks ran: once per build, however often `wait()`
    /// is called, and before they run, so a hook that waits does not recurse.
    #[allocative(skip)]
    console_ended: std::cell::Cell<bool>,

    /// What is shown, for `output_feed()`, when the console shows nothing
    /// itself (`display = "none"`).
    #[allocative(skip)]
    console_feed: Option<Arc<bazel::console::feed::Feed>>,

    /// When the `output_frame` hooks last ran, for their cadence.
    #[allocative(skip)]
    last_frame: std::cell::Cell<Option<std::time::Instant>>,

    /// Whether the task has `output_frame` hooks, once a frame was asked
    /// for: so a build without them stops reading the trait for frames.
    #[allocative(skip)]
    has_frames: std::cell::Cell<Option<bool>>,

    /// Shared UUID every gRPC sink indexes this invocation under. Minted
    /// before bazel emits `build_started` so forwarders can start
    /// immediately; distinct from Bazel's `build_started.uuid`.
    #[allocative(skip)]
    sink_invocation_id: RefCell<Option<String>>,

    #[allocative(skip)]
    child: RefCell<Child>,

    /// The client's binding to the cancellation token it was spawned under:
    /// cancelling that token sends the client one SIGINT, bazel's own
    /// graceful cancel. `handle.cancellation` in AXL. Reaping the child
    /// (`wait` / `try_wait`) marks it exited so a reused pid is never
    /// signalled.
    #[allocative(skip)]
    bound: Bound,

    /// The run's cancellation state the client was bound under.
    #[allocative(skip)]
    signals: Arc<Signals>,

    /// The BES trace sink, joined by `wait()` once the stream ends.
    #[allocative(skip)]
    trace_sink: RefCell<Option<JoinHandle<SinkOutcome>>>,

    /// The sink keeping action and test logs in the history, joined by
    /// `wait()` once the stream ends, so its index is written by then; held
    /// by `history_slot` instead when there is one.
    #[allocative(skip)]
    keep_sink: RefCell<Option<JoinHandle<SinkOutcome>>>,

    /// What the keep sink has kept of each action so far, when action logs
    /// are asked for (`kept_action_log()`).
    #[allocative(skip)]
    kept_actions: Option<bazel::bes::sink::keep::KeptActions>,

    /// Its place in the task's record (`task.json`): its exit noted at the
    /// reap, its summary once `wait()` has joined the stream and the keep
    /// sink, whose indexes it reads.
    #[allocative(skip)]
    history_slot: Option<Arc<BazelSlot>>,

    #[allocative(skip)]
    span: RefCell<tracing::Span>,
}

/// Serve the caller's own `--build_event_binary_file` as a file sink.
///
/// The flag `Build::spawn` appends for the CLI's own event file outranks it (the
/// option is single-valued), so Bazel never writes the caller's file; the
/// stream's raw bytes are mirrored there instead, complete once `wait()` has
/// joined the stream. A file sink already naming the path (compared lexically,
/// not resolved) is left as its only writer. Its folder is created, as Bazel
/// creates it; one that cannot be costs only the copy, which then fails to open.
fn tee_requested_bep_file(file_sinks: &mut Vec<FileSink>, path: PathBuf, trim: bool) {
    if file_sinks.iter().any(|sink| Path::new(&sink.path) == path) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    file_sinks.push(FileSink {
        path: path.to_string_lossy().into_owned(),
        signal: Arc::new(FileSignal::new()),
        trim,
        flush_each: false,
    });
}

/// Set once this process is a full screen's child that reports its builds
/// to it (the History screen's re-run): see [`follow_build_events`].
static BUILD_EVENTS_FOLLOWED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Have the history's copy of each build's events (`build_events_file`)
/// flushed after every event, for the screen following it from another
/// process; otherwise it is written a buffer at a time, as every other file
/// sink is.
pub fn follow_build_events() {
    BUILD_EVENTS_FOLLOWED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// How long [`Build::spawn`] gives a cancelled client to exit before killing
/// it, so that abandoning the stream is bounded.
const ABANDON_STOP: Duration = Duration::from_secs(30);

/// A build event stream whose reader has not started, while bazel already runs
/// and writes its file.
///
/// Subscribers bind to [`Unstarted::stream`], then [`Unstarted::start`] starts
/// the reader. If [`Build::spawn`] fails first, [`Unstarted::abandon`] stops
/// the client and reads the file to its end, which removes it, before the
/// error is returned: the task, and with it the CLI, may exit right after, and
/// the file would be left in the temp dir holding the build's unredacted
/// events.
struct Unstarted {
    pending: Option<(BuildEventStream, Reservation, Gate, Vec<FileSink>)>,
}

impl Unstarted {
    fn new(
        file: Option<Reservation>,
        gate: Gate,
        file_sinks: Vec<FileSink>,
        max_message_bytes: Option<usize>,
    ) -> Self {
        Self {
            pending: file.map(|file| {
                // BES sinks never fail the build; their warnings are how the
                // user learns of a lost copy or upload.
                bazel::bes::sink::set_warn(crate::diag::warn);
                let stream = BuildEventStream::new();
                let stream = match max_message_bytes {
                    Some(n) => stream.max_message_bytes(n),
                    None => stream,
                };
                (stream, file, gate, file_sinks)
            }),
        }
    }

    fn stream(&self) -> Option<&BuildEventStream> {
        self.pending.as_ref().map(|(stream, ..)| stream)
    }

    fn start(self) -> Option<BuildEventStream> {
        let (stream, file, gate, file_sinks) = self.pending?;
        Some(stream.start(file, gate, file_sinks))
    }

    /// `stop_client` ends the client, so the gate can say the stream is over;
    /// then the stream is read to its end with whatever is bound.
    fn abandon(self, stop_client: impl FnOnce()) {
        stop_client();
        if let Some(mut stream) = self.start() {
            let _ = stream.join();
        }
    }
}

/// `(asked, recording)`: whether something reads a build's compact execution
/// log, and whether it is to be recorded in the history (which may also be
/// asked for).
///
/// A list holding nothing but `record()` sinks records; any other list asks.
/// A `requested` path, named by the caller's own flags, is written as a copy,
/// so it asks as a consumer does. `history_records` is the history request
/// flag or `record_every_build()`.
fn execlog_demand(
    listed: bool,
    sinks: &[ExecLogSink],
    has_iters: bool,
    requested: bool,
    history_records: bool,
) -> (bool, bool) {
    let record_only = listed
        && !has_iters
        && !sinks.is_empty()
        && sinks.iter().all(|s| matches!(s, ExecLogSink::Record));
    let asked = (listed && !record_only) || requested;
    (asked, record_only || history_records)
}

/// Whether a task's own execution-log iterator was said to get no entries
/// beside the caller's own log: once a process.
static OWN_LOG_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a build writes its compact execution log: when something `asked`
/// for it, or when it is only `recording` on Bazel 7.4 or later (or a version
/// that cannot be told). Recording starts at 7.4, where the flag has the name
/// the history request is spelled with in an rc (see `exec_log_flags`), so an
/// older Bazel records nothing either way.
fn writes_execution_log(asked: bool, recording: bool, version: Option<&semver::Version>) -> bool {
    asked || (recording && bazel::execlog::knows_compact_file_flag(version))
}

/// The workspace a Bazel command run in `directory` (the CLI's own cwd when
/// `None`) belongs to, found from where its client works: absolute, with
/// symlinks resolved. See [`crate::project_root::find_bazel_root`].
pub(crate) fn workspace_of(directory: Option<&str>) -> Option<PathBuf> {
    let dir = match directory {
        Some(dir) => std::env::current_dir().ok()?.join(dir),
        None => std::env::current_dir().ok()?,
    };
    crate::project_root::find_bazel_root(&dir.canonicalize().ok()?)
}

/// A fresh temp folder readable by this user alone, for a file there is no
/// history folder for: an execution log holds command lines and environment
/// variables, a raw console what a stage redacted. Left in place, as Bazel may
/// upload an execution log after the client exits.
fn private_temp_dir(prefix: &str) -> Option<PathBuf> {
    crate::engine::std::fs::create_temp_dir(&std::env::temp_dir(), prefix).ok()
}

/// A fresh path for an execution log there is no history folder for
/// ([`private_temp_dir`]), or directly in the temp dir where no folder can be
/// made there.
fn private_temp_path() -> PathBuf {
    match private_temp_dir("aspect-execlog-") {
        Some(dir) => dir.join(bazel::execlog::FILE_NAME),
        None => std::env::temp_dir().join(format!("execlog-{}.zstd", uuid::Uuid::new_v4())),
    }
}

/// This Bazel invocation's folder in the task invocation's history, created,
/// or `None` when it cannot be, said once per run (or the task keeps none,
/// quietly: `keep_history = False`): a history that cannot be
/// written never fails a build. Its logs go to the temp dir instead: the
/// execution and workspace logs, where Bazel can still upload a recorded one,
/// and the console's raw log. The profile is left to Bazel.
fn history_dir(history: &History) -> Option<PathBuf> {
    history
        .next_bazel_dir()
        .inspect_err(|err| {
            if !history.keeps_none() && history.first_failure() {
                crate::diag::warn(&format!(
                    "could not create a history folder under {}: {err}. Builds' \
                     execution, workspace and console logs go to the temp dir instead, and \
                     the profile is left to Bazel.",
                    history.root().display(),
                ))
            }
        })
        .ok()
}

impl Build {
    /// Wait for the client to exit, a safe point: once the root is cancelled
    /// the task's exit comes back instead. `Ok(None)` when `timeout` passes
    /// first. Reaping marks the binding exited.
    fn wait_child(
        &self,
        timeout: Option<Duration>,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        {
            let mut child = self.child.borrow_mut();
            drop(child.stdin.take());
        }
        let status =
            children::wait_with(&self.bound, timeout, || self.child.borrow_mut().try_wait())?;
        if let Some(status) = &status {
            self.reaped(status);
        }
        Ok(status)
    }

    /// What follows the client's exit, once a reap sees it: the task's
    /// record notes the exit, the console's gate is told the client is gone,
    /// so a pid another process may reuse is not probed for it, and the
    /// profile Bazel has finished by then is copied to the path the caller
    /// asked for. Only the first call copies. A copy that fails is a warning,
    /// as a profile Bazel cannot write is. A build never reaped here (a
    /// cancelled wait, a task that does not wait) leaves the profile only in
    /// the history.
    fn reaped(&self, status: &std::process::ExitStatus) {
        if let Some(slot) = &self.history_slot {
            slot.exited(status.code(), self.sink_invocation_id());
        }
        if let Some(gone) = &self.console_gone {
            gone.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if let Some((from, to)) = self.profile_copy.take()
            && let Err(err) = bazel::profile::copy(&from, &to)
        {
            crate::diag::warn(&format!(
                "could not copy the profile from {} to {}: {err}",
                from.display(),
                to.display(),
            ));
        }
    }

    /// Spawn one bazel command and the readers of what it writes.
    ///
    /// The order is the contract: every output path is reserved and put on the
    /// command line before bazel is spawned; the readers start once the client
    /// pid exists, since its exit is what ends them (see `PidPair`); and the
    /// build event reader starts last, after every subscriber (iterators, gRPC
    /// and tracing sinks) is bound, so none misses the start of the stream.
    ///
    /// Every event is held, once for the whole stream, to the smallest
    /// `max_event_bytes` of the gRPC sinks in `build_events` (each unset one
    /// counting as 4 MiB), and to 4 MiB with none: iterators, file sinks and
    /// the history read the same capped stream the backends do.
    // TODO: this should return a thiserror::Error
    pub fn spawn(
        verb: &str,
        targets: impl IntoIterator<Item = String>,
        (build_events, sinks, iters): (bool, Vec<BuildEventSink>, Vec<BuildEventIter>),
        (execlog_enabled, execlog_sinks, execlog_iters): (bool, Vec<ExecLogSink>, Vec<ExecLogIter>),
        (workspace_events, workspace_event_iters): (bool, Vec<WorkspaceEventIter>),
        flags: Vec<String>,
        startup_flags: Vec<String>,
        stdout: Stdio,
        stderr: Stdio,
        console: Option<bazel::console::Config>,
        stdout_inherited: bool,
        directory: Option<String>,
        announce: AnnounceSpawn,
        signals: Arc<Signals>,
        telemetry: &TelemetrySources,
        cancellation: &CancellationToken,
        rt: AsyncRuntime,
        history: &History,
        keep_logs: bazel::bes::sink::keep::Keep,
        publish_all_actions: bool,
    ) -> Result<Build, std::io::Error> {
        // Logs to keep are read from the stream, which a build keeping some
        // has even with no other consumer; and keeping them makes the history
        // folder (below).
        let (build_events, bes_telemetry) = telemetry
            .build_events()
            .resolve(build_events || keep_logs.any(), telemetry.exporting());
        let (flags, requests) = bazel::cli_request::take(flags);
        let history_asked = requests.execlog_in_history;
        let publish_all_actions = publish_all_actions || requests.all_actions;
        let workspace = workspace_of(directory.as_deref());
        let requested_execlog_path = bazel::execlog::requested_path(
            &flags,
            verb,
            directory.as_deref(),
            workspace.as_deref(),
        );
        // Without an rc the CLI rendered, Bazel reads its own rc files, which
        // can name an execution log or profile path no flag here shows. A file
        // only recorded, or the profile, is then left where Bazel puts it, so
        // no path the caller set is silently overridden.
        let rc_visible = startup_flags.iter().any(|f| f == "--ignore_all_rc_files");
        // A caller who turned profiling off is given no `--profile`, and one
        // whose `--profile` path cannot be told has it left to Bazel.
        let requested_profile = bazel::profile::requested_path(&flags, verb, workspace.as_deref());
        let profile_unplaced = requested_profile.is_none()
            && bazel::rc::last_flag_value(&flags, bazel::profile::FLAG, verb)
                .is_some_and(|v| !v.is_empty());
        let profile_kept =
            rc_visible && !profile_unplaced && !bazel::profile::turned_off(&flags, verb);
        // Bazel refuses a build naming more than one log format. Beside the
        // caller's binary or JSON one, iterators that only read entries (with
        // no file of the compact log asked for) get none: every handle, a
        // hook's or a task's own, ends at once, rather than the build be
        // refused; a task's own with a warning, once.
        let other_format = bazel::execlog::other_format(&flags, verb);
        let mut execlog_iters = execlog_iters;
        if other_format
            && requested_execlog_path.is_none()
            && execlog_sinks
                .iter()
                .all(|s| matches!(s, ExecLogSink::Record))
        {
            if execlog_iters.iter().any(|i| !i.is_hook())
                && !OWN_LOG_WARNED.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                crate::diag::warn(
                    "an execution_log iterator gets no entries: the build writes the caller's own execution log (--execution_log_binary_file or --execution_log_json_file), and Bazel writes only one format",
                );
            }
            execlog_iters.iter().for_each(ExecLogIter::release);
            execlog_iters.clear();
        }
        let (asked, recording) = execlog_demand(
            execlog_enabled
                && !(other_format
                    && execlog_iters.is_empty()
                    && execlog_sinks
                        .iter()
                        .all(|s| matches!(s, ExecLogSink::Record))),
            &execlog_sinks,
            !execlog_iters.is_empty(),
            requested_execlog_path.is_some(),
            history_asked || history.records_every_build(),
        );
        // A log only recorded is skipped beside the caller's binary or JSON
        // one too (`other_format`, above).
        let recording = recording && rc_visible && !other_format;

        // Asked with the same startup flags the build below will use, because
        // that is what decides which server answers. Bazel kills a running
        // server whose startup options differ from the ones it is handed, so a
        // pid read without them is the pid of a server the build then replaces.
        //
        // The execution log, the build event file and the workspace log are
        // regular files whose end needs the client to have exited too (see
        // `bazel::stream`), so a stale `pid` can only cut a tail the server is
        // still writing after the client has gone, or, at a retry, the abandoned
        // attempt's file if the server is still writing it once the next has
        // started.
        let (pid, version) =
            bazel::server::Server::new(&super::runner::CancellableRunner(&signals), &startup_flags)
                .info()?;
        let recording = recording && writes_execution_log(false, true, version.as_ref());
        // `auto` telemetry reads a log the build writes anyway; `always`
        // turning it on for its telemetry is an ask of its own.
        let (log_on, execlog_telemetry) = telemetry
            .execution_log()
            .resolve(asked || recording, telemetry.exporting());
        // Nor does telemetry turn on a second format beside the caller's.
        let (log_on, execlog_telemetry) = if other_format && !asked {
            (false, false)
        } else {
            (log_on, execlog_telemetry)
        };
        let asked = asked || (log_on && !recording);
        let execlog_enabled = writes_execution_log(asked, recording, version.as_ref());
        // Every file below that stays goes into this one folder, made only
        // when one will.
        let console_log = console.as_ref().is_some_and(|c| c.keeps_log());
        let history_dir =
            (profile_kept || build_events || workspace_events || execlog_enabled || console_log)
                .then(|| history_dir(history))
                .flatten();
        // The console's raw log stays there too, with the files of what its
        // stages folded beside it; without a folder, in a private temp one,
        // and with neither, none: never at a name another user could take.
        let console = console.map(|mut config| {
            if config.keeps_log() {
                config.log = history_dir
                    .clone()
                    .or_else(|| private_temp_dir("aspect-console-"))
                    .map(|dir| {
                        bazel::console::log::Place::At(dir.join(bazel::console::log::FILE_NAME))
                    });
            }
            config
        });

        let span = tracing::info_span!(
            "ctx.bazel.build",
            build_events = build_events,
            workspace_events = workspace_events,
            execution_log = execlog_enabled,
            flags = ?flags
        );
        let _enter = span.enter();

        let targets: Vec<String> = targets.into_iter().collect();
        // Noted in the task's record once the client has been spawned.
        let (recorded_targets, recorded_flags) = (targets.clone(), flags.clone());

        // Read before `flags` moves into the command. When the CLI streams the
        // compact log or the BEP, its own flag below outranks the caller's, and
        // the file the caller asked for is written from the stream instead.
        let requested_bep_path = bazel::bes::binary_file::requested_path(
            &flags,
            verb,
            directory.as_deref(),
            workspace.as_deref(),
        );
        let requested_workspace_log =
            bazel::workspace_event::requested_path(&flags, verb, directory.as_deref());
        // Every action is reported when the CLI's own readers need it; then,
        // unless the caller asked for it too, what goes downstream is trimmed
        // back to what Bazel reports without it. An rc the CLI did not render
        // may ask for it unseen, so nothing is trimmed then.
        let cli_publishes_all_actions = build_events
            && (publish_all_actions
                || keep_logs.actions == bazel::bes::sink::keep::ActionLogs::All)
            && !bazel::bes::sink::downstream::asks_for_all_actions(&flags, verb);
        let trim_downstream = cli_publishes_all_actions && rc_visible;

        let mut cmd = super::runner::CancellableRunner(&signals).command();
        cmd.args(startup_flags);
        cmd.arg(verb);
        cmd.args(flags);

        if let Some(directory) = directory {
            cmd.current_dir(directory);
        }

        // File sinks share the BES reader's raw-bytes path (bazel's bytes, but
        // with secrets redacted); gRPC sinks run as broadcaster subscriber threads.
        let mut file_sinks: Vec<FileSink> = vec![];
        let mut grpc_sinks: Vec<BuildEventSink> = vec![];
        for sink in sinks {
            if let Some(path) = sink.file_path() {
                let signal = sink.bind_file().map_err(io::Error::other)?;
                file_sinks.push(FileSink {
                    path,
                    signal,
                    trim: trim_downstream,
                    flush_each: false,
                });
            } else {
                grpc_sinks.push(sink);
            }
        }
        let max_message_bytes = grpc_sinks
            .iter()
            .filter_map(BuildEventSink::grpc_max_message_bytes)
            .min();
        if build_events && let Some(path) = requested_bep_path {
            tee_requested_bep_file(&mut file_sinks, path, trim_downstream);
        }
        // The history keeps the whole stream, redacted. Bazel's
        // own file is a temp file the reader moves away at each attempt and
        // removes, since nothing reads it by path once the stream is over.
        let kept_build_events = history_dir
            .as_ref()
            .filter(|_| build_events)
            .map(|dir| dir.join(bazel::bes::binary_file::FILE_NAME));
        if let Some(path) = &kept_build_events {
            file_sinks.push(FileSink {
                flush_each: BUILD_EVENTS_FOLLOWED.load(std::sync::atomic::Ordering::SeqCst),
                ..FileSink::new(
                    path.to_string_lossy().into_owned(),
                    Arc::new(FileSignal::new()),
                )
            });
        }

        // Reserve the BEP path now, before `cmd.spawn()`, so it can go on the
        // command line; bazel creates the file. The reader is started later,
        // once the spawned child's pid is in hand and every subscriber is bound.
        //
        // Bazel's default action publishing (failed actions only) is left
        // alone, per-action data coming from the execution log, unless the
        // CLI's own readers need every action (above).
        let bes_file = if build_events {
            let file = BuildEventStream::reserve();
            cmd.arg("--build_event_binary_file_upload_mode=fully_async")
                .arg(bazel::bes::binary_file::FLAG)
                .arg(file.path());
            if cli_publishes_all_actions {
                cmd.arg(bazel::bes::sink::downstream::PUBLISH_ALL_ACTIONS);
            }
            Some(file)
        } else {
            None
        };

        // Bazel writes the profile into the history, where it stays, as the
        // execution log does below; a profile the caller asked for is copied
        // to the caller's path once the client exits (see `Build::reaped`).
        let profile_copy = history_dir
            .as_ref()
            .filter(|_| profile_kept)
            .and_then(|dir| {
                let path = dir.join(bazel::profile::file_name(requested_profile.as_deref()));
                cmd.arg(format!("{}={}", bazel::profile::FLAG, path.display()));
                // An earlier run's profile at the caller's path is not left to
                // be read as this build's, should this one never be reaped.
                requested_profile.map(|to| {
                    let _ = std::fs::remove_file(&to);
                    (path, to)
                })
            });

        // The workspace log stays in the history too; a path the caller asked
        // for is written as a copy while it is read. Without a history folder
        // the build reserves a temp path, removed once read. A log nobody here
        // reads is left to whatever asked for it.
        let workspace_event_file = workspace_events.then(|| {
            let log = match &history_dir {
                Some(dir) => bazel::workspace_event::LogFile::Kept(
                    dir.join(bazel::workspace_event::FILE_NAME),
                ),
                None => bazel::workspace_event::LogFile::reserve(),
            };
            cmd.arg(bazel::workspace_event::LOG_FILE_FLAG)
                .arg(log.path());
            let copies = requested_workspace_log
                .map(|path| path.to_string_lossy().into_owned())
                .into_iter()
                .collect::<Vec<_>>();
            (log, copies)
        });

        // Compact paths go to the tee reader inside the stream thread; decoded
        // paths get a writer thread each, cloning a subscriber of the decoded
        // stream, as the spawn spans do.
        let mut compact_paths: Vec<String> = vec![];
        let mut decoded_paths: Vec<String> = vec![];
        for sink in execlog_sinks {
            match sink {
                ExecLogSink::CompactFile { path } => compact_paths.push(path),
                ExecLogSink::File { path } => decoded_paths.push(path),
                ExecLogSink::Record => {}
            }
        }

        // Bazel writes the log into this task invocation's history, where it
        // stays: Bazel uploads it from there as a build tool log once the
        // build is over. Without a history folder it goes to a fresh temp file,
        // left in place for the same reason. A path the caller asked for is
        // written as a copy. The reader is started after the spawn, once the
        // client pid exists.
        let execlog_path = execlog_enabled.then(|| {
            let path = match &history_dir {
                Some(dir) => dir.join(bazel::execlog::FILE_NAME),
                None => private_temp_path(),
            };
            cmd.arg(format!(
                "{}={}",
                bazel::execlog::compact_file_flag(version.as_ref()),
                path.display()
            ));
            compact_paths =
                bazel::execlog::copies(requested_execlog_path, std::mem::take(&mut compact_paths));
            path
        });

        cmd.arg("--"); // separate flags from target patterns (not strictly necessary for build & test verbs but good form)
        cmd.args(targets);

        crate::trace!("exec: {:?}", cmd.get_args());
        announce_spawn(announce, version.as_ref(), &cmd);

        let console = match console {
            Some(config) => {
                let (child_stderr, child_stdout, console) =
                    Console::start(&config, stdout_inherited)?;
                cmd.stdout(child_stdout.unwrap_or(stdout));
                cmd.stderr(child_stderr);
                Some(console)
            }
            None => {
                cmd.stdout(stdout);
                cmd.stderr(stderr);
                None
            }
        };
        cmd.stdin(Stdio::null());

        // Bound to `cancellation` (the root unless the task said otherwise):
        // cancelling it runs bazel's Ctrl+C sequence on the client's tree.
        let (mut child, bound) = children::spawn(
            &mut cmd,
            &signals,
            cancellation,
            None,
            children::Kind::Bazel,
        )
        .map_err(|e| io::Error::other(format!("failed to spawn bazel: {e}")))?;
        let history_slot = history.bazel_started(
            verb,
            &recorded_targets,
            &recorded_flags,
            history_dir.as_deref(),
        );

        // The client pid ends the streams: the server outlives the invocation
        // and cannot say a build is over (see `PidPair`).
        let pids = PidPair {
            client: child.id(),
            server: Some(pid),
        };

        let mut console_gone = None;
        let (
            console,
            console_stdout,
            console_observations,
            console_links_written,
            console_events,
            console_log,
            console_feed,
            console_notes,
        ) = match console {
            Some(console) => {
                console
                    .client
                    .store(child.id(), std::sync::atomic::Ordering::SeqCst);
                console.abandon.disarm();
                console_gone = Some(console.gone);
                (
                    Some(console.forwarder),
                    console.stdout,
                    Some(console.observations),
                    Some(console.links_written),
                    console.events,
                    console.log,
                    console.feed,
                    Some((console.notes, console.kept_logs)),
                )
            }
            None => (None, None, None, None, None, None, None, None),
        };
        let build_event_stream =
            Unstarted::new(bes_file, pids.gate(), file_sinks, max_message_bytes);

        // Everything that can fail once bazel runs. A failure abandons the
        // stream (see `Unstarted`), and releases the handles this spawn bound.
        let mut execlog_bound = false;
        // What the keep sink has kept of each action so far, when action
        // logs are asked for: empty if no history folder could be made.
        let kept_actions = (keep_logs.actions != bazel::bes::sink::keep::ActionLogs::None)
            .then(bazel::bes::sink::keep::KeptActions::default);
        let parts = (|| -> io::Result<_> {
            // Started first: Bazel already has the path, so a stream dropped on
            // a later failure still reads to the client's exit, which `abandon`
            // forces. A temp reservation's detached reader removes the file when
            // it finishes, unless the process exits first.
            let mut workspace_event_stream = workspace_event_file
                .map(|(log, copies)| WorkspaceEventStream::spawn(log, pids.gate(), copies));

            // Same two-pid split for the execution log: the daemon writes the file, but
            // only the client can say the invocation is over and none is coming.
            //
            // The producer blocks rather than dropping. Blocking cannot reach Bazel —
            // Bazel writes the log to a regular file and this process tails it — so a
            // consumer slower than the decoder only lengthens the `join()` inside
            // `wait()`. And that `wait()` releases every subscriber before joining, so
            // one nobody drains cannot park the reader past the end of the build.
            // A log only recorded has nothing to read it, so no reader whose
            // failure could fail the build.
            let mut execlog_stream = match execlog_path.filter(|_| asked || execlog_telemetry) {
                Some(path) => Some(ExecLogStream::spawn_with_file(
                    path,
                    pid,
                    child.id(),
                    compact_paths,
                )?),
                None => None,
            };

            if !iters.is_empty() {
                let stream = build_event_stream.stream().ok_or_else(|| {
                    io::Error::other(
                        "ctx.bazel.build/test: build_events list contained `iterator()` handles \
                         but no BEP stream is configured",
                    )
                })?;
                for iter in &iters {
                    iter.bind(stream).map_err(io::Error::other)?;
                }
            }

            // One shared invocation_id across all gRPC sinks so every backend
            // indexes this invocation under the same UUID.
            let debug = std::env::var_os("ASPECT_DEBUG")
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let sink_invocation_id: Option<String> = if !grpc_sinks.is_empty() {
                let invocation_id = uuid::Uuid::new_v4().to_string();
                if debug {
                    errln!(
                        "BES sinks: spawning {} gRPC sink(s) sink_invocation_id={}",
                        grpc_sinks.len(),
                        invocation_id
                    );
                }
                let stream = build_event_stream.stream().unwrap();
                for sink in grpc_sinks {
                    sink.bind_grpc(rt.clone(), stream, invocation_id.clone(), trim_downstream)
                        .map_err(io::Error::other)?;
                }
                Some(invocation_id)
            } else {
                if debug {
                    let bes_backend = std::env::var("ASPECT_WORKFLOWS_BES_BACKEND")
                        .unwrap_or_else(|_| "<unset>".to_string());
                    let bes_results = std::env::var("ASPECT_WORKFLOWS_BES_RESULTS_URL")
                        .unwrap_or_else(|_| "<unset>".to_string());
                    errln!(
                        "BES sinks: 0 gRPC sinks configured (skipping spawn). \
                         ASPECT_WORKFLOWS_BES_BACKEND={bes_backend} \
                         ASPECT_WORKFLOWS_BES_RESULTS_URL={bes_results}"
                    );
                }
                None
            };

            // Decoded execlog sinks belong to the execlog stream — joined (and
            // write errors propagated) inside `execlog_stream.join()`.
            if let Some(stream) = execlog_stream.as_mut() {
                for path in decoded_paths {
                    let recv = subscriber(stream.receiver(), "execution log")?;
                    stream.attach_sink(bazel::execlog::sink::spawn_file(recv, path));
                }
                if execlog_telemetry {
                    let recv = subscriber(stream.receiver(), "execution log")?;
                    stream.attach_sink(bazel::execlog::telemetry::spawn(recv));
                }
            }

            // One receiver clone each: the channel is a broadcast ring, so the handles
            // read the same entries the decoded sinks do rather than competing for them.
            if !execlog_iters.is_empty() {
                let stream = execlog_stream.as_ref().ok_or_else(|| {
                    // `partition_execution_log` turns the stream on for any list, so a
                    // list holding a handle always has one. Stated, not unwrapped.
                    io::Error::other(
                        "ctx.bazel.build/test: execution_log list contained `iterator()` handles \
                         but no execution log stream is configured",
                    )
                })?;
                bind_handles(&execlog_iters, || stream.receiver(), "execution log")?;
                execlog_bound = true;
            }

            // Every consumer has cloned what it needs; an unread subscriber left
            // on the stream would cap them all (see `bazel::subscription::Origin`).
            if let Some(stream) = execlog_stream.as_mut() {
                stream.seal();
            }

            // Bound last, so no later failure can strand its handles.
            if let Some(stream) = workspace_event_stream.as_mut() {
                bind_handles(
                    &workspace_event_iters,
                    || stream.receiver(),
                    "workspace event",
                )?;
                stream.seal();
            }
            let trace_sink = bes_telemetry.then(|| {
                tracing_sink::Tracing::spawn(build_event_stream.stream().unwrap().subscribe())
            });
            // Without a history folder (none can be made) nothing is kept;
            // `history_dir` has said why.
            let keep_sink = match (keep_logs.any(), &history_dir, build_event_stream.stream()) {
                (true, Some(dir), Some(stream)) => {
                    let published = kept_actions.clone().unwrap_or_default();
                    Some(keep_logs.spawn(
                        stream.subscribe_filtered(Some(keep_logs.filter())),
                        dir.clone(),
                        published,
                        console_notes.clone(),
                    ))
                }
                _ => None,
            };
            // With a record, the record's slot joins the keep sink, first,
            // so its failures are matched with the indexes the sink writes.
            let keep_sink = match (&history_slot, keep_sink) {
                (Some(slot), Some(keep)) => {
                    slot.keep_with(keep);
                    None
                }
                (_, keep) => keep,
            };
            if let (Some(slot), Some(stream)) = (&history_slot, build_event_stream.stream()) {
                slot.summarize_from(bazel::bes::sink::summary::spawn(
                    stream.subscribe_filtered(Some(bazel::bes::sink::summary::filter())),
                ));
            }
            Ok((
                workspace_event_stream,
                execlog_stream,
                sink_invocation_id,
                trace_sink,
                keep_sink,
            ))
        })();
        let (workspace_event_stream, execlog_stream, sink_invocation_id, trace_sink, keep_sink) =
            match parts {
                Ok(parts) => parts,
                Err(err) => {
                    if let Some(slot) = &history_slot {
                        slot.failed(&err.to_string());
                    }
                    // A handle bound before the failure is a subscriber its reader
                    // would wait for forever; nothing else will release it.
                    if execlog_bound {
                        execlog_iters.iter().for_each(ExecLogIter::release);
                    }
                    // Held console lines too: a line the forwarder holds would
                    // keep Bazel's stderr undrained while it is stopped below.
                    drop(console_events);
                    build_event_stream.abandon(|| {
                        bound.token().cancel();
                        let waited =
                            children::wait_with(&bound, Some(ABANDON_STOP), || child.try_wait());
                        if !matches!(waited, Ok(Some(_))) {
                            // The whole tree: the real client behind bazelisk or
                            // a wrapper would outlive its parent.
                            bazel::process::signal::kill(bound.target());
                            let _ = child.wait();
                            bound.mark_exited();
                        }
                    });
                    // Reaped: the console's reader must not probe its pid again.
                    if let Some(gone) = &console_gone {
                        gone.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    return Err(err);
                }
            };

        // Start reading only now that every subscriber is bound (see
        // `BuildEventStream::new`).
        let build_event_stream = build_event_stream.start();

        drop(_enter);
        Ok(Self {
            child: RefCell::new(child),
            build_event_stream: RefCell::new(build_event_stream),
            workspace_event_stream: RefCell::new(workspace_event_stream),
            execlog_stream: RefCell::new(execlog_stream),
            execlog_iters,
            workspace_event_iters,
            execlog_read: asked,
            profile_copy: RefCell::new(profile_copy),
            console: RefCell::new(console),
            console_stdout: RefCell::new(console_stdout),
            console_observations,
            console_links_written,
            console_log,
            kept_build_events,
            console_gone,
            console_events: RefCell::new(console_events),
            hook_error: RefCell::new(None),
            console_ended: std::cell::Cell::new(false),
            console_feed,
            last_frame: std::cell::Cell::new(None),
            has_frames: std::cell::Cell::new(None),
            sink_invocation_id: RefCell::new(sink_invocation_id),
            bound,
            signals,
            trace_sink: RefCell::new(trace_sink),
            keep_sink: RefCell::new(keep_sink),
            kept_actions,
            history_slot,
            span: RefCell::new(span),
        })
    }
}

impl<'v> AllocValue<'v> for Build {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.build.Build")]
impl<'v> values::StarlarkValue<'v> for Build {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("build_methods", build_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: values::Heap<'v>) -> Option<values::Value<'v>> {
        match attribute {
            // The shared invocation ID that every gRPC BES sink used when
            // forwarding this build's events. Empty string when no BES sinks
            // were configured. Differs from Bazel's build_started.uuid.
            "sink_invocation_id" => {
                let id = self.sink_invocation_id.borrow();
                Some(heap.alloc_str(id.as_deref().unwrap_or("")).to_value())
            }
            // Whether a `fatal` stage of the `output` processor matched.
            // False without one.
            "output_fatal" => Some(Value::new_bool(
                self.console_observations
                    .as_ref()
                    .is_some_and(|o| o.fatal()),
            )),
            // The first line a `fatal` stage matched, or None.
            "output_fatal_line" => Some(
                match self
                    .console_observations
                    .as_ref()
                    .and_then(|o| o.fatal_line())
                {
                    Some(line) => heap.alloc_str(&line).to_value(),
                    None => Value::new_none(),
                },
            ),
            // Millis the console has been waiting for Bazel to write; 0 while
            // it processes or holds a line, without `output`, and once
            // `wait()` has joined it. With the client still running, a
            // growing value is a stall.
            "output_idle_ms" => {
                // The less idle of the console and a stdout of its own: bazel
                // writing either is not a stall.
                let idle = [&self.console, &self.console_stdout]
                    .iter()
                    .filter_map(|c| c.borrow().as_ref().map(|f| f.idle_ms()))
                    .min()
                    .unwrap_or(0);
                Some(heap.alloc(idle.min(i32::MAX as u64) as i32))
            }
            // Bazel's raw log path: its stderr (and stdout, when it
            // is the same file) as it wrote it, kept when the `output`
            // processor changes what is shown. None otherwise, or when it
            // could not be created.
            "output_log" => Some(match &self.console_log {
                Some(path) => heap.alloc_str(&path.to_string_lossy()).to_value(),
                None => Value::new_none(),
            }),
            // The task history's copy of the build event stream, written as
            // the build runs, or None when the history keeps none: for a
            // reader following the build from outside (`bazel.build_events
            // .follow`).
            "build_events_file" => Some(match &self.kept_build_events {
                Some(path) => heap.alloc_str(&path.to_string_lossy()).to_value(),
                None => Value::new_none(),
            }),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: values::Heap<'v>) -> bool {
        matches!(
            attribute,
            "sink_invocation_id"
                | "output_fatal"
                | "output_fatal_line"
                | "output_idle_ms"
                | "output_log"
                | "build_events_file"
        )
    }
}

#[starlark_module]
pub(crate) fn build_methods(registry: &mut MethodsBuilder) {
    /// Run the `BazelTrait.output_frame` hooks when a frame is due, then
    /// answer the lines the console's `intercept` stages hold now with the
    /// `BazelTrait.output_intercept` hooks, waiting briefly after each for
    /// the next a burst brings, up to a budget per call. For a task's build
    /// loop, so held lines are answered and frames drawn while the build
    /// runs; `wait()` does the rest. With `until_ended`, does both until the
    /// console ends instead, as `wait()` does, for a task about to block on
    /// something else first. Does nothing for a build with neither. A hook's
    /// error is raised by `wait()`, once the build is joined.
    fn run_output_hooks<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = false)] until_ended: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        // Called for every build event: when there is nothing to answer and
        // no frame is due, skip reading the trait.
        if build.console_events.borrow().is_none() && !until_ended && !build.frame_due() {
            return Ok(NoneType);
        }
        let console = TraitConsole::of(eval);
        if let Some(console) = &console {
            build.frame(console, this, eval, false);
        }
        if build.console_events.borrow().is_none() && !until_ended {
            return Ok(NoneType);
        }
        if until_ended {
            build.answer_until_the_console_ends(console.as_ref(), this, eval);
            return Ok(NoneType);
        }
        let Some(console) = console else {
            return Ok(NoneType);
        };
        let mut wait = Duration::ZERO;
        for _ in 0..HOOK_BUDGET {
            let held = {
                let events = build.console_events.borrow();
                let Some(rx) = events.as_ref() else {
                    break;
                };
                match rx.recv_timeout(wait) {
                    Ok(held) => held,
                    Err(_) => break,
                }
            };
            if let Err(e) = console.answer(held, eval) {
                build.hook_failed("intercept", e);
                break;
            }
            wait = HOOK_FOLLOW;
        }
        Ok(NoneType)
    }

    /// What the console showed since the last call, for a build whose
    /// processor shows nothing itself (`display = "none"`), as a dict:
    /// `lines`, the lines the stages passed, each drawable in one row:
    /// escape sequences stripped, a carriage return's redraw kept at its
    /// newest state, and other control characters but a tab removed (a
    /// backspace too, not applied);
    /// `links`, each line's links at the same index, as `[start, end, url]`
    /// with `start` and `end` characters of the line: the URLs and the paths
    /// to files the console linked (`--console:file-hyperlink-format` says
    /// what a file's opens), for a screen to draw as links and open;
    /// `dropped`, how many older lines were let go to keep at most 10000
    /// between calls; and `progress`, the rows of Bazel's progress area as it
    /// was before its latest redraw (at most one redraw behind), its status
    /// line (`[n / m] …`) first and then one per running action, empty once
    /// the console ends. None for any other build.
    fn output_feed<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        let Some(feed) = &build.console_feed else {
            return Ok(values::Value::new_none());
        };
        let got = feed.drain();
        Ok(heap.alloc(serde_json::json!({
            "lines": got.lines,
            "links": got.links,
            "colours": got.colours,
            "dropped": got.dropped,
            "progress": got.progress,
        })))
    }

    /// How many URLs the console wrote out beside its links, for a terminal
    /// that shows no OSC 8 link but opens a URL written out (the macOS
    /// Terminal): a line's, the recap's and the summary's paths. So far
    /// during the build, all of them after `wait()`. 0 without a console.
    #[starlark(attribute)]
    fn output_links_written<'v>(this: values::Value<'v>) -> anyhow::Result<u64> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        Ok(build.console_links_written.as_ref().map_or(0, |w| w.get()))
    }

    /// The logs this build's history has kept, so far, of the action whose
    /// primary output (`ActionCompleted`'s `primary_output`) is `output`: a
    /// dict of `{"stdout": path, "stderr": path}`, each stream it kept, empty
    /// when it kept neither (a retry's action replaces its earlier
    /// attempt's). Kept as each one arrives, so a log Bazel has since
    /// replaced can still be read. None when the build keeps no action logs
    /// (`keep_action_logs = "none"`, `--history:action-logs=none`).
    fn kept_action_log<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] output: &str,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        let Some(kept) = &build.kept_actions else {
            return Ok(values::Value::new_none());
        };
        let mut streams = serde_json::Map::new();
        if let Some(k) = kept.lock().unwrap().get(output) {
            for (name, path) in [("stdout", &k.stdout), ("stderr", &k.stderr)] {
                if let Some(p) = path {
                    streams.insert(name.into(), p.to_string_lossy().into_owned().into());
                }
            }
        }
        Ok(heap.alloc(serde_json::Value::Object(streams)))
    }

    /// The `(id, line)` hits of the `output` processor's `observe` stages:
    /// those so far during the build, all of them after `wait()`, in the
    /// order they came. Empty without a processor. At most 1024 hits are
    /// kept, besides each id's first, each line cut to 2048 bytes.
    fn output_matches<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<(String, String)>> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        Ok(build
            .console_observations
            .as_ref()
            .map(|o| o.matches())
            .unwrap_or_default())
    }

    /// What the `output` processor's `aggregate` stages kept, one dict per
    /// stage in the order they run: `{"id", "count", "groups", "samples",
    /// "dropped_groups", "dropped_groups_truncated", "dropped_matches"}`. A
    /// group is `{"key": {capture: value}, "line" and "captures" (its first
    /// match's), "last" (its last match), "count", "values": {name: value},
    /// "truncated": [name]}`; a sample, and a group's `last`, is `{"group":
    /// index into groups, "seq": which match, from 1, "line", "captures":
    /// {name: value}}`. So far during the build, all of it after `wait()`.
    /// Empty without a processor.
    fn output_aggregates<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        let reports = build
            .console_observations
            .as_ref()
            .map(|o| o.aggregates())
            .unwrap_or_default();
        Ok(heap.alloc(serde_json::Value::Array(reports)))
    }

    /// The client's cancellation token, a `child()` of the one passed as
    /// `cancellation =` at the spawn (`ctx.cancellation.root` by default).
    /// `build.cancellation.cancel()` sends the client one SIGINT, bazel's
    /// own graceful cancel; the build then ends the way an interrupted bazel
    /// does, with exit code 8, and `wait()` returns.
    #[starlark(attribute)]
    fn cancellation<'v>(
        this: values::Value<'v>,
    ) -> anyhow::Result<crate::engine::cancellation::Token> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        Ok(crate::engine::cancellation::Token::new(
            build.bound.token().clone(),
            build.signals.clone(),
        ))
    }

    /// Send the client one SIGINT now, bazel's graceful cancel, without
    /// going through its token: to the client and every process below it, so
    /// the real client behind bazelisk or a wrapper script gets it too.
    fn interrupt<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        // Nothing after a reap: the pid may be someone else's by then.
        if !build.bound.exited() {
            bazel::process::signal::interrupt(build.bound.target());
        }
        Ok(NoneType)
    }

    /// Check whether the invocation has finished: its `BuildStatus`, or
    /// `None` while the client is still running. Non-blocking by default;
    /// with `timeout_ms` it waits up to that long first, a bounded wait for
    /// a loop that also watches a cancellation token.
    fn try_wait<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = 0)] timeout_ms: i32,
    ) -> anyhow::Result<NoneOr<BuildStatus>> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        if timeout_ms < 0 {
            anyhow::bail!("timeout_ms must not be negative: {timeout_ms}");
        }
        let status = if timeout_ms == 0 {
            let status = children::try_wait(&mut build.child.borrow_mut(), &build.bound)?;
            if let Some(status) = &status {
                build.reaped(status);
            }
            status
        } else {
            build.wait_child(Some(Duration::from_millis(timeout_ms as u64)))?
        };
        Ok(match status {
            Some(status) => NoneOr::Other(BuildStatus::of(status)),
            None => NoneOr::None,
        })
    }

    /// Block until the Bazel invocation finishes and return a `BuildStatus`.
    ///
    /// Every `execution_log.iterator()` and `workspace_events.iterator()`
    /// handle belongs **before** this call, not merely preferably. `wait()`
    /// releases every still-bound handle and then joins the streams, so a
    /// handle stops yielding from here on and anything it had not been drained
    /// of is gone.
    ///
    /// That is deliberate: each of those is a subscriber the producer blocks for,
    /// which is what makes the iteration complete, and releasing them is the only
    /// thing that lets a build nobody drained end. The drain therefore belongs
    /// before the call, not after.
    ///
    /// With a captured console, the `BazelTrait.output_intercept` hooks first
    /// answer every line it holds until it ends, then the console is joined,
    /// so Bazel's last lines are out before the task prints anything, and the
    /// `output_end` hooks are called with this build. A hook's error is raised
    /// once the build is joined.
    ///
    /// The wait ends the task instead if `ctx.cancellation.root` is
    /// cancelled meanwhile and no `intercept()` is in effect; the client has
    /// its SIGINT by then.
    fn wait<'v>(
        this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<BuildStatus> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        let console = TraitConsole::of(eval);
        build.answer_until_the_console_ends(console.as_ref(), this, eval);
        let status = build.wait_status().inspect_err(|_| {
            build.release_frames_screen();
        })?;
        if let Some(console) = &console
            && build.console_observations.is_some()
            && !build.console_ended.replace(true)
        {
            build.frame(console, this, eval, true);
            if let Err(e) = console.ended(this, eval) {
                build.hook_failed("end", e);
            }
        }
        match build.hook_error.take() {
            Some(e) => Err(e),
            None => Ok(status),
        }
    }
}

/// How often the `BazelTrait.output_frame` hooks run while a build's console
/// is open.
const FRAME_EVERY: Duration = Duration::from_millis(100);

/// Lines [`build_methods`]' `run_output_hooks` answers per call, so a burst
/// cannot monopolize a task's build loop.
const HOOK_BUDGET: usize = 1000;

/// How long `run_output_hooks` waits after an answer for the next held line:
/// lines are held one at a time, so the next of a burst comes only once the
/// one before is answered.
const HOOK_FOLLOW: Duration = Duration::from_millis(20);

/// What an AXL error says went wrong: its `error:` line (a traceback's
/// frames come first), else its last line.
fn cause(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    let line = text
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("error: "))
        .or_else(|| text.lines().map(str::trim).filter(|l| !l.is_empty()).last())
        .unwrap_or_default();
    line.to_string()
}

impl Build {
    /// Keep `e`, the first a console hook of `kind` (`frame`, `intercept`,
    /// `end`) raised, for `wait()`, and let go of the held lines: the rest
    /// are shown at once. No frame runs after it, so a screen the build's
    /// frame hooks took is given back ([`Build::release_frames_screen`]),
    /// saying so at once when one was and the build goes on: a console
    /// hidden for that screen stays hidden, and the error is raised only
    /// once the build ends.
    fn hook_failed(&self, kind: &str, e: anyhow::Error) {
        let released = self.release_frames_screen();
        if released && self.hook_error.borrow().is_none() && !self.console_done() {
            errln!(
                "WARNING: a console {kind} hook failed ({}); the build goes on without its screen, and the error follows when it ends.",
                cause(&e)
            );
        }
        drop(self.console_events.take());
        self.hook_error.borrow_mut().get_or_insert(e);
    }

    /// Answer every held line with `console`'s hooks until the console ends
    /// (the client exited and its stderr closed), or the task is cancelled,
    /// running its `output_frame` hooks meanwhile. A hook's error lets go of
    /// the rest ([`Build::hook_failed`]). With frame hooks and no held lines,
    /// it waits on the client between frames, so the client is reaped here
    /// rather than in [`Build::wait_status`]: the console's gate learns it
    /// is gone, and the profile is copied, then.
    fn answer_until_the_console_ends<'v>(
        &self,
        console: Option<&TraitConsole<'v>>,
        this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) {
        let Some(console) = console else {
            drop(self.console_events.take());
            return;
        };
        let framed = !console.frame.is_empty() && self.console.borrow().is_some();
        if framed && self.console_events.borrow().is_none() {
            while !self.console_done() {
                self.frame(console, this, eval, false);
                match self.wait_child(Some(FRAME_EVERY / 4)) {
                    Ok(None) => {}
                    Ok(Some(_)) => std::thread::sleep(FRAME_EVERY / 4),
                    // Cancelled: `wait()` reports it.
                    Err(_) => return,
                }
            }
            return;
        }
        let tick = framed.then_some(FRAME_EVERY);
        loop {
            let next = {
                let events = self.console_events.borrow();
                let Some(rx) = events.as_ref() else {
                    return;
                };
                children::recv_cancellable(&self.signals, rx, tick)
            };
            if framed {
                self.frame(console, this, eval, false);
            }
            match next {
                Recv::Item(held) => {
                    if let Err(e) = console.answer(held, eval) {
                        self.hook_failed("intercept", e);
                        return;
                    }
                }
                Recv::Tick => {}
                Recv::Closed | Recv::Cancelled => {
                    drop(self.console_events.take());
                    return;
                }
            }
        }
    }

    /// Whether the captured console has ended: its forwarder is done.
    fn console_done(&self) -> bool {
        self.console.borrow().as_ref().is_none_or(|f| f.finished())
    }

    /// Whether a frame is due: the task has frame hooks (as far as is known
    /// yet), the console is open and none ran within [`FRAME_EVERY`].
    fn frame_due(&self) -> bool {
        self.has_frames.get() != Some(false)
            && self.console.borrow().is_some()
            && !self.console_ended.get()
            && self
                .last_frame
                .get()
                .is_none_or(|t| t.elapsed() >= FRAME_EVERY)
    }

    /// Run the `output_frame` hooks with `this` build when one is due, or
    /// now with `force` (the last frame, as the console ends). After any
    /// console hook's error, `output_intercept`'s too, no frame runs: the
    /// error is kept for `wait()`, and the screen is given back at once
    /// ([`Build::release_frames_screen`]).
    fn frame<'v>(
        &self,
        console: &TraitConsole<'v>,
        this: values::Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
        force: bool,
    ) {
        self.has_frames.set(Some(!console.frame.is_empty()));
        if console.frame.is_empty() || self.hook_error.borrow().is_some() {
            return;
        }
        if !force && !self.frame_due() {
            return;
        }
        self.last_frame.set(Some(std::time::Instant::now()));
        if let Err(e) = console.framed(this, eval) {
            self.hook_failed("frame", e);
        }
    }

    /// No frame will run for this build any more (a hook failed, or the wait
    /// did): with frame hooks, give back the terminal a screen holds, which
    /// is process-wide, rather than leave it frozen until the CLI exits.
    /// Whether a screen was given back.
    fn release_frames_screen(&self) -> bool {
        self.has_frames.get() == Some(true) && crate::engine::release_terminal()
    }

    /// The client's token, the one `handle.cancellation` wraps.
    pub(super) fn cancellation_token(&self) -> &CancellationToken {
        self.bound.token()
    }

    /// The shared id every gRPC BES sink used for this build, `""` without
    /// sinks: what the `sink_invocation_id` attribute reads.
    pub(super) fn sink_invocation_id(&self) -> String {
        self.sink_invocation_id.borrow().clone().unwrap_or_default()
    }

    /// What the `wait()` method does: release every iterator handle, wait for
    /// the client, then join every stream. Idempotent only in that a second
    /// call after the first finds nothing left to join.
    pub(super) fn wait_status(&self) -> anyhow::Result<BuildStatus> {
        let build = self;

        // Re-enter the span so trace coverage includes the full build lifecycle
        let span = build.span.borrow().clone();
        let _enter = span.enter();

        // Release every handle first, so no exit below — a cancel, a failed
        // join — leaves a reader parked on one the task never drained. The task
        // cannot read a handle once it has called `wait()`.
        for iter in &build.workspace_event_iters {
            iter.release();
        }
        for iter in &build.execlog_iters {
            iter.release();
        }
        // Held console lines too: `wait()` answered them all, and lines held
        // from here on are shown at once rather than each waiting out its
        // timeout.
        drop(build.console_events.take());

        let result = build
            .wait_child(None)?
            .expect("a wait without a timeout reports a status");

        // First, so bazel's own last lines precede every warning below. A
        // console that failed to read is reported, not raised: the build's
        // status and its other streams matter more than its last lines.
        for console in [build.console.take(), build.console_stdout.take()] {
            if let Some(mut console) = console
                && let Err(err) = console.join()
            {
                crate::diag::warn(&format!("reading Bazel's console failed: {err}"));
            }
        }

        // Wait for BES stream to complete.
        let stream_error = match &mut *build.build_event_stream.borrow_mut() {
            Some(event_stream) => match event_stream.join() {
                Ok(_) => {
                    if let Some(warning) =
                        bazel::bes::warning::incomplete_events(event_stream.end(), result.success())
                    {
                        crate::diag::warn(&warning);
                    }
                    None
                }
                Err(err) => Some(err),
            },
            None => None,
        };
        // The sinks may still hold the stream's last events (`BuildMetrics`
        // for the trace sink; the logs for the keep sink, which writes its
        // index once the stream ends), so the task must not exit before they
        // are done, even after a failed join above. They never fail, and they
        // end once they have drained the closed stream.
        if let Some(trace_sink) = build.trace_sink.take() {
            let _ = trace_sink.join();
        }
        if let Some(keep_sink) = build.keep_sink.take() {
            let _ = keep_sink.join();
        }
        // Joins the keep sink itself when it holds it.
        if let Some(slot) = &build.history_slot {
            slot.summarize(false);
        }
        if let Some(err) = stream_error {
            anyhow::bail!("build event stream thread error: {}", err);
        }

        // Both logs are joined before either failure is reported, so one
        // failing does not leave the other's reader running. A failed BES join
        // above returns first; the handles are released by then, so those
        // readers end on their own.
        let mut stream_errors = Vec::new();
        if let Some(stream) = build.workspace_event_stream.take() {
            let path = stream.path().display().to_string();
            if let Err(err) = stream.join() {
                stream_errors.push(format!(
                    "reading the workspace event log '{path}' failed: {err}"
                ));
            }
        }

        // Wait for Execlog stream to complete.
        //
        // This is the one place that has both the reader's verdict and bazel's exit
        // code, which is what makes the warning safe to print: a log that never
        // appeared is ordinary on a failed invocation (a mistyped target pattern
        // never reaches the phase that creates one, and bazel has already said so)
        // and an anomaly on a successful one that something reads it for. A log
        // that stopped decoding is warned about either way, since Bazel says
        // nothing about it. See
        // `ReaderOutcome`.
        if let Some(execlog_stream) = build.execlog_stream.take() {
            let path = execlog_stream.path().display().to_string();
            match execlog_stream.join() {
                Ok(ReaderOutcome::NeverAppeared) if result.success() && build.execlog_read => {
                    crate::diag::warn(
                        "bazel completed successfully but wrote no execution log, so \
                         nothing that reads it saw this invocation.",
                    );
                }
                Ok(ReaderOutcome::Undecodable(reason)) => {
                    crate::diag::warn(&format!(
                        "the execution log '{path}' stopped decoding ({reason}), so its \
                         hooks, spawn telemetry and decoded sinks hold only the entries \
                         before that point. The build's result and copies of the compact \
                         log are unaffected."
                    ));
                }
                Ok(_) => {}
                Err(err) => {
                    stream_errors.push(format!("reading the execution log '{path}' failed: {err}"))
                }
            }
        };
        if !stream_errors.is_empty() {
            anyhow::bail!("{}", stream_errors.join("; "));
        }

        // Drop the span to end the trace
        drop(build.span.replace(tracing::Span::none()));

        Ok(BuildStatus::of(result))
    }
}

#[cfg(test)]
mod tests {
    //! End-to-end coverage of `ctx.bazel.build` via the `basil` fake-bazel
    //! binary, selected per-test via `--scenario=<name>`.

    use crate::engine::history::bazel_dir;

    #[test]
    fn a_hook_errors_cause_is_its_error_line() {
        let traceback = anyhow::anyhow!(
            "Traceback (most recent call last):\n  File <builtin>, in <module>\n  * a.axl:3, in _frame\n      fail(\"boom\")\nerror: fail: boom at frame 5\n --> a.axl:3:5"
        );
        assert_eq!(super::cause(&traceback), "fail: boom at frame 5");
        assert_eq!(super::cause(&anyhow::anyhow!("plain\nlast")), "last");
    }

    #[test]
    fn a_workspace_is_found_from_a_relative_or_symlinked_directory() {
        let ws = tempfile::tempdir().unwrap();
        let ws_root = ws.path().canonicalize().unwrap();
        std::fs::write(ws_root.join("MODULE.bazel"), "").unwrap();
        std::fs::create_dir_all(ws_root.join("e2e/pkg")).unwrap();
        std::fs::write(ws_root.join("e2e/MODULE.bazel"), "").unwrap();
        let pkg = ws_root.join("e2e/pkg");
        assert_eq!(
            super::workspace_of(pkg.to_str()),
            Some(ws_root.join("e2e")),
            "the nearest wins"
        );
        #[cfg(unix)]
        {
            let other = tempfile::tempdir().unwrap();
            let link = other.path().join("link");
            std::os::unix::fs::symlink(&pkg, &link).unwrap();
            assert_eq!(
                super::workspace_of(link.to_str()),
                Some(ws_root.join("e2e")),
                "found from where the link leads, as Bazel's client does"
            );
        }
    }

    use axl_proto::tools::protos::ExecLogEntry;
    use prost::Message;

    /// Serializes the tests that drive the execution log through basil.
    ///
    /// They set `BASIL_SERVER_PID`, which is process-wide, so running two at
    /// once would have one clobber the other's daemon stand-in and the loser
    /// would read a dead pid — which is a passing result for some of these tests
    /// and a failing one for others, i.e. flaky either way.
    static EXECLOG_BASIL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// How long the daemon stand-in lives.
    ///
    /// It has to outlast the longest timeout any test passes to
    /// [`run_basil_script`], because a timeout leaks the stand-in
    /// ([`DaemonStandIn::leak`]) and the build that outran the timeout keeps
    /// asking this pid whether more bytes are coming. A stand-in that died first
    /// would answer "the writer is gone" to a build still writing.
    #[cfg(unix)]
    const STAND_IN_LIFETIME: std::time::Duration = std::time::Duration::from_secs(300);

    /// A live process standing in for the Bazel daemon, plus the lock that
    /// serializes the tests using one. Torn down on drop, so a panicking body
    /// cannot leave `BASIL_SERVER_PID` naming a process it killed.
    ///
    /// The readers ask two pids whether more bytes are coming: the client,
    /// whose exit means the command is over, and the daemon, which holds the
    /// file open while writing it. basil's `info` invocation is a short-lived
    /// process, so the pid it reports is already reaped by the time a reader
    /// looks. Neither that pid nor the stand-in ever holds a file, so `held`
    /// answers the same for both, and the client's exit is what ends the read.
    /// The stand-in keeps the server pid a live process, as a real daemon's
    /// is, for the tests whose subject is a daemon that outlives the client.
    #[cfg(unix)]
    struct DaemonStandIn {
        /// `None` once [`leak`](Self::leak)ed, which is what leaves the stand-in
        /// running past this guard.
        daemon: Option<std::process::Child>,
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    #[cfg(unix)]
    impl DaemonStandIn {
        fn new() -> Self {
            let _serial = EXECLOG_BASIL.lock().unwrap_or_else(|e| e.into_inner());
            let daemon = std::process::Command::new("/bin/sleep")
                .arg(STAND_IN_LIFETIME.as_secs().to_string())
                .spawn()
                .expect("spawn the daemon stand-in");
            // SAFETY: process-wide env mutation, serialized against the other
            // execlog-through-basil tests by `EXECLOG_BASIL`. A concurrent basil
            // test with `build_events` also reads this pid as the BES file's
            // holder; like basil's own reaped pid it holds nothing, so its
            // answer does not change.
            unsafe {
                std::env::set_var("BASIL_SERVER_PID", daemon.id().to_string());
            }
            Self {
                daemon: Some(daemon),
                _serial,
            }
        }

        /// Leave the stand-in process running, for a body that has not finished.
        ///
        /// A timed-out `crate::test::with_timeout` leaves its worker thread inside
        /// a live build still asking this pid whether more bytes are coming.
        /// Killing the stand-in would answer "the writer is gone" to a build that
        /// is still writing, turning one hang into a run of unrelated failures.
        /// Dropping the `Child` without killing it keeps that worker's answers
        /// valid and costs only a `sleep` that exits on its own.
        ///
        /// It does not hold `EXECLOG_BASIL` past this call: `self` drops here,
        /// `Drop` early-returns on the missing child, and the guard is released —
        /// so the next execlog test runs and re-points `BASIL_SERVER_PID` at its
        /// own stand-in. Harmless, because the stranded worker captured the pid it
        /// is asking about when its build spawned.
        fn leak(mut self) {
            self.daemon = None;
        }
    }

    #[cfg(unix)]
    impl Drop for DaemonStandIn {
        fn drop(&mut self) {
            let Some(daemon) = self.daemon.as_mut() else {
                return;
            };
            let _ = daemon.kill();
            let _ = daemon.wait();
            // SAFETY: as in `new`.
            unsafe {
                std::env::remove_var("BASIL_SERVER_PID");
            }
        }
    }

    /// Run `body` under a daemon stand-in. For a body that cannot hang; one that
    /// can belongs in [`run_basil_script`].
    #[cfg(unix)]
    fn with_daemon_stand_in<T>(body: impl FnOnce() -> T) -> T {
        let _stand_in = DaemonStandIn::new();
        body()
    }

    /// Run `script` as a task under a daemon stand-in, bounded by `timeout`, and
    /// return what `run_task` reported.
    ///
    /// Every log test below fails by parking rather than by answering
    /// wrongly, so each needs a timeout, and a timeout fires while the work is
    /// still running — hence [`DaemonStandIn::leak`] rather than an ordinary
    /// teardown. `hang` explains the timeout.
    #[cfg(unix)]
    fn run_basil_script(
        timeout: std::time::Duration,
        script: String,
        hang: &str,
    ) -> anyhow::Result<Option<u8>> {
        run_basil_script_with(timeout, script, hang, |eval| eval)
    }

    /// [`run_basil_script`], with the run set up further by `configure`: every
    /// signal exported, so builds attach their telemetry sinks, or a history
    /// root a test inspects.
    #[cfg(unix)]
    fn run_basil_script_with(
        timeout: std::time::Duration,
        script: String,
        hang: &str,
        configure: impl FnOnce(crate::test::EvalBuilder) -> crate::test::EvalBuilder + Send + 'static,
    ) -> anyhow::Result<Option<u8>> {
        assert!(
            timeout < STAND_IN_LIFETIME,
            "the daemon stand-in ({STAND_IN_LIFETIME:?}) must outlast a leaked \
             test's timeout ({timeout:?}); see STAND_IN_LIFETIME",
        );
        let stand_in = DaemonStandIn::new();
        let result = crate::test::with_timeout(timeout, move || {
            configure(crate::test::eval(&script).with_fake_bazel()).run_task(0)
        });
        match result {
            Some(exit) => exit,
            None => {
                stand_in.leak();
                panic!("timed out after {timeout:?}: {hang}");
            }
        }
    }

    /// [`run_basil_script`], asserting the task exited 0. `wrong` explains a
    /// task that ran and returned nonzero, which is the scripts' own `return 2` /
    /// `return 3` arms.
    #[cfg(unix)]
    fn run_basil_task(timeout: std::time::Duration, script: String, hang: &str, wrong: &str) {
        let exit = run_basil_script(timeout, script, hang).expect("run_task");
        assert_eq!(exit, Some(0), "{wrong}");
    }

    /// How many entries or events basil's log scenarios write: its
    /// `BEYOND_CAPACITY` (crates/basil/src/main.rs), which this must match.
    /// Longer than `CHANNEL_CAPACITY`, the only length that can tell a stream
    /// that delivers everything apart from one that stops at the ring.
    #[cfg(unix)]
    const SCENARIO_ENTRIES: u32 = 3000;

    /// An `iterator()` handle must receive the whole log.
    ///
    /// The handle's subscriber is cloned from the stream's own, which is
    /// registered when the stream spawns, before Bazel writes anything, and the
    /// producer blocks for it: every entry arrives, contiguously, and how fast
    /// the caller reads decides only how long the build's `wait()` takes.
    ///
    /// The assertion is contiguity and the exact count, not just "more than the
    /// capacity". A short prefix is the shape the failure takes, and in a real log,
    /// whose leading records are inputs rather than spawns, a prefix can contain no
    /// spawns at all — a confidently wrong answer rather than an obviously short
    /// one. The timeout is because a blocking producer fails by parking.
    #[cfg(unix)]
    #[test]
    fn an_iterator_handle_delivers_the_whole_log() {
        use std::time::Duration;

        let capacity = bazel::subscription::CHANNEL_CAPACITY as u32;
        assert!(
            SCENARIO_ENTRIES > capacity,
            "the scenario's log must outrun the ring ({capacity}) to be worth asserting on",
        );

        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("report.txt");
        // `with_timeout` runs the body on a thread that may outlive this frame,
        // so the script is rendered here rather than borrowing the path inside it.
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    for entry in entries:
        seen += 1
        if entry.id != seen:
            # basil numbers its entries 1..N, so anything else is a gap, a
            # replay or a reorder.
            return 2
    status = build.wait()
    if not status.success: return 1
    ctx.std.fs.create({report:?}).write("%d\n" % seen)
    return 0

Test = task(implementation = _impl)
"#,
            report = report.to_str().unwrap(),
        );

        run_basil_task(
            Duration::from_secs(60),
            script,
            "the execution log's producer parked",
            "every entry must arrive, with contiguous ids",
        );

        let text = std::fs::read_to_string(&report).expect("the task should have reported");
        let seen: u32 = text.trim().parse().unwrap();
        assert_eq!(
            seen, SCENARIO_ENTRIES,
            "an iterator handle must be lossless; a count short of the log is the \
             prefix a dropping producer leaves, and one short by a varying amount is \
             the race it leaves",
        );
    }

    /// Under the default `auto` policy, with traces exported, a build that
    /// writes the log gets the spawn telemetry sink, a subscriber the producer
    /// blocks for, so it has to keep draining: one that stopped would park the
    /// reader at the ring and stall the iterator beside it, which the timeout
    /// catches. The iterator counting every entry is what shows the two shared
    /// the stream.
    #[cfg(unix)]
    #[test]
    fn the_spawn_telemetry_sink_drains_the_log_beside_an_iterator() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("report.txt");
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    for _ in entries:
        seen += 1
    status = build.wait()
    if not status.success: return 1
    ctx.std.fs.create({report:?}).write("%d\n" % seen)
    return 0

Test = task(implementation = _impl)
"#,
            report = report.to_str().unwrap(),
        );

        let exit = run_basil_script_with(
            Duration::from_secs(60),
            script,
            "the spawn telemetry sink stopped draining and parked the reader",
            |eval| eval.with_exporting(),
        )
        .expect("run_task");
        assert_eq!(
            exit,
            Some(0),
            "a build with spawn telemetry must still end cleanly"
        );

        let text = std::fs::read_to_string(&report).expect("the task should have reported");
        assert_eq!(text.trim().parse::<u32>().unwrap(), SCENARIO_ENTRIES);
    }

    /// `execution_log = []`: the log is on with no consumer.
    ///
    /// The stream drops its own subscriber once consumers are bound, so with none
    /// the first `send` fails and the reader drains the remaining bytes without
    /// decoding them. The tests below cover the subscribers handles hold, which
    /// `wait()` releases itself.
    ///
    /// The timeout is the whole assertion; a regression here is a hang.
    #[cfg(unix)]
    #[test]
    fn a_log_nobody_iterates_does_not_hang_the_build() {
        use std::time::Duration;

        run_basil_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [],
        stderr = None,
    )
    status = build.wait()
    if not status.success: return 1
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "an execution log nobody iterated parked the reader past the end of the build",
            "a build whose log nobody reads must still end cleanly",
        );
    }

    /// An iteration that stops early must not hold the build open.
    ///
    /// A `break` leaves the handle's subscriber in place, undrained, with the
    /// reader parked against it on a full ring, and `ExecLogStream::join()`
    /// cannot reach a subscriber the stream never owned. `wait()` releasing the
    /// handle is what makes the loop below one a task is allowed to write.
    ///
    /// Ten entries of the scenario's log, so the break lands while the producer is
    /// still writing rather than after it has finished.
    #[cfg(unix)]
    #[test]
    fn an_abandoned_handle_iteration_does_not_hang_the_build() {
        use std::time::Duration;

        run_basil_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    for entry in entries:
        seen += 1
        if seen == 10:
            break
    status = build.wait()
    if not status.success: return 1
    if seen != 10: return 2
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "an abandoned handle iteration parked the reader past the \
             end of the build",
            "a task that stops iterating early must still reach the end of its build",
        );
    }

    /// The same, for a handle the task never iterated at all.
    ///
    /// Here the subscriber sits at entry zero, so the reader parks from the
    /// producer's first thousand entries onwards, before the task does anything
    /// else. `wait()` releasing every still-bound handle is what ends it.
    #[cfg(unix)]
    #[test]
    fn an_undrained_iterator_handle_does_not_hang_the_build() {
        use std::time::Duration;

        run_basil_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries],
        stderr = None,
    )
    status = build.wait()
    if not status.success: return 1
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "an undrained `iterator()` handle parked the reader past the end of the build",
            "an undrained handle must still end the build",
        );
    }

    /// Two `workspace_events.iterator()` handles on one build, over a log longer
    /// than the channel capacity: both see every event, in order. The stream
    /// drops its own subscriber once they are bound, so neither is capped at
    /// the ring by a subscriber nobody reads.
    ///
    /// The producer waits for the slower handle, so the task takes turns, one
    /// event from each: draining one to the end first would park it against
    /// the other at the capacity. Leaving a `for` loop does not unsubscribe.
    #[cfg(unix)]
    #[test]
    fn two_workspace_event_handles_both_receive_the_whole_log() {
        use std::time::Duration;

        let script = format!(
            r#"
def _impl(ctx):
    handles = [bazel.workspace_events.iterator(), bazel.workspace_events.iterator()]
    build = ctx.bazel.build(
        flags = ["--scenario=workspace_events_beyond_capacity"],
        workspace_events = handles,
        stderr = None,
    )
    for n in range(1, {SCENARIO_ENTRIES} + 1):
        for handle in handles:
            got = None
            for event in handle:
                got = event.location
                break
            if got != str(n): return 2
    for handle in handles:
        for _ in handle:
            return 3
    status = build.wait()
    if not status.success: return 1
    return 0

Test = task(implementation = _impl)
"#
        );
        run_basil_task(
            Duration::from_secs(60),
            script,
            "the workspace event reader parked",
            &format!("each handle must see all {SCENARIO_ENTRIES} events, in order"),
        );
    }

    /// A workspace event handle the task never iterated must not hold the build
    /// open: `wait()` releases it before joining the reader.
    #[cfg(unix)]
    #[test]
    fn an_undrained_workspace_event_handle_does_not_hang_the_build() {
        use std::time::Duration;

        run_basil_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=workspace_events_beyond_capacity"],
        workspace_events = [bazel.workspace_events.iterator()],
        stderr = None,
    )
    if not build.wait().success: return 1
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "an undrained workspace event handle parked the reader past the end of the build",
            "an undrained handle must still end the build",
        );
    }

    /// A handle still bound to a running build, passed to a second one: the
    /// second spawn fails naming why, and must not release the handle, so the
    /// first build's iteration still sees its whole log rather than ending
    /// early as if the stream had.
    #[cfg(unix)]
    fn a_handle_reused_while_bound_still_serves_its_build(
        scenario: &str,
        handle: &str,
        arg: &str,
        check: &str,
    ) {
        let script = format!(
            r#"
def _impl(ctx):
    events = {handle}
    first = ctx.bazel.build(
        flags = ["--scenario={scenario}"],
        {arg} = [events],
        stderr = None,
    )
    err, _ = catch(
        ctx.bazel.build,
        flags = ["--scenario={scenario}"],
        {arg} = [events],
        stderr = None,
    )
    if not err: return 2
    if "already bound" not in err.message: return 3
    seen = 0
    for event in events:
        seen += 1
        if {check} != str(seen): return 4
    if seen != {SCENARIO_ENTRIES}: return 5
    if not first.wait().success: return 1
    return 0

Test = task(implementation = _impl)
"#
        );
        run_basil_task(
            std::time::Duration::from_secs(60),
            script,
            "the first build's reader parked",
            "the second spawn must refuse the handle without releasing it",
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_workspace_event_handle_reused_while_bound_still_serves_its_build() {
        a_handle_reused_while_bound_still_serves_its_build(
            "workspace_events_beyond_capacity",
            "bazel.workspace_events.iterator()",
            "workspace_events",
            "event.location",
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_execution_log_handle_reused_while_bound_still_serves_its_build() {
        a_handle_reused_while_bound_still_serves_its_build(
            "execlog_beyond_capacity",
            "bazel.execution_log.iterator()",
            "execution_log",
            "str(event.id)",
        );
    }

    /// The workspace log stays in the Bazel invocation's history folder, and a
    /// path already asked for on the command line is written as a copy of it,
    /// so an earlier run's log left at that path reaches neither the handle
    /// nor the copy.
    #[cfg(unix)]
    #[test]
    fn a_requested_workspace_log_is_a_copy_of_the_one_in_history() {
        use prost::Message;

        let history = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("asked.log");
        let stale: Vec<u8> = (0..5)
            .flat_map(|_| {
                axl_proto::workspace_log::WorkspaceEvent {
                    location: "stale".into(),
                    ..Default::default()
                }
                .encode_length_delimited_to_vec()
            })
            .collect();
        std::fs::write(&asked, stale).unwrap();
        let script = format!(
            r#"
def _impl(ctx):
    events = bazel.workspace_events.iterator()
    build = ctx.bazel.build(
        flags = [
            "--scenario=workspace_events_beyond_capacity",
            "--experimental_workspace_rules_log_file={asked}",
        ],
        workspace_events = [events],
        stderr = None,
    )
    seen = 0
    for event in events:
        seen += 1
        if event.location != str(seen): return 3
    if not build.wait().success: return 1
    if seen != {SCENARIO_ENTRIES}: return 2
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );
        run_task_with_history(&script, history.path());
        let kept =
            bazel_dir(&only_invocation(history.path()), 1).join(bazel::workspace_event::FILE_NAME);
        let kept = std::fs::read(&kept).unwrap_or_else(|e| panic!("{}: {e}", kept.display()));
        assert!(!kept.is_empty());
        assert_eq!(
            std::fs::read(&asked).unwrap(),
            kept,
            "the copy is the log, whole"
        );
    }

    /// A build asked to keep logs keeps the ones it was asked for in its
    /// history folder, with an index, though nothing else read its events.
    #[cfg(unix)]
    #[test]
    fn a_build_keeps_the_logs_it_was_asked_for_in_history() {
        let history = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let script = r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=action_and_test_logs", "--basil-scratch=SCRATCH"],
        keep_action_logs = "failed",
        keep_test_logs = "all",
        stderr = None,
    )
    build.wait()
    return 0

Test = task(implementation = _impl)
"#
        .replace("SCRATCH", scratch.path().to_str().unwrap());
        run_task_with_history(&script, history.path());
        let dir = bazel_dir(&only_invocation(history.path()), 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("actions/pkg/broken/Genrule-1.stderr")).unwrap(),
            "ERROR: broken\n"
        );
        assert!(
            !dir.join("actions/pkg/fine").exists(),
            "a success is not kept under `failed`"
        );
        let index: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("actions.json")).unwrap()).unwrap();
        assert_eq!(index["actions"][0]["label"], "//pkg:broken");
        for (label, text) in [("fails", "FAIL\n"), ("passes", "PASS\n")] {
            let log = dir.join(format!(
                "testlogs/pkg/{label}/run_1/shard_1/attempt_1/test.log"
            ));
            assert_eq!(std::fs::read_to_string(&log).unwrap(), text, "{label}");
        }
        let args = std::fs::read_to_string(scratch.path().join("args")).unwrap();
        assert!(
            !args.contains("--build_event_publish_all_actions"),
            "`failed` needs Bazel's default publishing"
        );
    }

    /// The task's record notes each build: its command, its exit, and the
    /// summary of its stream, whose failures point at the logs kept for them.
    #[cfg(unix)]
    #[test]
    fn the_task_record_notes_each_build_and_its_failures() {
        let history = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let script = r#"
def _impl(ctx):
    build = ctx.bazel.build(
        "//pkg:all",
        flags = ["--scenario=action_and_test_logs", "--basil-scratch=SCRATCH", "--remote_header=x-api-key=secret"],
        keep_action_logs = "failed",
        keep_test_logs = "failed",
        stderr = None,
    )
    build.wait()
    return 0

Test = task(implementation = _impl)
"#
        .replace("SCRATCH", scratch.path().to_str().unwrap());
        run_task_with_history(&script, history.path());
        let entry = only_invocation(history.path());
        let record = crate::engine::task_record::read(&entry.join("task.json")).unwrap();
        assert_eq!(
            (record.status.as_str(), record.exit_code),
            ("ended", Some(0))
        );
        let build = &record.bazel[0];
        assert_eq!(
            (
                build.command.as_str(),
                build.patterns.as_slice(),
                build.folder.as_deref()
            ),
            ("build", &["//pkg:all".to_string()][..], Some("bazel/1"))
        );
        assert!(
            build.ended.is_some() && build.exit_code.is_some(),
            "{build:?}"
        );
        assert!(
            build
                .flags
                .iter()
                .any(|f| f == "--remote_header=x-api-key=<REDACTED>"),
            "{:?}",
            build.flags
        );
        let summary = build.summary.as_ref().expect("a summary");
        let action = summary
            .failures
            .iter()
            .find(|f| f.kind == "action" && f.label == "//pkg:broken")
            .unwrap_or_else(|| panic!("{summary:?}"));
        assert_eq!(action.logs, ["bazel/1/actions/pkg/broken/Genrule-1.stderr"]);
        let test = summary
            .failures
            .iter()
            .find(|f| f.kind == "test" && f.label == "//pkg:fails")
            .unwrap_or_else(|| panic!("{summary:?}"));
        assert_eq!(
            test.logs.first().map(String::as_str),
            Some("bazel/1/testlogs/pkg/fails/run_1/shard_1/attempt_1/test.log")
        );
        for log in action.logs.iter().chain(&test.logs) {
            assert!(entry.join(log).exists(), "{log}");
        }
    }

    /// Keeping every action's logs runs the build with
    /// `--build_event_publish_all_actions`, and keeps a successful action's.
    #[cfg(unix)]
    #[test]
    fn keeping_every_actions_logs_asks_bazel_for_every_action() {
        let history = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let script = r#"
def _impl(ctx):
    ctx.bazel.build(
        flags = ["--scenario=action_and_test_logs", "--basil-scratch=SCRATCH"],
        keep_action_logs = "all",
        stderr = None,
    ).wait()
    return 0

Test = task(implementation = _impl)
"#
        .replace("SCRATCH", scratch.path().to_str().unwrap());
        run_task_with_history(&script, history.path());
        let args = std::fs::read_to_string(scratch.path().join("args")).unwrap();
        assert!(
            args.lines()
                .any(|a| a == "--build_event_publish_all_actions"),
            "{args}"
        );
        let dir = bazel_dir(&only_invocation(history.path()), 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("actions/pkg/fine/Genrule-1.stdout")).unwrap(),
            "fine\n"
        );
        assert!(!dir.join("testlogs").exists(), "test logs not asked for");
    }

    /// The actions a BEP file reports, as `(label, success)`.
    #[cfg(unix)]
    fn bep_file_actions(path: &std::path::Path) -> Vec<(String, bool)> {
        use axl_proto::build_event_stream::build_event::Payload;
        use axl_proto::build_event_stream::build_event_id::Id;
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("the BEP file at {} should exist: {e}", path.display()));
        let mut buf = bytes.as_slice();
        let mut actions = vec![];
        while !buf.is_empty() {
            let event =
                axl_proto::build_event_stream::BuildEvent::decode_length_delimited(&mut buf)
                    .expect("the BEP file should hold whole events");
            if let (Some(Payload::Action(a)), Some(Id::ActionCompleted(id))) = (
                &event.payload,
                event.id.as_ref().and_then(|id| id.id.as_ref()),
            ) {
                actions.push((id.label.clone(), a.success));
            }
        }
        actions
    }

    /// Run a build of the `action_and_test_logs` scenario under an rc of
    /// `rc` the CLI renders, with `extra` flags and `args` to `ctx.bazel.build`,
    /// writing a requested BEP file and an AXL file sink, and asserting an
    /// iterator saw both actions. Returns the actions the file sinks got, those
    /// the history's copy holds, and the command line.
    #[cfg(unix)]
    fn every_action_downstream(
        rc: &str,
        extra: &[&str],
        args: &str,
    ) -> (Vec<(String, bool)>, Vec<(String, bool)>, String) {
        let history = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let rc_file = out.path().join("test.bazelrc");
        std::fs::write(&rc_file, rc).unwrap();
        let asked = out.path().join("asked.bep");
        let sink = out.path().join("sink.bep");
        let flags: Vec<String> = [
            "--scenario=action_and_test_logs".to_string(),
            format!("--basil-scratch={}", scratch.path().display()),
            format!("--build_event_binary_file={}", asked.display()),
        ]
        .into_iter()
        .chain(extra.iter().map(|f| f.to_string()))
        .collect();
        let script = format!(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    rc = ctx.bazel.parse_rc(startup_flags = ["--nosystem_rc", "--nohome_rc", "--noworkspace_rc", "--bazelrc={rc_file}"], flags = [])
    build = ctx.bazel.build(
        rc = rc,
        flags = {flags:?},
        build_events = [iter, bazel.build_events.file(path = {sink:?})],
        stderr = None,
        {args}
    )
    seen = len([e for e in iter if e.kind == "action_completed"])
    build.wait()
    return 0 if seen == 2 else 1

Test = task(implementation = _impl)
"#,
            sink = sink.to_str().unwrap(),
            rc_file = rc_file.display(),
        );
        run_task_with_history(&script, history.path());
        let kept =
            bazel_dir(&only_invocation(history.path()), 1).join(bazel::bes::binary_file::FILE_NAME);
        let args = std::fs::read_to_string(scratch.path().join("args")).unwrap();
        let asked = bep_file_actions(&asked);
        assert_eq!(asked, bep_file_actions(&sink), "both file sinks agree");
        (asked, bep_file_actions(&kept), args)
    }

    #[cfg(unix)]
    fn broken() -> (String, bool) {
        ("//pkg:broken".to_string(), false)
    }

    #[cfg(unix)]
    fn fine() -> (String, bool) {
        ("//pkg:fine".to_string(), true)
    }

    /// Every action the CLI asked Bazel for reaches its own readers, the
    /// history's copy and an iterator, but not a requested BEP file or a file
    /// sink: those get what Bazel reports without the flag.
    #[cfg(unix)]
    #[test]
    fn the_actions_the_cli_asked_for_stay_with_its_own_readers() {
        let (downstream, kept, args) =
            every_action_downstream("", &[], r#"keep_action_logs = "all","#);
        assert!(
            args.lines()
                .any(|a| a == "--build_event_publish_all_actions"),
            "{args}"
        );
        assert_eq!(downstream, [broken()]);
        assert_eq!(kept, [broken(), fine()]);
    }

    /// The same, asked for through `publish_all_actions`, with no logs kept.
    #[cfg(unix)]
    #[test]
    fn publish_all_actions_asks_bazel_for_every_action_for_the_cli_alone() {
        let (downstream, kept, args) =
            every_action_downstream("", &[], "publish_all_actions = True,");
        assert!(
            args.lines()
                .any(|a| a == "--build_event_publish_all_actions"),
            "{args}"
        );
        assert_eq!(downstream, [broken()]);
        assert_eq!(kept, [broken(), fine()]);
    }

    /// A feature asks through the task's flags: the request is taken off the
    /// command line, Bazel gets the flag, and downstream is trimmed.
    #[cfg(unix)]
    #[test]
    fn a_feature_request_in_the_flags_is_the_clis() {
        let (downstream, kept, args) =
            every_action_downstream("", &["--build_event_publish_all_actions=%aspect_cli%"], "");
        assert!(!args.contains("%aspect_cli%"), "{args}");
        assert!(
            args.lines()
                .any(|a| a == "--build_event_publish_all_actions"),
            "{args}"
        );
        assert_eq!(downstream, [broken()]);
        assert_eq!(kept, [broken(), fine()]);
    }

    /// `ctx.bazel.test` takes `publish_all_actions` too.
    #[cfg(unix)]
    #[test]
    fn a_test_asks_bazel_for_every_action_too() {
        let scratch = tempfile::tempdir().unwrap();
        let script = format!(
            r#"
def _impl(ctx):
    ctx.bazel.test(
        flags = ["--scenario=success", "--basil-scratch={scratch}"],
        build_events = True,
        publish_all_actions = True,
        stderr = None,
    ).wait()
    return 0

Test = task(implementation = _impl)
"#,
            scratch = scratch.path().display(),
        );
        let history = tempfile::tempdir().unwrap();
        run_task_with_history(&script, history.path());
        let args = std::fs::read_to_string(scratch.path().join("args")).unwrap();
        assert!(
            args.lines()
                .any(|a| a == "--build_event_publish_all_actions"),
            "{args}"
        );
    }

    /// A caller asking for every action itself gets every action downstream,
    /// and the flag is not given twice.
    #[cfg(unix)]
    #[test]
    fn a_caller_asking_for_every_action_gets_them_downstream() {
        let (downstream, kept, args) = every_action_downstream(
            "",
            &["--build_event_publish_all_actions"],
            r#"keep_action_logs = "all","#,
        );
        assert_eq!(
            args.lines()
                .filter(|a| *a == "--build_event_publish_all_actions")
                .count(),
            1,
            "{args}"
        );
        assert_eq!(downstream, [broken(), fine()]);
        assert_eq!(kept, [broken(), fine()]);
    }

    /// An rc asking for every action counts as the caller's ask.
    #[cfg(unix)]
    #[test]
    fn an_rc_asking_for_every_action_gets_them_downstream() {
        let (downstream, _, _) = every_action_downstream(
            "build --build_event_publish_all_actions\n",
            &[],
            r#"keep_action_logs = "all","#,
        );
        assert_eq!(downstream, [broken(), fine()]);
    }

    /// Without an rc the CLI rendered, Bazel reads one the CLI cannot see,
    /// which may ask for every action, so nothing is trimmed.
    #[cfg(unix)]
    #[test]
    fn an_unseen_rc_leaves_downstream_untrimmed() {
        let scratch = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let asked = out.path().join("asked.bep");
        let script = format!(
            r#"
def _impl(ctx):
    ctx.bazel.build(
        flags = ["--scenario=action_and_test_logs", "--basil-scratch={scratch}", "--build_event_binary_file={asked}"],
        keep_action_logs = "all",
        stderr = None,
    ).wait()
    return 0

Test = task(implementation = _impl)
"#,
            scratch = scratch.path().display(),
            asked = asked.display(),
        );
        let history = tempfile::tempdir().unwrap();
        run_task_with_history(&script, history.path());
        assert_eq!(bep_file_actions(&asked), [broken(), fine()]);
    }

    #[test]
    fn an_unknown_keep_value_is_an_error() {
        let err = crate::test::eval(
            r#"
def _impl(ctx):
    ctx.bazel.build(flags = ["--scenario=success"], keep_test_logs = "some")
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect_err("an unknown value");
        assert!(format!("{err:#}").contains("keep_test_logs"), "{err:#}");
    }

    /// A build whose events the CLI reads keeps them in its history folder, as
    /// the stream's sinks get them, and a requested BEP file holds the same.
    #[cfg(unix)]
    #[test]
    fn a_build_events_stream_is_kept_in_history() {
        let history = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("ide.bep");
        let script = format!(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=success", "--build_event_binary_file={asked}"],
        build_events = [iter],
        stderr = None,
    )
    seen = len([e for e in iter])
    if not build.wait().success: return 1
    if seen != 2: return 2
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );
        run_task_with_history(&script, history.path());
        let kept =
            bazel_dir(&only_invocation(history.path()), 1).join(bazel::bes::binary_file::FILE_NAME);
        assert_eq!(bep_file_kinds(&kept), ["started", "finished"]);
        assert_eq!(
            std::fs::read(&asked).unwrap(),
            std::fs::read(&kept).unwrap()
        );
    }

    /// A build's stream kept in the history reads back as the events it
    /// streamed: found through the run's record, then `bazel.build_events.read`,
    /// whole and complete; `kinds=` filters it.
    #[cfg(unix)]
    #[test]
    fn a_kept_stream_reads_back_through_the_history() {
        let history = tempfile::tempdir().unwrap();
        let script = r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(flags = ["--scenario=success"], build_events = [iter], stderr = None)
    live = [type(e.payload) for e in iter]
    if not build.wait().success: return 1
    record = ctx.std.history.get(ctx.task.id)
    if record["outcome"] != "running": return 2
    if [r["task"]["id"] for r in ctx.std.history.list(outcome = "running")] != [ctx.task.id]: return 3
    if ctx.std.history.list(outcome = "ok"): return 4
    path = record["bazel"][0]["build_events"]
    stored = bazel.build_events.read(path)
    kept = [type(e.payload) for e in stored]
    if kept != live or not stored.complete or stored.error != None: return 5
    started = [type(e.payload) for e in bazel.build_events.read(path, kinds = ["started"])]
    if started != [live[0]] or len(started) != 1: return 6
    return 0

Test = task(implementation = _impl)
"#;
        run_task_with_history(script, history.path());
    }

    /// A build names the history's copy of its stream, which another
    /// process could follow as it is written: polled after the build, it
    /// has every event, and then nothing more.
    #[cfg(unix)]
    #[test]
    fn a_builds_kept_stream_can_be_followed() {
        let history = tempfile::tempdir().unwrap();
        let script = r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(flags = ["--scenario=success"], build_events = [iter], stderr = None)
    path = build.build_events_file
    if not path or not path.endswith("/build_events.binpb"): return 1
    followed = bazel.build_events.follow(path)
    live = [type(e.payload) for e in iter]
    if not build.wait().success: return 2
    got = [type(e.payload) for e in followed.poll()]
    if got != live or not followed.complete or followed.error != None: return 3
    if followed.poll(): return 4
    if bazel.build_events.follow(path + ".missing").poll(): return 6
    return 0

Test = task(implementation = _impl)
"#;
        run_task_with_history(script, history.path());
    }

    /// A child's output, polled line by line as it comes, never waiting
    /// unless asked, until it ends.
    #[cfg(unix)]
    #[test]
    fn a_childs_output_can_be_polled() {
        let script = r#"
def _impl(ctx):
    child = ctx.std.process.command("/bin/sh").args(["-c", "printf 'one\r\ntwo\nthree'"]).stdout("piped").spawn()
    lines = child.stdout().poller()
    got = []
    for _ in range(200):
        got += lines.poll(timeout_ms = 50)
        if lines.ended:
            break
    child.wait()
    if got != ["one", "two", "three"]: return 1
    if lines.poll() != [] or not lines.ended: return 2
    return 0

Test = task(implementation = _impl)
"#;
        let exit = crate::test::eval(script).run_task(0).expect("run_task");
        assert_eq!(exit, Some(0));
    }

    #[test]
    fn an_unknown_outcome_is_an_error() {
        let err = crate::test::eval(
            r#"
def _impl(ctx):
    ctx.std.history.list(outcome = "fine")
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect_err("an unknown outcome");
        assert!(
            format!("{err:#}").contains("the outcomes are ok, failed"),
            "{err:#}"
        );
    }

    /// Every build's profile stays in its history folder, and a `--profile`
    /// the caller asked for gets a copy once the client exits, in the format
    /// its name asks for.
    #[cfg(unix)]
    #[test]
    fn the_profile_stays_in_history_and_a_requested_one_is_a_copy() {
        let history = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("asked.profile.gz");
        let json = dir.path().join("asked.json");
        let script = format!(
            r#"
def _impl(ctx):
    for flags in [[], ["--profile={asked}"], ["--profile={json}"]]:
        build = ctx.bazel.build(rc = ctx.bazel.parse_rc(startup_flags = ["--ignore_all_rc_files"], flags = []), flags = flags, stderr = None)
        if not build.wait().success: return 1
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
            json = json.to_str().unwrap(),
        );
        run_task_with_history(&script, history.path());
        let invocation = only_invocation(history.path());
        let uncompressed = bazel_dir(&invocation, 3).join(bazel::profile::JSON_FILE_NAME);
        assert!(
            uncompressed.exists() && json.exists(),
            "a path not ending in .gz asks Bazel for plain JSON: {}",
            uncompressed.display()
        );
        for n in [1, 2] {
            let kept = bazel_dir(&invocation, n).join(bazel::profile::FILE_NAME);
            assert!(kept.exists(), "{}", kept.display());
        }
        assert_eq!(
            std::fs::read(&asked).unwrap(),
            std::fs::read(bazel_dir(&invocation, 2).join(bazel::profile::FILE_NAME)).unwrap(),
        );
    }

    /// A history that cannot be created fails no build: the logs are read
    /// from the temp dir, a requested workspace log is still copied, and the
    /// caller's `--profile` is left to Bazel.
    #[cfg(unix)]
    #[test]
    fn a_history_that_cannot_be_created_fails_no_build() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("not-a-dir");
        std::fs::write(&root, b"").unwrap();
        let (asked_log, asked_profile) = (dir.path().join("ws.log"), dir.path().join("p.gz"));
        let script = format!(
            r#"
def _impl(ctx):
    rc = ctx.bazel.parse_rc(startup_flags = ["--ignore_all_rc_files"], flags = [])
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        rc = rc,
        flags = ["--scenario=execlog_beyond_capacity", "--profile={asked_profile}"],
        execution_log = [entries],
        stderr = None,
    )
    seen = len([e for e in entries])
    if not build.wait().success: return 1
    if seen != {SCENARIO_ENTRIES}: return 2
    events = bazel.workspace_events.iterator()
    build = ctx.bazel.build(
        rc = rc,
        flags = [
            "--scenario=workspace_events_beyond_capacity",
            "--experimental_workspace_rules_log_file={asked_log}",
        ],
        workspace_events = [events],
        stderr = None,
    )
    seen = len([e for e in events])
    if not build.wait().success: return 3
    if seen != {SCENARIO_ENTRIES}: return 4
    return 0

Test = task(implementation = _impl)
"#,
            asked_profile = asked_profile.to_str().unwrap(),
            asked_log = asked_log.to_str().unwrap(),
        );
        run_task_with_history(&script, &root);
        assert!(asked_profile.exists(), "Bazel writes the caller's profile");
        assert!(
            std::fs::metadata(&asked_log).unwrap().len() > 0,
            "the requested workspace log is still a copy"
        );
    }

    /// Without an rc the CLI rendered, Bazel's own rc files may name a
    /// `--profile` no flag shows, so the profile is left where Bazel puts it.
    #[cfg(unix)]
    #[test]
    fn without_a_rendered_rc_the_profile_is_left_to_bazel() {
        let history = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("asked.profile.gz");
        let script = format!(
            r#"
def _impl(ctx):
    build = ctx.bazel.build(flags = ["--profile={asked}"], stderr = None)
    return 0 if build.wait().success else 1

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );
        run_task_with_history(&script, history.path());
        assert!(asked.exists(), "Bazel writes the caller's profile itself");
        assert_only_records(
            history.path(),
            "a build with nothing to keep keeps no files",
        );
    }

    /// Recording asks for no more than a file in the history, so a build with
    /// no rendered rc, whose own rc files could name a log path, records
    /// nothing and makes no history entry.
    #[cfg(unix)]
    #[test]
    fn without_a_rendered_rc_nothing_is_recorded() {
        let history = tempfile::tempdir().unwrap();
        run_task_with_history(
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [bazel.execution_log.record()],
        stderr = None,
    )
    return 0 if build.wait().success else 1

Test = task(implementation = _impl)
"#,
            history.path(),
        );
        assert_only_records(history.path(), "nothing recorded");
    }

    /// A task that keeps no history (`keep_history = False`) leaves nothing
    /// in it, though it runs Bazel: no entry, no build's folder, no prune.
    #[cfg(unix)]
    #[test]
    fn a_task_that_keeps_no_history_leaves_nothing_in_it() {
        let history = tempfile::tempdir().unwrap();
        run_task_with_history(
            r#"
def _impl(ctx):
    build = ctx.bazel.build(flags = ["//..."], stderr = None)
    return 0 if build.wait().success else 1

Test = task(implementation = _impl, keep_history = False)
"#,
            history.path(),
        );
        let left: Vec<_> = std::fs::read_dir(history.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    /// Bazel refuses a second log format, so a caller's JSON or binary log
    /// leaves nothing recorded beside it.
    #[cfg(unix)]
    #[test]
    fn a_recording_beside_another_log_format_is_skipped() {
        for flag in ["--execution_log_json_file", "--execution_log_binary_file"] {
            let history = tempfile::tempdir().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let script = format!(
                r#"
def _impl(ctx):
    rc = ctx.bazel.parse_rc(startup_flags = ["--ignore_all_rc_files"], flags = [])
    build = ctx.bazel.build(
        rc = rc,
        flags = ["--nogenerate_json_trace_profile", "{flag}={log}"],
        execution_log = [bazel.execution_log.record()],
        stderr = None,
    )
    return 0 if build.wait().success else 1

Test = task(implementation = _impl)
"#,
                log = dir.path().join("exec.log").display(),
            );
            run_task_with_history(&script, history.path());
            assert_only_records(history.path(), flag);
        }
    }

    /// A caller who turned profiling off is given no `--profile`, so gets none
    /// in the history either.
    #[cfg(unix)]
    #[test]
    fn a_profile_turned_off_stays_off() {
        let history = tempfile::tempdir().unwrap();
        run_task_with_history(
            r#"
def _impl(ctx):
    rc = ctx.bazel.parse_rc(startup_flags = ["--ignore_all_rc_files"], flags = [])
    build = ctx.bazel.build(
        rc = rc,
        flags = ["--nogenerate_json_trace_profile"],
        stderr = None,
    )
    return 0 if build.wait().success else 1

Test = task(implementation = _impl)
"#,
            history.path(),
        );
        assert_only_records(history.path(), "the profile left to Bazel");
    }

    /// The workspace handle's AXL surface: `done` before binding and after
    /// `drain()`, `try_pop()` mid-build, and a drained handle yielding nothing
    /// while the build still finishes.
    #[cfg(unix)]
    #[test]
    fn a_workspace_event_handle_pops_drains_and_reports_done() {
        run_basil_task(
            std::time::Duration::from_secs(60),
            r#"
def _impl(ctx):
    events = bazel.workspace_events.iterator()
    if events.done: return 2
    build = ctx.bazel.build(
        flags = ["--scenario=workspace_events_beyond_capacity"],
        workspace_events = [events],
        stderr = None,
    )
    first = None
    for event in events:
        first = event
        break
    if first == None or first.location != "1": return 3
    popped = events.try_pop()
    if popped != None and popped.location != "2": return 4
    events.drain()
    if not events.done: return 5
    if events.try_pop() != None: return 6
    for _ in events:
        return 7
    if not build.wait().success: return 1
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "a drained workspace event handle parked the reader",
            "the handle's done, try_pop and drain must behave as documented",
        );
    }

    /// A workspace log Bazel never creates ends the stream once the client
    /// has exited, rather than after [`QUIET`](bazel::stream::QUIET): an
    /// iterating task sees no events, promptly. Timed inside the task, since
    /// the harness around it costs more than the window.
    #[cfg(unix)]
    #[test]
    fn a_workspace_log_bazel_never_writes_ends_promptly() {
        let quiet = bazel::stream::QUIET.as_secs_f64();
        run_basil_task(
            std::time::Duration::from_secs(60),
            format!(
                r#"
load("@std//time.axl", "monotonic")

def _impl(ctx):
    events = bazel.workspace_events.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        workspace_events = [events],
        stderr = None,
    )
    start = monotonic()
    for _ in events:
        return 2
    if monotonic() - start >= {quiet}: return 3
    if not build.wait().success: return 1
    return 0

Test = task(implementation = _impl)
"#
            ),
            "the reader waited on a workspace log that never appeared",
            "a log that never appears must end with no events, before the quiet window",
        );
    }

    /// A workspace log that fails `wait()` must not leave the execution log's
    /// undrained handle subscribed: both lists are released before either join.
    #[cfg(unix)]
    #[test]
    fn a_failed_workspace_log_still_releases_the_execution_log_handles() {
        run_basil_task(
            std::time::Duration::from_secs(60),
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=corrupt_workspace_log_beside_execlog"],
        execution_log = [entries],
        workspace_events = [bazel.workspace_events.iterator()],
        stderr = None,
    )
    err, _ = catch(build.wait)
    if not err: return 2
    if "workspace event" not in err.message: return 3
    if not entries.done: return 4
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "wait() hung on a failed workspace log",
            "a failed workspace log must fail wait() and still release the execution log's handle",
        );
    }

    /// The execution log's handles bind before the workspace log's, so a
    /// workspace handle still bound to a running build fails the spawn after a
    /// fresh execution log handle was bound: that one is released, and the
    /// running build keeps its handle.
    #[cfg(unix)]
    #[test]
    fn a_failed_workspace_bind_releases_the_execution_log_handles_this_spawn_bound() {
        run_basil_task(
            std::time::Duration::from_secs(60),
            format!(
                r#"
def _impl(ctx):
    events = bazel.workspace_events.iterator()
    first = ctx.bazel.build(
        flags = ["--scenario=workspace_events_beyond_capacity"],
        workspace_events = [events],
        stderr = None,
    )
    entries = bazel.execution_log.iterator()
    err, _ = catch(
        ctx.bazel.build,
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries],
        workspace_events = [events],
        stderr = None,
    )
    if not err: return 2
    if "already bound" not in err.message: return 3
    if not entries.done: return 4
    if len([e for e in events]) != {SCENARIO_ENTRIES}: return 5
    if not first.wait().success: return 1
    return 0

Test = task(implementation = _impl)
"#
            ),
            "a failed spawn left the first build's reader parked",
            "the second spawn's execution log handle must be released and the first build's kept",
        );
    }

    /// `drain()` mid-build on an `execution_log.iterator()` handle, the AXL-level
    /// coverage of the handle's `drain()` and its `done` attribute.
    ///
    /// Three things in one script, because they only mean anything together:
    /// leaving a `for` loop early does *not* unsubscribe (`done` stays false, so
    /// a second loop could resume); `drain()` does, idempotently; and a drained
    /// handle then yields nothing while the build still ends — the release frees
    /// the producer parked at the channel capacity with most of the log left to
    /// send. Timed out because that last part fails by parking.
    #[cfg(unix)]
    #[test]
    fn a_drained_handle_stops_consuming_mid_build() {
        use std::time::Duration;

        run_basil_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    for _ in entries:
        seen += 1
        if seen == 5:
            break
    if entries.done: return 2  # leaving a loop is not unsubscribing
    entries.drain()
    entries.drain()            # idempotent
    if not entries.done: return 3
    for _ in entries:
        return 4               # a drained handle yields nothing
    status = build.wait()
    if not status.success: return 1
    if seen != 5: return 5
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "a handle drained mid-build left the reader parked past the end of the build",
            "a drained handle must unsubscribe, yield nothing more, and let the build end",
        );
    }

    /// A consumer slower than the decoder must still receive every entry.
    ///
    /// The decoder reads a log already complete on disk, so it outruns any AXL
    /// loop; a loop that also does per-entry work falls more than a ring behind
    /// within the first few hundred entries. That costs only time: the producer
    /// parks until the loop catches up.
    ///
    /// The per-entry work is arithmetic rather than a sleep, so the cost scales
    /// with the machine instead of adding a fixed second to CI.
    #[cfg(unix)]
    #[test]
    fn a_slow_consumer_still_receives_every_entry() {
        use std::time::Duration;

        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    busy = 0
    for entry in entries:
        seen += 1
        if entry.id != seen:
            return 2  # a gap: the producer dropped what this loop fell behind on
        for i in range(500):
            busy += i
    status = build.wait()
    if not status.success: return 1
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#
        );

        run_basil_task(
            Duration::from_secs(120),
            script,
            "a slow consumer parked the reader for good",
            &format!(
                "a consumer the decoder outruns must still see all {SCENARIO_ENTRIES} entries"
            ),
        );
    }

    /// Two decoded consumers on one build: a `file()` sink and a live
    /// `iterator()` handle, over a log longer than the channel capacity.
    ///
    /// Both must see **every** entry — the producer blocks for each, and the
    /// channel is a broadcast, so the two do not compete.
    /// This is also the combination most exposed to an unread subscriber left on
    /// the stream: blocking sends park against it at the capacity, and the drain
    /// runs before the `wait()` that would release it, so the two would wait on
    /// each other. Hence the timeout.
    #[cfg(unix)]
    #[test]
    fn a_file_sink_and_a_live_handle_both_see_the_whole_log() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let sink_path = dir.path().join("decoded.binpb");
        // `with_timeout` runs the body on a thread that may outlive this frame,
        // so the script is rendered here rather than borrowing the path inside it.
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries, bazel.execution_log.file(path = {sink:?})],
        stderr = None,
    )
    seen = 0
    for entry in entries:
        seen += 1
        if entry.id != seen:
            return 2  # a gap: the producer dropped something
    status = build.wait()
    if not status.success: return 1
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            sink = sink_path.to_str().unwrap(),
        );

        // The timeout is here to catch a deadlock, not to bound a healthy run,
        // which finishes in well under a second.
        run_basil_task(
            Duration::from_secs(60),
            script,
            "a file sink plus a live handle deadlocked",
            &format!(
                "a handle alongside a file sink must see all {SCENARIO_ENTRIES} entries, \
                 contiguously"
            ),
        );

        // The sink's writer is joined inside `wait()`, so its file is complete by
        // now: every entry length-delimited, not a prefix.
        assert_eq!(
            decoded_entry_count(&sink_path),
            SCENARIO_ENTRIES,
            "the decoded file sink should be complete",
        );
    }

    /// Count the length-delimited `ExecLogEntry`s a decoded `file()` sink wrote,
    /// asserting their ids are the contiguous run basil produced.
    ///
    /// A short count is the prefix a dropping producer leaves; a gap in the ids is
    /// what a sink competing for entries rather than broadcasting would leave.
    #[cfg(unix)]
    fn decoded_entry_count(path: &std::path::Path) -> u32 {
        let written = std::fs::read(path).expect("the sink should have written a file");
        count_length_delimited(&written, "the sink")
    }

    /// The same count over a compact (zstd) log, as Bazel and the reader thread's
    /// `MultiTeeReader` write it.
    #[cfg(unix)]
    fn compact_entry_count(path: &std::path::Path) -> u32 {
        let raw = std::fs::read(path).expect("bazel should have written the compact log");
        let decoded = zstd::decode_all(raw.as_slice()).expect("a complete zstd frame");
        count_length_delimited(&decoded, "the log")
    }

    #[cfg(unix)]
    fn count_length_delimited(mut buf: &[u8], what: &str) -> u32 {
        let mut count = 0;
        while !buf.is_empty() {
            let entry = ExecLogEntry::decode_length_delimited(&mut buf)
                .unwrap_or_else(|e| panic!("{what} should hold whole entries: {e}"));
            count += 1;
            assert_eq!(entry.id, count, "{what} should hold a contiguous run");
        }
        count
    }

    /// A deployment wired to an Aspect BES backend, a Workflows runner and a
    /// generated bazelrc each put `--execution_log_compact_file` on the command
    /// line themselves, and the backend reads the file at *that* path. The CLI's
    /// own flag outranks it, so that file is written from the stream, whole.
    #[cfg(unix)]
    #[test]
    fn an_already_requested_execution_log_is_written_as_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("deployment-asked-for-this.zstd");
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity", "--execution_log_compact_file={asked}"],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    for entry in entries:
        seen += 1
    status = build.wait()
    if not status.success: return 1
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );

        run_basil_task(
            std::time::Duration::from_secs(60),
            script,
            "the reader never reached the log",
            "the reader must deliver the whole log",
        );
        assert_eq!(
            compact_entry_count(&asked),
            SCENARIO_ENTRIES,
            "the log must still be written where the deployment asked, at {}",
            asked.display(),
        );
    }

    /// Run `script`, which must exit 0, with its task invocation's history
    /// kept under `history`.
    #[cfg(unix)]
    fn run_task_with_history(script: &str, history: &std::path::Path) {
        let history = history.to_path_buf();
        let exit = run_basil_script_with(
            std::time::Duration::from_secs(60),
            script.to_owned(),
            "the task did not finish",
            move |eval| eval.with_history(history),
        );
        assert_eq!(exit.expect("run_task"), Some(0));
    }

    /// Every task invocation entry under `history` holds its record and no
    /// build's files.
    #[cfg(unix)]
    fn assert_only_records(history: &std::path::Path, why: &str) {
        for entry in std::fs::read_dir(history)
            .unwrap()
            .map(|e| e.unwrap().path())
        {
            if entry
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with('.')
            {
                continue;
            }
            let mut names: Vec<String> = std::fs::read_dir(&entry)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(
                names,
                ["output.ansi", "output.log", "task.json"],
                "{why}: {}",
                entry.display()
            );
        }
    }

    /// The one task invocation entry under `history`, past the prune's own
    /// dotfiles.
    #[cfg(unix)]
    fn only_invocation(history: &std::path::Path) -> std::path::PathBuf {
        let entries: Vec<_> = std::fs::read_dir(history)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .collect();
        assert_eq!(entries.len(), 1, "one task invocation: {entries:?}");
        entries.into_iter().next().unwrap()
    }

    /// Each Bazel invocation of a task writes its compact execution log into a
    /// folder of its own in the task invocation's history, where it stays, and
    /// `record()` asks for one with nothing to read it.
    #[cfg(unix)]
    #[test]
    fn each_bazel_invocation_keeps_its_log_in_the_task_invocations_history() {
        let history = tempfile::tempdir().unwrap();
        run_task_with_history(
            r#"
def _impl(ctx):
    for _ in range(2):
        build = ctx.bazel.build(rc = ctx.bazel.parse_rc(startup_flags = ["--ignore_all_rc_files"], flags = []),
            flags = ["--scenario=execlog_beyond_capacity"],
            execution_log = [bazel.execution_log.record()],
            stderr = None,
        )
        if not build.wait().success: return 1
    return 0

Test = task(implementation = _impl)
"#,
            history.path(),
        );
        let invocation = only_invocation(history.path());
        for n in [1, 2] {
            let log = bazel_dir(&invocation, n).join(bazel::execlog::FILE_NAME);
            assert_eq!(
                compact_entry_count(&log),
                SCENARIO_ENTRIES,
                "{}",
                log.display()
            );
        }
    }

    #[test]
    fn recording_alone_skips_a_bazel_before_7_4() {
        use super::writes_execution_log;
        let v = |s: &str| semver::Version::parse(s).unwrap();
        assert!(writes_execution_log(false, true, Some(&v("7.4.0"))));
        assert!(
            writes_execution_log(false, true, None),
            "an unknown version"
        );
        assert!(!writes_execution_log(false, true, Some(&v("7.3.2"))));
        assert!(writes_execution_log(false, true, Some(&v("7.4.0-rc1"))));
        assert!(
            writes_execution_log(true, false, Some(&v("7.3.2"))),
            "asked for"
        );
        assert!(!writes_execution_log(false, false, None));
    }

    #[test]
    fn only_a_list_of_record_sinks_records_without_asking() {
        use super::{ExecLogSink, execlog_demand};
        let record = || vec![ExecLogSink::Record];
        let copy = || ExecLogSink::CompactFile { path: "c".into() };
        assert_eq!(
            execlog_demand(false, &[], false, false, false),
            (false, false)
        );
        assert_eq!(
            execlog_demand(true, &record(), false, false, false),
            (false, true)
        );
        assert_eq!(
            execlog_demand(true, &record(), true, false, false),
            (true, false),
            "a handle beside record() reads the log",
        );
        assert_eq!(
            execlog_demand(true, &[ExecLogSink::Record, copy()], false, false, false),
            (true, false),
        );
        assert_eq!(
            execlog_demand(true, &[], true, false, false),
            (true, false),
            "a handle alone"
        );
        assert_eq!(
            execlog_demand(false, &[], false, true, false),
            (true, false),
            "a requested path is a copy to write",
        );
        assert_eq!(
            execlog_demand(false, &[], false, false, true),
            (false, true)
        );
    }

    /// `record_every_build()` has a build write its log with no
    /// `execution_log=` of its own, as a deployment's custom task would.
    #[cfg(unix)]
    #[test]
    fn record_every_build_writes_a_log_for_a_build_that_asks_for_none() {
        let history = tempfile::tempdir().unwrap();
        run_task_with_history(
            r#"
def _impl(ctx):
    bazel.execution_log.record_every_build()
    build = ctx.bazel.build(rc = ctx.bazel.parse_rc(startup_flags = ["--ignore_all_rc_files"], flags = []), flags = ["--scenario=execlog_beyond_capacity"], stderr = None)
    return 0 if build.wait().success else 1

Test = task(implementation = _impl)
"#,
            history.path(),
        );
        let log = bazel_dir(&only_invocation(history.path()), 1).join(bazel::execlog::FILE_NAME);
        assert_eq!(compact_entry_count(&log), SCENARIO_ENTRIES);
    }

    /// The history request a feature injects as a flag writes the log into the
    /// history, and nothing is written under the marker's name.
    #[cfg(unix)]
    #[test]
    fn a_history_request_flag_records_the_log_in_the_history() {
        let history = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        run_task_with_history(
            &format!(
                r#"
def _impl(ctx):
    build = ctx.bazel.build(rc = ctx.bazel.parse_rc(startup_flags = ["--ignore_all_rc_files"], flags = []),
        flags = ["--scenario=execlog_beyond_capacity", "--execution_log_compact_file=%aspect_history%"],
        directory = {cwd:?},
        stderr = None,
    )
    return 0 if build.wait().success else 1

Test = task(implementation = _impl)
"#,
                cwd = cwd.path().to_str().unwrap(),
            ),
            history.path(),
        );
        let log = bazel_dir(&only_invocation(history.path()), 1).join(bazel::execlog::FILE_NAME);
        assert_eq!(compact_entry_count(&log), SCENARIO_ENTRIES);
        assert!(
            std::fs::read_dir(cwd.path()).unwrap().next().is_none(),
            "nothing written under the marker's name",
        );
    }

    /// A path the caller asks for with nothing in AXL reading the log is still
    /// written: Bazel writes the log into the history, and the CLI copies it.
    #[cfg(unix)]
    #[test]
    fn a_requested_execution_log_with_no_consumer_is_written_as_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("asked.zstd");
        let history = dir.path().join("history");
        run_task_with_history(
            &format!(
                r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity", "--execution_log_compact_file={asked}"],
        stderr = None,
    )
    return 0 if build.wait().success else 1

Test = task(implementation = _impl)
"#,
                asked = asked.to_str().unwrap(),
            ),
            &history,
        );
        assert_eq!(compact_entry_count(&asked), SCENARIO_ENTRIES, "the copy");
        let kept = bazel_dir(&only_invocation(&history), 1).join(bazel::execlog::FILE_NAME);
        assert_eq!(compact_entry_count(&kept), SCENARIO_ENTRIES, "Bazel's own");
    }

    /// Two `CompactFile` sinks naming one path share one copy, rather than two
    /// tees each `File::create`-ing it and writing the log into it twice over.
    /// A config appending a path the artifacts feature already appended is how
    /// it arrives in practice.
    ///
    /// The `iterator()` handle is here to count entries, and it also makes this a
    /// build with a named decoded consumer — so the stream drops its own
    /// subscriber. [`two_compact_sinks_complete_with_no_decoded_consumer`] is the
    /// configuration without one.
    #[cfg(unix)]
    #[test]
    fn a_compact_sink_never_names_the_file_bazel_is_writing() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let sink_path = dir.path().join("compact.zstd");
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [
            entries,
            bazel.execution_log.compact_file(path = {sink:?}),
            bazel.execution_log.compact_file(path = {sink:?}),
        ],
        stderr = None,
    )
    seen = 0
    for entry in entries:
        seen += 1
        if entry.id != seen:
            return 2  # a gap: the log was truncated under the reader
    status = build.wait()
    if not status.success: return 1
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            sink = sink_path.to_str().unwrap(),
        );

        run_basil_task(
            Duration::from_secs(60),
            script,
            "the reader parked",
            &format!("two sinks naming one path must still deliver all {SCENARIO_ENTRIES} entries"),
        );

        assert_eq!(
            compact_entry_count(&sink_path),
            SCENARIO_ENTRIES,
            "the compact log on disk should be complete",
        );
    }

    /// Two *distinct* compact sinks and no decoded consumer at all.
    ///
    /// With no decoded sink and no handle the stream has no subscriber, so the
    /// reader skips decoding. Compact sinks are tee'd from the raw bytes before
    /// decoding, so both files must still be complete.
    #[cfg(unix)]
    #[test]
    fn two_compact_sinks_complete_with_no_decoded_consumer() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("one.zstd");
        let second = dir.path().join("two.zstd");
        let script = format!(
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [
            bazel.execution_log.compact_file(path = {first:?}),
            bazel.execution_log.compact_file(path = {second:?}),
        ],
        stderr = None,
    )
    status = build.wait()
    return 0 if status.success else 1

Test = task(implementation = _impl)
"#,
            first = first.to_str().unwrap(),
            second = second.to_str().unwrap(),
        );

        run_basil_task(
            Duration::from_secs(60),
            script,
            "compact sinks with no decoded consumer parked the reader past the end \
             of the build",
            "a build with compact sinks and no decoded consumer must end cleanly",
        );

        for path in [&first, &second] {
            assert_eq!(
                compact_entry_count(path),
                SCENARIO_ENTRIES,
                "a compact sink is tee'd from the raw bytes, so it is complete \
                 whether or not anything decodes: {}",
                path.display(),
            );
        }
    }

    /// A tee'd compact copy that cannot be opened fails alone: the decoded stream
    /// and the other copies get the whole log, and `wait()` names the copy.
    #[cfg(unix)]
    #[test]
    fn a_compact_copy_that_cannot_open_fails_alone() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.zstd");
        let bad = dir.path().join("missing-dir").join("bad.zstd");
        let teed = dir.path().join("teed.zstd");
        let report = dir.path().join("report.txt");
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [
            entries,
            bazel.execution_log.compact_file(path = {first:?}),
            bazel.execution_log.compact_file(path = {bad:?}),
            bazel.execution_log.compact_file(path = {teed:?}),
        ],
        stderr = None,
    )
    seen = len([e for e in entries])
    ctx.std.fs.create({report:?}).write("%d\n" % seen)
    build.wait()
    return 0

Test = task(implementation = _impl)
"#,
            first = first.to_str().unwrap(),
            bad = bad.to_str().unwrap(),
            teed = teed.to_str().unwrap(),
            report = report.to_str().unwrap(),
        );

        let err = run_basil_script(Duration::from_secs(60), script, "the reader parked")
            .expect_err("a failed copy must fail `wait()`");
        let seen = std::fs::read_to_string(&report).unwrap();
        assert_eq!(seen.trim(), SCENARIO_ENTRIES.to_string(), "decoded entries");
        for path in [&first, &teed] {
            assert_eq!(
                compact_entry_count(path),
                SCENARIO_ENTRIES,
                "{}",
                path.display()
            );
        }
        assert!(!bad.exists());
        let err = format!("{err:#}");
        assert!(
            err.contains(bad.to_str().unwrap()) && err.contains("failed to create"),
            "the error should name the copy and why: {err}",
        );
    }

    /// A `compact_file` sink beside an `iterator()` handle: the compact copy is
    /// tee'd from the raw bytes and the handle reads the decoded stream, so both
    /// get everything.
    #[cfg(unix)]
    #[test]
    fn a_compact_sink_and_a_handle_both_see_the_whole_log() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let sink_path = dir.path().join("compact.zstd");
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries, bazel.execution_log.compact_file(path = {sink:?})],
        stderr = None,
    )
    seen = 0
    for entry in entries:
        seen += 1
        if entry.id != seen:
            return 2  # a gap: the producer dropped something
    status = build.wait()
    if not status.success: return 1
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            sink = sink_path.to_str().unwrap(),
        );

        run_basil_task(
            Duration::from_secs(60),
            script,
            "a compact sink plus a handle deadlocked",
            &format!(
                "a handle beside a compact sink must see all \
                 {SCENARIO_ENTRIES} entries, contiguously"
            ),
        );

        assert_eq!(
            compact_entry_count(&sink_path),
            SCENARIO_ENTRIES,
            "the compact log on disk should be complete too",
        );
    }

    /// `--execution_log_compact_file=` clears the flag rather than naming a
    /// file, so it is not a path to copy the log to.
    ///
    /// An rc file setting the flag for a config and a command line clearing it
    /// is an ordinary way to reach this. Read back as `Some("")` it would be a
    /// copy at `""`, which cannot be created and fails `wait()`.
    #[cfg(unix)]
    #[test]
    fn an_empty_execution_log_flag_is_not_a_reusable_path() {
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity", "--execution_log_compact_file="],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    for _ in entries:
        seen += 1
    status = build.wait()
    if not status.success: return 1
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#
        );
        run_basil_task(
            std::time::Duration::from_secs(60),
            script,
            "the reader never ended",
            "a cleared flag must not become a copy at \"\"",
        );
    }

    /// A relative `--execution_log_compact_file` names a path under the cwd
    /// `Build::spawn` gives Bazel, as Bazel would resolve it, so its copy goes
    /// there and not under the CLI's own cwd. `format` and `gazelle` both pass
    /// a `directory` and both consume the log.
    #[cfg(unix)]
    #[test]
    fn a_relative_execution_log_flag_resolves_against_the_spawn_directory() {
        let dir = tempfile::tempdir().unwrap();
        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        directory = {dir:?},
        flags = ["--scenario=execlog_beyond_capacity", "--execution_log_compact_file=relative.zstd"],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    for _ in entries:
        seen += 1
    status = build.wait()
    if not status.success: return 1
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            dir = dir.path().to_str().unwrap(),
        );

        run_basil_task(
            std::time::Duration::from_secs(60),
            script,
            "the reader never ended",
            "a relative path must be copied under the spawn's directory",
        );
        assert!(
            dir.path().join("relative.zstd").exists(),
            "the copy of a relative path goes under the spawn's directory",
        );
    }

    /// Write a compact execution log of `count` entries numbered from `first`,
    /// standing in for the one an earlier build left at a reused path.
    ///
    /// The format is the real one: a zstd frame over varint-length-prefixed
    /// `ExecLogEntry`s, which is what `ExecLogStream` decodes. Ids far from the
    /// ones basil writes are what let a test say *which* build's log it got.
    #[cfg(unix)]
    fn seed_execution_log(path: &std::path::Path, first: u32, count: u32) {
        use std::io::Write;
        let file = std::fs::File::create(path).expect("create the stale log");
        let mut encoder = zstd::Encoder::new(file, 0).expect("zstd encoder");
        for id in first..first + count {
            let entry = ExecLogEntry {
                id,
                r#type: Some(axl_proto::tools::protos::exec_log_entry::Type::File(
                    axl_proto::tools::protos::exec_log_entry::File {
                        path: format!("stale/f{id}.txt"),
                        digest: None,
                    },
                )),
            };
            encoder
                .write_all(&entry.encode_length_delimited_to_vec())
                .expect("write the stale log");
        }
        encoder.finish().expect("finish the zstd frame");
    }

    /// A log an earlier build left at the caller's path is not this build's.
    ///
    /// A path reused across builds is the normal case: a Workflows runner or an
    /// Aspect-wired deployment names one. Bazel writes the log into the task
    /// invocation's history, and the caller's path is a copy, so the old log there
    /// is replaced, never read. Were it read, the consumer would be handed the
    /// previous run's entries: spawns that never ran, inputs that are gone, and
    /// a `kinds = ["spawn"]` hook told about actions this build did not execute.
    ///
    /// The stale ids are 90001-upwards so the assertion names the build the
    /// entries came from rather than counting them. `execlog_after_bes` publishes
    /// this build's log two seconds after the event stream closes, a window in
    /// which only the stale file exists.
    #[cfg(unix)]
    #[test]
    fn a_log_left_by_an_earlier_build_is_not_delivered_as_this_one() {
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("reused-across-builds.zstd");
        seed_execution_log(&asked, 90_001, 7);

        let script = format!(
            r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_after_bes", "--execution_log_compact_file={asked}"],
        execution_log = [entries],
        stderr = None,
    )
    seen = 0
    highest = 0
    for entry in entries:
        seen += 1
        if entry.id > highest:
            highest = entry.id
    status = build.wait()
    if not status.success: return 1
    # Any id from the seeded run means the reader opened the previous log.
    if highest > {SCENARIO_ENTRIES}: return 2
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );

        // The timeout catches a reader parked on the stale inode rather than
        // bounding a healthy run, which takes a little over two seconds.
        run_basil_task(
            std::time::Duration::from_secs(120),
            script,
            "the reader never reached this build's log",
            "the consumer must receive this build's log, not the one already at the path",
        );
        assert_eq!(
            compact_entry_count(&asked),
            SCENARIO_ENTRIES,
            "this build's log"
        );
    }

    /// `bazel info server_pid` must be asked with the startup flags the build
    /// will use.
    ///
    /// That pid is the execution log's nominated holder. Bazel kills a running
    /// server whose startup options differ from the ones it is handed, so a pid
    /// read without the build's startup flags belongs to a server the build then
    /// replaces. Dead, the holder makes the reader take a log that does not exist
    /// yet for one that never will: it ends the stream clean and empty, every
    /// `exec_log_event` hook fires zero times, every sink writes nothing, and the
    /// build passes. Reproduced against a real Bazel with
    /// `--bazel-startup-flag=--nosystem_rc`, which is enough on its own.
    ///
    /// Asserted on basil's argv rather than on an entry count, because basil has
    /// no server to restart — the fake cannot show the consequence, only that the
    /// two invocations were told the same thing.
    #[cfg(unix)]
    #[test]
    fn the_server_pid_is_read_with_the_builds_startup_flags() {
        let dir = tempfile::tempdir().unwrap();
        let argv_log = dir.path().join("argv.log");
        // `BASIL_ARGV_LOG` is process-wide, so any basil a concurrent test runs
        // while it is set appends here too — including that test's own
        // `info server_pid`. The marker is what tells this test's invocations
        // apart, so the assertion looks for a marked `info` line rather than
        // insisting every `info` line is marked. It is minted per run because the
        // failure is one-sided: a foreign line carrying the marker would pass this
        // test without the build's startup flags having reached `info` at all.
        let marker = format!(
            "--host_jvm_args=-Daspect.argv_marker={}",
            uuid::Uuid::new_v4()
        );
        let script = format!(
            r#"
def _impl(ctx):
    rc = ctx.bazel.parse_rc(startup_flags = ["--nosystem_rc", {marker:?}], flags = [])
    build = ctx.bazel.build(
        rc = rc,
        flags = ["--scenario=success"],
        stderr = None,
    )
    status = build.wait()
    return 0 if status.success else 1

Test = task(implementation = _impl)
"#,
        );

        let exit = with_daemon_stand_in(|| {
            // SAFETY: process-wide, but `with_daemon_stand_in` holds the lock that
            // serializes the basil tests for the whole closure.
            unsafe {
                std::env::set_var("BASIL_ARGV_LOG", &argv_log);
            }
            let out = crate::test::eval(&script).with_fake_bazel().run_task(0);
            unsafe {
                std::env::remove_var("BASIL_ARGV_LOG");
            }
            out
        });
        assert_eq!(
            exit.expect("run_task"),
            Some(0),
            "the build should have run"
        );

        let recorded = std::fs::read_to_string(&argv_log).expect("basil should have logged argv");
        let info = recorded
            .lines()
            .filter(|line| line.split_whitespace().any(|a| a == "info"));
        assert!(
            info.clone().next().is_some(),
            "no `info` invocation at all in:\n{recorded}",
        );
        assert!(
            info.clone().any(|line| line.contains(&marker)),
            "`bazel info` ran without the build's startup flags, so the server pid it \
             reports is not the server that runs the build:\n{}",
            info.map(|l| format!("  {l}\n")).collect::<String>(),
        );
    }

    /// Binding several handles is all-or-nothing.
    ///
    /// A bound handle is a subscriber the producer blocks for, so one left
    /// behind by a half-finished bind would park the reader on a consumer that
    /// is never coming. `execution_log = [h, h]` is the AXL shape that reaches
    /// this: `UnpackValue` clones share one state, so the second bind sees the
    /// first's.
    ///
    /// `release` and a drained handle both end in `Done`, so a later `bind` fails
    /// either way — the rollback is only visible as the handle no longer holding
    /// its subscriber.
    #[cfg(unix)]
    #[test]
    fn a_failed_execlog_bind_releases_the_handles_already_bound() {
        use super::Handle;
        use bazel::execlog::feed::ExecLogStream;

        // A reaped pid for the log's nominated holder, so the reader thread
        // takes the missing file as "no log is coming" and exits instead of
        // waiting out the test.
        let mut gone = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawn a process to reap");
        let reaped = gone.id();
        gone.wait().expect("reap it");

        let dir = tempfile::tempdir().unwrap();
        let stream = ExecLogStream::spawn_with_file(
            dir.path().join("never-written.zstd"),
            reaped,
            reaped,
            vec![],
        )
        .expect("spawn the reader");

        let signals = crate::engine::cancellation::Signals::new();
        let already = super::ExecLogIter::new(signals.clone(), None);
        already
            .bind(stream.receiver().expect("a subscriber to clone"))
            .expect("the first bind should succeed");
        let fresh = super::ExecLogIter::new(signals, None);

        let err = super::bind_handles(
            &[fresh.clone(), already.clone()],
            || stream.receiver(),
            "execution log",
        )
        .expect_err("binding an already-bound handle must fail");
        assert!(
            err.to_string().contains("already bound"),
            "unexpected error: {err}",
        );
        assert!(
            !fresh.is_live(),
            "the handle bound before the failure must be released, or the log \
             reader blocks on a consumer that will never read",
        );
        assert!(
            already.is_live(),
            "the handle whose bind failed belongs to another binding and must \
             stay subscribed",
        );
    }

    /// Every accepted `kinds=` spelling, the tag it resolves to, and the
    /// string `type(event.payload)` reports for that payload — pinned
    /// together, because the whole point of [`EVENT_KINDS`] is that the three
    /// agree. A row drifting apart is the bug this table replaced: `kinds =
    /// ["action_completed"]` was accepted while `type(event.payload)`
    /// returned `action_executed`, so the filter read right and never matched.
    mod event_kinds {
        use super::super::*;
        use axl_proto::build_event_stream::build_event::Payload;
        use bazel::bes::kind::{EVENT_KINDS, payload_discriminant, payload_name};
        use starlark::values::Heap;

        /// One default-constructed payload per `Payload` variant, in
        /// [`EVENT_KINDS`] order. Exhaustive by construction: the match in
        /// `payload_discriminant` fails to compile if a variant is added
        /// upstream, and `the_table_covers_every_payload_variant` fails if
        /// one is missing from here.
        fn every_payload() -> Vec<Payload> {
            vec![
                Payload::Progress(Default::default()),
                Payload::Aborted(Default::default()),
                Payload::Started(Default::default()),
                Payload::Expanded(Default::default()),
                Payload::Configured(Default::default()),
                Payload::Action(Default::default()),
                Payload::Completed(Default::default()),
                Payload::TestResult(Default::default()),
                Payload::Finished(Default::default()),
                Payload::UnstructuredCommandLine(Default::default()),
                Payload::StructuredCommandLine(Default::default()),
                Payload::OptionsParsed(Default::default()),
                Payload::NamedSetOfFiles(Default::default()),
                Payload::WorkspaceStatus(Default::default()),
                Payload::Fetch(Default::default()),
                Payload::Configuration(Default::default()),
                Payload::TestSummary(Default::default()),
                Payload::BuildToolLogs(Default::default()),
                Payload::BuildMetrics(Default::default()),
                Payload::BuildMetadata(Default::default()),
                Payload::WorkspaceInfo(Default::default()),
                Payload::TargetSummary(Default::default()),
                Payload::ConvenienceSymlinksIdentified(Default::default()),
                Payload::ExecRequest(Default::default()),
                Payload::TestProgress(Default::default()),
            ]
        }

        #[test]
        fn the_table_covers_every_payload_variant() {
            assert_eq!(
                every_payload().len(),
                EVENT_KINDS.len(),
                "a Payload variant is missing from EVENT_KINDS or every_payload()"
            );
            let mut tags: Vec<i32> = EVENT_KINDS.iter().map(|(tag, ..)| *tag).collect();
            tags.sort_unstable();
            tags.dedup();
            assert_eq!(
                tags.len(),
                EVENT_KINDS.len(),
                "duplicate tag in EVENT_KINDS"
            );

            let mut names: Vec<&str> = EVENT_KINDS
                .iter()
                .flat_map(|(_, type_name, aliases)| {
                    std::iter::once(*type_name).chain(aliases.iter().copied())
                })
                .collect();
            names.sort_unstable();
            let total = names.len();
            names.dedup();
            assert_eq!(names.len(), total, "duplicate alias in EVENT_KINDS");
        }

        /// `payload_discriminant` is the receive side of the filter and
        /// `EVENT_KINDS` the send side; a disagreement silently drops events.
        #[test]
        fn payload_tags_agree_with_the_kind_table() {
            for (payload, (tag, type_name, _)) in every_payload().into_iter().zip(EVENT_KINDS) {
                assert_eq!(payload_discriminant(&payload), *tag, "tag for {type_name}");
                assert_eq!(payload_name(&payload), *type_name);
            }
        }

        /// The agreement test: the name the table advertises is the string
        /// AXL's `type(event.payload)` returns for that payload.
        #[test]
        fn every_kind_is_named_the_way_type_reports_it() {
            Heap::temp(|heap| {
                for (payload, (_, type_name, _)) in every_payload().into_iter().zip(EVENT_KINDS) {
                    let reported = heap.alloc(payload).get_type();
                    assert_eq!(
                        reported, *type_name,
                        "`type(event.payload)` reports {reported:?}, so `kinds=` must accept it"
                    );
                }
            });
        }

        #[test]
        fn every_alias_resolves_to_its_tag() {
            Heap::temp(|heap| {
                for (tag, type_name, aliases) in EVENT_KINDS {
                    for name in std::iter::once(type_name).chain(aliases.iter()) {
                        let v = heap.alloc_str(name).to_value();
                        assert_eq!(parse_event_kind(v).unwrap(), *tag, "alias {name}");
                    }
                }
            });
        }

        #[test]
        fn a_raw_tag_number_passes_through() {
            let (action_tag, ..) = EVENT_KINDS
                .iter()
                .find(|(_, name, _)| *name == "action_executed")
                .expect("action_executed must be in the table");
            Heap::temp(|heap| {
                assert_eq!(
                    parse_event_kind(heap.alloc(*action_tag)).unwrap(),
                    *action_tag
                );
            });
        }

        /// An int outside `i32` fails `unpack_i32`, so without its own arm it
        /// reaches the wrong-type bail and gets told an int is not a tag.
        #[test]
        fn a_tag_too_large_for_i32_is_reported_as_a_tag_not_a_type_error() {
            Heap::temp(|heap| {
                let err = parse_event_kind(heap.alloc(1_099_511_627_776i64))
                    .expect_err("an out-of-range tag must be rejected")
                    .to_string();
                assert!(
                    err.contains("out of range"),
                    "should say it is out of range, not report a type error: {err}"
                );
                assert!(
                    !err.contains("got int"),
                    "an int must not be told it is not a tag: {err}"
                );
            });
        }

        #[test]
        fn a_tag_no_row_claims_is_rejected() {
            Heap::temp(|heap| {
                let err = parse_event_kind(heap.alloc(999i32))
                    .expect_err("a tag outside the table must be rejected")
                    .to_string();
                assert!(err.contains("999"), "unexpected error: {err}");
            });
        }

        /// Every spelling `kinds=` accepted before the payload names were added,
        /// frozen with the tag it resolved to.
        ///
        /// Deliberately a literal rather than a walk of `EVENT_KINDS`: a test
        /// derived from the table cannot notice a row losing an alias, it just
        /// iterates one fewer time. Nothing in-tree passes these through
        /// `parse_event_kind` either — `RESULTS_KINDS` is locked by its own AXL
        /// drift test and `process_event` is fed synthetic events — so without
        /// this list a cleanup that drops a "redundant" alias keeps every test
        /// green and makes `aspect build --live` fail at runtime on an unknown
        /// kind, because `build.axl` passes `results.KINDS` to `kinds=`.
        #[test]
        fn the_spellings_accepted_before_the_payload_names_still_resolve() {
            const FROZEN: &[(&str, i32)] = &[
                ("aborted", 4),
                ("action", 8),
                ("action_completed", 8),
                ("build_finished", 11),
                ("build_metadata", 24),
                ("build_metrics", 22),
                ("build_started", 5),
                ("build_tool_logs", 21),
                ("completed", 9),
                ("configuration", 19),
                ("configured", 7),
                ("convenience_symlinks_identified", 27),
                ("exec_request", 28),
                ("expanded", 6),
                ("fetch", 17),
                ("finished", 11),
                ("named_set", 15),
                ("named_set_of_files", 15),
                ("options_parsed", 14),
                ("pattern_expanded", 6),
                ("progress", 3),
                ("started", 5),
                ("structured_command_line", 13),
                ("target_completed", 9),
                ("target_configured", 7),
                ("target_summary", 26),
                ("test_result", 10),
                ("test_summary", 20),
                ("unstructured_command_line", 12),
                ("workspace_config", 25),
                ("workspace_info", 25),
                ("workspace_status", 16),
            ];
            Heap::temp(|heap| {
                for (name, tag) in FROZEN {
                    let v = heap.alloc_str(name).to_value();
                    let got = parse_event_kind(v)
                        .unwrap_or_else(|e| panic!("`{name}` no longer resolves: {e}"));
                    assert_eq!(
                        got, *tag,
                        "`{name}` resolved to {got}, was {tag} — a public spelling changed meaning",
                    );
                }
            });
        }

        /// A typo has to name every valid kind: without the list a user has
        /// no way to discover the spellings.
        #[test]
        fn an_unknown_kind_enumerates_the_valid_names() {
            Heap::temp(|heap| {
                let v = heap.alloc_str("not_a_real_kind").to_value();
                let err = parse_event_kind(v)
                    .expect_err("expected a rejection")
                    .to_string();
                assert!(err.contains("not_a_real_kind"), "unexpected error: {err}");
                for (_, type_name, aliases) in EVENT_KINDS {
                    for name in std::iter::once(type_name).chain(aliases.iter()) {
                        assert!(
                            err.contains(name),
                            "the error should list {name}, got: {err}"
                        );
                    }
                }
            });
        }

        /// A non-string, non-int entry still gets told what a kind looks like.
        #[test]
        fn a_wrong_typed_entry_enumerates_the_valid_names() {
            Heap::temp(|heap| {
                let err = parse_event_kind(heap.alloc(vec![1i32]))
                    .expect_err("expected a rejection")
                    .to_string();
                assert!(err.contains("action_executed"), "unexpected error: {err}");
            });
        }
    }

    /// An iterator handle bound before the stream starts receives every event
    /// of a clean build, the early burst included.
    #[test]
    fn iterator_handle_receives_early_burst() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter],
        stderr = None,
    )
    started = 0
    finished = 0
    other = 0
    for event in iter:
        kind = event.kind
        if kind == "build_started":
            started += 1
        elif kind == "build_finished":
            finished += 1
        else:
            other += 1
    status = build.wait()
    if not status.success: return 1
    if started != 1: return 2
    if finished != 1: return 3
    if other != 0: return 4
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect("run_task");

        assert_eq!(exit, Some(0));
    }

    /// The kinds of the events in the binary BEP file at `path`, in order.
    fn bep_file_kinds(path: &std::path::Path) -> Vec<String> {
        use axl_proto::build_event_stream::build_event::Payload;
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("the BEP file at {} should exist: {e}", path.display()));
        let mut buf = bytes.as_slice();
        let mut kinds = vec![];
        while !buf.is_empty() {
            let event =
                axl_proto::build_event_stream::BuildEvent::decode_length_delimited(&mut buf)
                    .expect("the BEP file should hold whole events");
            kinds.push(match event.payload {
                Some(Payload::Started(_)) => "started".to_string(),
                Some(Payload::Finished(_)) => "finished".to_string(),
                other => format!("{other:?}"),
            });
        }
        kinds
    }

    /// Run `script` against basil and assert it exited 0.
    fn run_bes_task(script: &str) {
        let exit = crate::test::eval(script)
            .with_fake_bazel()
            .run_task(0)
            .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// Stream a build that asks for `--build_event_binary_file=<path>` to an
    /// iterator, asserting the iterator sees every event.
    fn stream_with_requested_bep_file(path: &std::path::Path) {
        run_bes_task(&format!(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=success", "--build_event_binary_file={path}"],
        build_events = [iter],
        stderr = None,
    )
    seen = len([e for e in iter])
    if not build.wait().success: return 1
    if seen != 2: return 2
    return 0

Test = task(implementation = _impl)
"#,
            path = path.to_str().unwrap(),
        ));
    }

    /// A caller's `--build_event_binary_file` is written even though the CLI's
    /// own flag, appended last, is the one Bazel writes to.
    #[test]
    fn a_requested_bep_file_is_written_while_the_cli_streams() {
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("ide.bep");
        stream_with_requested_bep_file(&asked);
        assert_eq!(bep_file_kinds(&asked), ["started", "finished"]);
    }

    /// A relative path lands where Bazel would put it: under the workspace
    /// root, in a folder created for it.
    #[test]
    fn a_relative_bep_file_resolves_against_the_workspace_root() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("MODULE.bazel"), "").unwrap();
        let pkg = ws.path().join("pkg");
        std::fs::create_dir(&pkg).unwrap();
        run_bes_task(&format!(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(
        directory = {dir:?},
        flags = ["--scenario=success", "--build_event_binary_file=out/relative.bep"],
        build_events = [iter],
        stderr = None,
    )
    seen = len([e for e in iter])
    if not build.wait().success: return 1
    if seen != 2: return 2
    return 0

Test = task(implementation = _impl)
"#,
            dir = pkg.to_str().unwrap(),
        ));
        assert_eq!(
            bep_file_kinds(&ws.path().join("out/relative.bep")),
            ["started", "finished"]
        );
    }

    /// Without a stream the CLI adds no flag of its own, so the caller's flag reaches
    /// Bazel untouched and Bazel writes the file itself.
    #[test]
    fn without_a_stream_bazel_writes_the_requested_bep_file() {
        let dir = tempfile::tempdir().unwrap();
        let asked = dir.path().join("ide.bep");
        run_bes_task(&format!(
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=success", "--build_event_binary_file={asked}"],
        stderr = None,
    )
    return 0 if build.wait().success else 1

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        ));
        assert_eq!(bep_file_kinds(&asked), ["started", "finished"]);
    }

    /// A path that cannot be created costs only the caller's copy: the CLI's
    /// own consumers of the stream still see every event.
    #[test]
    fn an_unwritable_bep_file_does_not_end_the_stream() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("a-file");
        std::fs::write(&blocker, "").unwrap();
        let unwritable = blocker.join("ide.bep");
        stream_with_requested_bep_file(&unwritable);
        assert!(!unwritable.exists());
    }

    /// A file sink at a path that cannot be opened reports its own failure;
    /// the build, its iterators and the other sinks finish as usual.
    #[test]
    fn a_file_sink_that_cannot_open_fails_alone() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.bep");
        let bad = dir.path().join("missing-dir").join("bad.bep");
        run_bes_task(&format!(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    good = bazel.build_events.file(path = {good:?})
    bad = bazel.build_events.file(path = {bad:?})
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter, bad, good],
        stderr = None,
    )
    seen = len([e for e in iter])
    if not build.wait().success: return 1
    if seen != 2: return 2
    good.wait()
    bad.wait()
    if good.failed: return 3
    if not bad.failed: return 4
    if "cannot open" not in bad.error: return 5
    return 0

Test = task(implementation = _impl)
"#,
            good = good.to_str().unwrap(),
            bad = bad.to_str().unwrap(),
        ));
        assert!(std::fs::metadata(&good).unwrap().len() > 0);
    }

    /// `fully_async` lets the Bazel client exit while the server still writes
    /// the terminal `BuildFinished` event. The invocation must deliver that
    /// event before it reports a complete stream.
    #[cfg(unix)]
    #[test]
    fn fully_async_bep_delivers_build_finished_after_client_exit() {
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let mut server = std::process::Command::new(crate::test::basil_bin())
            .arg("__async_bep_tail_writer")
            .arg(dir.path())
            .spawn()
            .expect("spawn fake Bazel server writer");
        let server_pid_flag = format!("--basil-server-pid={}", server.id());
        let script = format!(
            r#"
def _impl(ctx):
    events = bazel.build_events.iterator()
    rc = ctx.bazel.parse_rc(startup_flags = [{server_pid_flag:?}], flags = [])
    build = ctx.bazel.build(
        rc = rc,
        flags = ["--scenario=async_bep_tail_after_client_exit", {control_dir_flag:?}],
        build_events = [events],
        stderr = None,
    )
    started = 0
    finished = 0
    for event in events:
        if event.kind == "build_started":
            started += 1
        elif event.kind == "build_finished":
            finished += 1
    status = build.wait()
    if not status.success: return 1
    if started != 1: return 2
    if finished != 1: return 3
    return 0

Test = task(implementation = _impl)
"#,
            server_pid_flag = server_pid_flag,
            control_dir_flag = format!("--basil-control-dir={}", dir.path().display()),
        );

        let result = crate::test::with_timeout(Duration::from_secs(30), move || {
            crate::test::eval(&script).with_fake_bazel().run_task(0)
        });
        let Some(result) = result else {
            let _ = server.kill();
            let _ = server.wait().expect("wait for fake Bazel server writer");
            std::mem::forget(dir);
            panic!("timed out waiting for the asynchronous BEP tail");
        };

        let deadline = Instant::now() + Duration::from_secs(10);
        let server_status = loop {
            if let Some(status) = server
                .try_wait()
                .expect("check fake Bazel server writer status")
            {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = server.kill();
                let _ = server.wait().expect("wait for fake Bazel server writer");
                panic!("timed out waiting for fake Bazel server writer to exit");
            }
            std::thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(
            result.expect("run_task"),
            Some(0),
            "the stream must deliver BuildFinished after the client exits"
        );
        assert!(
            server_status.success(),
            "fake Bazel server writer failed: {server_status}"
        );
    }

    /// A client a signal ended has no exit code; its status names the
    /// signal instead.
    #[cfg(unix)]
    #[test]
    fn a_client_a_signal_ends_reports_the_signal() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    status = ctx.bazel.build(flags = ["--scenario=signal_killed_sigkill"], stderr = None).wait()
    if status.success or status.code != None: return 1
    if status.signal != 9: return 2
    ok = ctx.bazel.build(flags = ["--scenario=success"], stderr = None).wait()
    if ok.code != 0 or ok.signal != None: return 3
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// A mistyped flag reaches Bazel, which rejects the command line and
    /// exits without ever creating the BEP file. The build must report that
    /// exit code instead of waiting forever for a file that cannot come.
    #[test]
    fn a_rejected_command_line_does_not_hang_the_bes_reader() {
        use std::time::Duration;
        // Generous: the timeout is here to catch a hang, not to bound a
        // healthy run, which finishes in well under a second.
        let result = crate::test::with_timeout(Duration::from_secs(60), || {
            crate::test::eval(
                r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=rejects_command_line"],
        build_events = [iter],
        stderr = None,
    )
    events = 0
    for _ in iter:
        events += 1
    status = build.wait()
    if status.success: return 1
    if status.code != 2: return 2
    if events != 0: return 3
    return 0

Test = task(implementation = _impl)
"#,
            )
            .with_fake_bazel()
            .run_task(0)
        });

        match result {
            None => panic!("timed out: a rejected command line hung the BES reader"),
            Some(exit) => assert_eq!(
                exit.expect("run_task"),
                Some(0),
                "expected an empty stream and bazel's exit code 2"
            ),
        }
    }

    /// The same rejected command line with the execution log stream on. Bazel
    /// writes no execution log and exits, leaving its daemon idling behind it for
    /// `--max_idle_secs`; the reader has to take the client's death as the answer,
    /// because the daemon's is minutes away. `BASIL_SERVER_PID` stands a live
    /// process in for that daemon — without it basil reports its own already-reaped
    /// pid and the wait would end for the wrong reason, passing either way.
    #[cfg(unix)]
    #[test]
    fn a_rejected_command_line_does_not_hang_the_execlog_reader() {
        use std::time::Duration;

        // Generous: the timeout is here to catch a hang, not to bound a
        // healthy run, which finishes in well under a second.
        let result = with_daemon_stand_in(|| {
            crate::test::with_timeout(Duration::from_secs(60), || {
                crate::test::eval(
                    r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=rejects_command_line"],
        execution_log = [],
        stderr = None,
    )
    status = build.wait()
    if status.success: return 1
    if status.code != 2: return 2
    return 0

Test = task(implementation = _impl)
"#,
                )
                .with_fake_bazel()
                .run_task(0)
            })
        });

        match result {
            None => panic!("timed out: a rejected command line hung the execlog reader"),
            Some(exit) => assert_eq!(
                exit.expect("run_task"),
                Some(0),
                "expected no execution log and bazel's exit code 2"
            ),
        }
    }

    /// Regression for aspect-build/aspect-cli#1060: REMOTE_CACHE_EVICTED
    /// without a follow-up retry must not hang the BES reader.
    #[test]
    fn bug_1060_remote_cache_evicted_without_retry_does_not_hang() {
        use std::time::Duration;
        // The timeout exists to catch a hang, not to bound a healthy run —
        // keep it generous so full-suite pool contention can't trip it.
        let result = crate::test::with_timeout(Duration::from_secs(60), || {
            crate::test::eval(
                r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=cache_evicted_no_retry"],
        build_events = [iter],
        stderr = None,
    )
    for _ in iter:
        pass
    build.wait()
    return 0

Test = task(implementation = _impl)
"#,
            )
            .with_fake_bazel()
            .run_task(0)
        });

        match result {
            None => panic!("build hung on REMOTE_CACHE_EVICTED with no retry (bug 1060)"),
            Some(r) => {
                let exit = r.expect("run_task");
                assert_eq!(exit, Some(0));
            }
        }
    }

    /// A build bazel retries after a cache eviction, through the whole stack:
    /// both attempts reach the AXL iterator although the second started the
    /// event file over, and bazel's own exit code is surfaced. basil reports a
    /// server pid that never holds the file, so this also shows the retry is
    /// followed without one.
    #[test]
    fn a_cache_eviction_retry_delivers_both_attempts_end_to_end() {
        use std::time::Duration;
        let result = crate::test::with_timeout(Duration::from_secs(60), || {
            crate::test::eval(
                r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=cache_evicted_with_retry"],
        build_events = [iter],
        stderr = None,
    )
    count = 0
    for _ in iter:
        count += 1
    code = build.wait().code
    if count != 4: return 10 + count
    if code != 0: return 2
    return 0

Test = task(implementation = _impl)
"#,
            )
            .with_fake_bazel()
            .run_task(0)
        });

        match result {
            None => panic!("timed out: a cache-eviction retry hung the BES reader"),
            Some(r) => assert_eq!(
                r.expect("run_task"),
                Some(0),
                "1x = the iterator saw x events, not 4; 2 = bazel's exit code was lost"
            ),
        }
    }

    /// A spawn that fails once bazel runs reads the event file bazel wrote to
    /// its end, delivering it to what was bound and removing it, before the
    /// error is returned.
    #[test]
    fn an_abandoned_stream_leaves_no_event_file_behind() {
        use super::{BuildEvent, BuildEventStream, Gate, Unstarted};
        use prost::Message;
        use std::path::Path;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let last = BuildEvent {
            last_message: true,
            ..Default::default()
        };
        let file = BuildEventStream::reserve();
        let path = file.path().to_path_buf();
        let client_exited = Arc::new(AtomicBool::new(false));
        let exited = client_exited.clone();
        let gate = Gate::new(move || exited.load(Ordering::SeqCst)).with_held(|_| false);
        let unstarted = Unstarted::new(Some(file), gate, vec![], None);
        let sub = unstarted.stream().unwrap().subscribe();
        unstarted.abandon(|| {
            // Bazel wrote its file, and stopping the client ends it.
            std::fs::write(&path, last.encode_length_delimited_to_vec()).unwrap();
            client_exited.store(true, Ordering::SeqCst);
        });
        assert_eq!(sub.recv().expect("the event was read").event, last);
        let mut moved = path.clone().into_os_string();
        moved.push(".1");
        assert!(!path.exists() && !Path::new(&moved).exists());
    }

    /// Iterator handles are single-use; reusing one errors.
    #[test]
    fn iterator_handle_rejects_reuse() {
        let err = crate::test::eval(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    first = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter],
        stderr = None,
    )
    for _ in iter:
        pass
    first.wait()
    second = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter],
        stderr = None,
    )
    second.wait()
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect_err("expected reuse error");
        assert!(
            err.to_string().contains("already bound"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn grpc_rejects_negative_max_retries() {
        let err = crate::axl_check!(
            r#"bazel.build_events.grpc(uri = "http://localhost:1", max_retries = -1)"#
        )
        .expect_err("expected validation error")
        .to_string();
        assert!(
            err.contains("max_retries") && err.contains(">= 0"),
            "unexpected error: {err}"
        );
    }

    /// Omitting the knob is the path that defers to
    /// `ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES`, so it must stay valid.
    #[test]
    fn grpc_accepts_omitted_buffer_bytes() {
        crate::axl_check!(r#"bazel.build_events.grpc(uri = "grpcs://bes.example.com")"#)
            .expect("omitting retry_max_buffer_bytes should validate");
    }

    #[test]
    fn grpc_rejects_zero_buffer_bytes() {
        let err = crate::axl_check!(
            r#"bazel.build_events.grpc(uri = "http://localhost:1", retry_max_buffer_bytes = 0)"#
        )
        .expect_err("expected validation error")
        .to_string();
        assert!(
            err.contains("retry_max_buffer_bytes") && err.contains("> 0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn grpc_rejects_malformed_retry_min_delay() {
        let err = crate::axl_check!(
            r#"bazel.build_events.grpc(uri = "http://localhost:1", retry_min_delay = "garbage")"#
        )
        .expect_err("expected validation error")
        .to_string();
        assert!(err.contains("retry_min_delay"), "unexpected error: {err}");
    }

    #[test]
    fn grpc_rejects_malformed_timeout() {
        let err = crate::axl_check!(
            r#"bazel.build_events.grpc(uri = "http://localhost:1", timeout = "garbage")"#
        )
        .expect_err("expected validation error")
        .to_string();
        assert!(err.contains("timeout"), "unexpected error: {err}");
    }

    #[test]
    fn grpc_accepts_full_knob_set() {
        crate::axl_check!(
            r#"bazel.build_events.grpc(
    uri = "grpcs://bes.example.com",
    metadata = {"x-auth": "tok"},
    max_retries = 0,
    retry_min_delay = "500ms",
    retry_max_buffer_bytes = 1048576,
    timeout = "30s",
    max_event_bytes = 16777216,
)"#
        )
        .expect("snippet should validate");
    }

    #[test]
    fn grpc_rejects_a_max_event_bytes_under_64_kib() {
        let err = crate::axl_check!(
            r#"bazel.build_events.grpc(uri = "http://localhost:1", max_event_bytes = 1000)"#
        )
        .expect_err("expected validation error")
        .to_string();
        assert!(
            err.contains("max_event_bytes must be at least 65536 (64 KiB), got 1000"),
            "unexpected error: {err}"
        );
    }

    /// The event size limit is the gRPC sink's alone: the build takes none.
    #[test]
    fn build_takes_no_max_event_bytes() {
        let err = crate::test::eval(
            r#"
def _impl(ctx):
    ctx.bazel.build(max_event_bytes = 65536)
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect_err("max_event_bytes is not a parameter of ctx.bazel.build");
        assert!(
            format!("{err:?}").contains("max_event_bytes"),
            "unexpected error: {err:?}"
        );
    }

    /// How many of basil's `oversized_command_line` action's 2000 arguments
    /// reach an iterator, with gRPC sinks of the given `max_event_bytes` (a
    /// Starlark list, None for unset) bound to the same build.
    fn command_line_kept(max_event_bytes: &str) -> u8 {
        use std::time::Duration;
        let code = format!(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator(kinds = ["action_executed"])
    sinks = [
        bazel.build_events.grpc(
            uri = "http://127.0.0.1:1",
            max_retries = 0,
            retry_min_delay = "0s",
            max_event_bytes = n,
        )
        for n in {max_event_bytes}
    ]
    build = ctx.bazel.build(
        flags = ["--scenario=oversized_command_line"],
        build_events = [iter] + sinks,
        stderr = None,
    )
    kept = [len(event.payload.command_line) for event in iter]
    build.wait()
    for sink in sinks:
        sink.wait()
    if len(kept) != 1:
        return 255
    # Hundreds of arguments, so the exit code holds the count.
    return kept[0] // 10

Test = task(implementation = _impl)
"#
        );
        let result = crate::test::with_timeout(Duration::from_secs(60), move || {
            crate::test::eval(&code).with_fake_bazel().run_task(0)
        })
        .expect("test hung");
        result.expect("run_task").expect("an exit code")
    }

    /// Of the event-size limits of a build's gRPC sinks, the smallest holds
    /// the whole stream, an unset one counting as 4 MiB; with none, 4 MiB.
    /// Each argument is 102 bytes encoded, and 4 KiB of each limit is left
    /// for the request wrapping the event.
    #[test]
    fn the_smallest_grpc_sink_limit_holds_the_stream() {
        let tenths = |limit: usize| ((limit - 4096) / 102 / 10) as u8;
        assert_eq!(command_line_kept("[]"), 200, "no gRPC sink: 4 MiB");
        assert_eq!(command_line_kept("[None]"), 200, "unset: 4 MiB");
        let kept = command_line_kept("[131072]");
        assert!(
            (tenths(131072) - 1..=tenths(131072)).contains(&kept),
            "one sink at 128 KiB kept {kept}0"
        );
        let kept = command_line_kept("[None, 131072, 98304]");
        assert!(
            (tenths(98304) - 1..=tenths(98304)).contains(&kept),
            "the 96 KiB sink of three kept {kept}0"
        );
    }

    #[test]
    fn iterator_rejects_empty_kinds() {
        let err = crate::axl_check!(r#"bazel.build_events.iterator(kinds = [])"#)
            .expect_err("expected validation error")
            .to_string();
        assert!(err.contains("kinds"), "unexpected error: {err}");
    }

    #[test]
    fn iterator_rejects_unknown_kind_string() {
        let err = crate::axl_check!(r#"bazel.build_events.iterator(kinds = ["bogus"])"#)
            .expect_err("expected validation error")
            .to_string();
        assert!(err.contains("bogus"), "unexpected error: {err}");
    }

    #[test]
    fn iterator_accepts_kind_strings() {
        crate::axl_check!(
            r#"bazel.build_events.iterator(kinds = ["target_completed", "named_set_of_files"])"#
        )
        .expect("snippet should validate");
    }

    /// The regression this branch exists for: a `kinds=` filter naming a
    /// payload the way `type(event.payload)` reports it delivers exactly
    /// those events, and the same literal reads the payload test. Before the
    /// fix `"action_executed"` was rejected outright and the accepted
    /// `"action_completed"` never matched `type(event.payload)`.
    #[test]
    fn kinds_filter_delivers_the_payload_type_it_names() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator(kinds = ["action_executed"])
    build = ctx.bazel.build(
        flags = ["--scenario=action_and_named_set"],
        build_events = [iter],
        stderr = None,
    )
    count = 0
    matched = 0
    for event in iter:
        count += 1
        if type(event.payload) == "action_executed":
            matched += 1
    build.wait()
    if count != 1: return 1
    if matched != 1: return 2
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// The legacy spelling keeps selecting the same payload, so a
    /// third-party `.aspect/*.axl` filter does not change behavior.
    #[test]
    fn the_legacy_action_completed_alias_selects_the_same_events() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator(kinds = ["action_completed"])
    build = ctx.bazel.build(
        flags = ["--scenario=action_and_named_set"],
        build_events = [iter],
        stderr = None,
    )
    count = 0
    for event in iter:
        count += 1
        if type(event.payload) != "action_executed": return 3
        if event.kind != "action_completed": return 4
    build.wait()
    if count != 1: return 1
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// `kinds=` drops non-matching events before yielding.
    #[test]
    fn iterator_kinds_filter_drops_non_matching_events() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator(kinds = ["build_finished"])
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter],
        stderr = None,
    )
    count = 0
    finished = 0
    for event in iter:
        count += 1
        if event.kind == "build_finished":
            finished += 1
    build.wait()
    if count != 1: return 1
    if finished != 1: return 2
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// `iter.drain()` ends iteration early, idempotently.
    #[test]
    fn iterator_drain_terminates_early() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter],
        stderr = None,
    )
    iter.drain()
    iter.drain()
    seen = 0
    for _ in iter:
        seen += 1
    build.wait()
    if not iter.done: return 1
    if seen != 0: return 2
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// Fresh sink: `done/failed/error` defaults, and `wait()` on an idle
    /// sink is a no-op.
    #[test]
    fn sink_attrs_default_before_bind() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    sink = bazel.build_events.grpc(uri = "grpcs://example.com")
    if sink.done: return 1
    if sink.failed: return 2
    if sink.error != None: return 3
    sink.wait()
    if sink.done: return 4
    return 0

Test = task(implementation = _impl)
"#,
        )
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
    }

    /// gRPC sink with an unparseable URI surfaces `failed = True` and a
    /// non-empty `error` after `wait()`; bazel's exit is unaffected.
    #[test]
    fn sink_grpc_failure_surfaces_on_wait() {
        use std::time::Duration;
        // Generous timeout: this runs concurrently with the engine::grpc
        // e2e server tests (same `grpc` filter), and pool contention can
        // stretch a normally sub-second run well past 15s.
        let result = crate::test::with_timeout(Duration::from_secs(60), || {
            crate::test::eval(
                r#"
def _impl(ctx):
    iter = bazel.build_events.iterator()
    sink = bazel.build_events.grpc(
        uri = "not a uri",
        max_retries = 0,
        retry_min_delay = "0s",
    )
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter, sink],
        stderr = None,
    )
    for _ in iter: pass
    status = build.wait()
    if not status.success: return 1
    if status.code != 0: return 2
    sink.wait()
    if not sink.done: return 3
    if not sink.failed: return 4
    if sink.error == None: return 5
    return 0

Test = task(implementation = _impl)
"#,
            )
            .with_fake_bazel()
            .run_task(0)
        })
        .expect("test hung");
        assert_eq!(result.expect("run_task"), Some(0));
    }

    /// Re-binding a Live sink without an intervening `wait()` errors.
    #[test]
    fn sink_rejects_double_bind_while_live() {
        let err = crate::test::eval(
            r#"
def _impl(ctx):
    sink = bazel.build_events.grpc(uri = "not a uri", max_retries = 0, retry_min_delay = "0s")
    iter = bazel.build_events.iterator()
    first = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter, sink],
        stderr = None,
    )
    iter2 = bazel.build_events.iterator()
    second = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [iter2, sink],
        stderr = None,
    )
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect_err("expected Live-rebind error");
        assert!(
            err.to_string().contains("still Live"),
            "unexpected error: {err}"
        );
    }
}
