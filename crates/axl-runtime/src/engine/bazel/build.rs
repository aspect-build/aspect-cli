use crate::errln;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
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

use axl_proto::build_event_stream::BuildEvent;

use crate::engine::r#async::rt::AsyncRuntime;
use crate::engine::cancellation::Signals;
use crate::engine::children::{self, Bound, Recv, recv_cancellable};
use tokio_util::sync::CancellationToken;

use super::iter::ExecLogIter;
use super::iter::ExecutionLogIterator;
use super::iter::WorkspaceEventIterator;
use super::sink::execlog::ExecLogSink;
use super::sink::grpc;
use super::sink::retry::{RetryConfig, SinkOutcome, SinkStats};
use super::sink::tracing as tracing_sink;
use super::stream::ExecLogStream;
use super::stream::Subscriber;
use super::stream::SubscriberFilter;
use super::stream::WorkspaceEventStream;
use super::stream::{BuildEventEnvelope, BuildEventStream};

/// The Bazel flag naming where the compact execution log is written. Read off the
/// command line to detect an already-requested log, and added when this call is
/// the one requesting it.
const EXECUTION_LOG_COMPACT_FILE: &str = "--execution_log_compact_file";

/// Subscribe each `execution_log.iterator()` handle to `stream`.
///
/// Unwinds on failure — a handle passed twice, or reused from an earlier build,
/// errors on its second bind with earlier handles already subscribed. They have
/// to be released here because the caller returns before anything else would
/// drop them: the reader would then block against subscribers no AXL code can
/// reach, leaving a sibling `File` sink's writer waiting on entries that never come.
fn bind_execlog_iters(stream: &ExecLogStream, iters: &[ExecLogIter]) -> io::Result<()> {
    for (bound, iter) in iters.iter().enumerate() {
        let bind = || -> io::Result<()> {
            let recv = stream.receiver().ok_or_else(|| {
                io::Error::other("execution log stream has no subscriber to clone")
            })?;
            iter.bind(recv).map_err(io::Error::other)
        };
        if let Err(err) = bind() {
            for already in &iters[..bound] {
                already.release();
            }
            return Err(err);
        }
    }
    Ok(())
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
        Writable::Stdout(arc) => {
            let guard = arc.lock().unwrap();
            let borrowed = guard.borrow();
            let s = borrowed.as_ref().ok_or_else(closed)?;
            dup_fd(s)
        }
        Writable::Stderr(arc) => {
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

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.BuildStatus>")]
pub struct BuildStatus {
    success: bool,
    code: Option<i32>,
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
    #[starlark(attribute)]
    fn code<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<i32>> {
        Ok(NoneOr::from_option(
            this.downcast_ref::<BuildStatus>().unwrap().code,
        ))
    }
}

#[derive(Clone)]
enum SinkConfig {
    Grpc {
        uri: String,
        metadata: HashMap<String, String>,
        retry: RetryConfig,
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

pub struct FileSignal {
    result: Mutex<Option<Result<(), String>>>,
    cv: std::sync::Condvar,
}

impl FileSignal {
    pub fn new() -> Self {
        Self {
            result: Mutex::new(None),
            cv: std::sync::Condvar::new(),
        }
    }

    pub fn complete(&self, result: Result<(), String>) {
        let mut guard = self.result.lock().unwrap();
        if guard.is_none() {
            *guard = Some(result);
            self.cv.notify_all();
        }
    }

    fn is_complete(&self) -> bool {
        self.result.lock().unwrap().is_some()
    }

    fn wait(&self) -> Result<(), String> {
        let mut guard = self.result.lock().unwrap();
        while guard.is_none() {
            guard = self.cv.wait(guard).unwrap();
        }
        guard.as_ref().unwrap().clone()
    }
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
    pub fn new_grpc(uri: String, metadata: HashMap<String, String>, retry: RetryConfig) -> Self {
        Self::new(SinkConfig::Grpc {
            uri,
            metadata,
            retry,
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
    ) -> anyhow::Result<()> {
        let SinkConfig::Grpc {
            uri,
            metadata,
            retry,
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
            rt,
            stream.subscribe(),
            uri.clone(),
            metadata.clone(),
            invocation_id,
            retry.clone(),
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

    /// Subscribe the iterator's receiver. Must run before bazel opens the
    /// BEP FIFO so the early burst is buffered. The `kinds=` filter is
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

/// True when `event`'s payload kind is in `kinds`. A payload-less event never
/// matches a filter (there's no kind to test). Used to build the send-side
/// subscriber filter in `BuildEventIter::bind`.
fn event_kind_in(event: &BuildEvent, kinds: &HashSet<i32>) -> bool {
    let Some(payload) = event.payload.as_ref() else {
        return false;
    };
    kinds.contains(&payload_discriminant(payload))
}

/// Maps a payload variant to its proto field number — same integers the
/// `bazel.build.build_event.*` constants resolve to, so `kinds=` matches by
/// integer set lookup.
fn payload_discriminant(p: &axl_proto::build_event_stream::build_event::Payload) -> i32 {
    use axl_proto::build_event_stream::build_event::Payload;
    match p {
        Payload::Progress(_) => 3,
        Payload::Aborted(_) => 4,
        Payload::Started(_) => 5,
        Payload::UnstructuredCommandLine(_) => 12,
        Payload::StructuredCommandLine(_) => 13,
        Payload::OptionsParsed(_) => 14,
        Payload::WorkspaceStatus(_) => 16,
        Payload::Fetch(_) => 17,
        Payload::Configuration(_) => 19,
        Payload::Expanded(_) => 6,
        Payload::Configured(_) => 7,
        Payload::Action(_) => 8,
        Payload::NamedSetOfFiles(_) => 15,
        Payload::Completed(_) => 9,
        Payload::TestResult(_) => 10,
        Payload::TestSummary(_) => 20,
        Payload::TargetSummary(_) => 26,
        Payload::Finished(_) => 11,
        Payload::BuildToolLogs(_) => 21,
        Payload::BuildMetrics(_) => 22,
        Payload::WorkspaceInfo(_) => 25,
        Payload::BuildMetadata(_) => 24,
        Payload::ConvenienceSymlinksIdentified(_) => 27,
        Payload::ExecRequest(_) => 28,
        Payload::TestProgress(_) => 30,
    }
}

/// Optionally print the detected Bazel version and/or the exact command being
/// spawned, one `INFO:` line each, before a `bazel build`/`test`/`run`
/// invocation. Gated by the `--announce-bazel-version` / `--announce-bazel-command`
/// task flags, resolved in AXL and passed through as `announce`.
///
/// Styled in grey so the long `INFO: Spawning:` line reads as background
/// context next to bazel's own (undimmed) `INFO:` output. Falls back to
/// plain text when stderr isn't a TTY and we're not on a recognized CI host
/// — matching the gate used elsewhere in the runtime (see
/// `multi_phase::running_verb_color`).
pub(super) fn announce_spawn(
    announce: AnnounceSpawn,
    version: Option<&semver::Version>,
    cmd: &Command,
) {
    let (grey, reset) = grey_style();
    if announce.version {
        errln!("{grey}INFO: {}{reset}", version_line(version));
    }
    if announce.command {
        errln!("{grey}INFO: Spawning: {}{reset}", render_command(cmd));
    }
}

/// Return `(grey_prefix, reset)` ANSI escape pair for the announce lines.
///
/// Empty strings when stderr isn't a TTY and we're not on a recognized CI
/// host, so file-captured / piped output stays plain. CI hosts (GitHub
/// Actions, Buildkite, …) render ANSI in their log viewers even though
/// stderr is a non-TTY pipe — same heuristic as `running_verb_color`.
///
/// Uses 256-color grey (`38;5;244`) rather than SGR 2 (faint): GitHub
/// Actions' log viewer silently drops SGR 2, which is the bug the original
/// implementation hit. 256-color escapes are rendered by GHA, Buildkite,
/// and every TTY we ship to, and match the grey `tools/bazel` itself uses
/// for its `[tools/bazel]` trace line.
fn grey_style() -> (&'static str, &'static str) {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() || crate::ci::on_recognized_ci() {
        ("\x1b[38;5;244m", "\x1b[0m")
    } else {
        ("", "")
    }
}

/// The version `INFO:` text. `version` is `None` for a non-release build (see
/// [`super::info::parse_release`]), which notes the assume-latest behavior.
fn version_line(version: Option<&semver::Version>) -> String {
    match version {
        Some(v) => format!("Bazel {v}"),
        None => "Bazel development version (version-conditional flags assume latest)".to_string(),
    }
}

/// Render `cmd` as a space-joined `program arg…` line for display. Read back
/// from the fully assembled `Command`, so it shows the full argument set
/// aspect-cli passes Bazel (including the internal BES/execlog flags).
///
/// Secrets (env-var values, request headers, URL credentials) are redacted via
/// [`super::stream::redaction::redact_command_args`] — the same rules the BES
/// sink redaction uses — since this line is printed to CI logs by default.
/// Args are space-joined for readability, not shell-quoted: a value with a
/// space (or a `<REDACTED>` placeholder) is not guaranteed copy-paste-safe.
fn render_command(cmd: &Command) -> String {
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let redacted = super::stream::redaction::redact_command_args(args.iter().map(String::as_str));
    std::iter::once(cmd.get_program().to_string_lossy().into_owned())
        .chain(redacted)
        .collect::<Vec<_>>()
        .join(" ")
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
    /// its filter plus an `Arc` to the shared bind state.
    #[allocative(skip)]
    execlog_iters: Vec<ExecLogIter>,

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

    #[allocative(skip)]
    span: RefCell<tracing::Span>,
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
        children::wait_with(&self.bound, timeout, || self.child.borrow_mut().try_wait())
    }

    // TODO: this should return a thiserror::Error
    pub fn spawn(
        verb: &str,
        targets: impl IntoIterator<Item = String>,
        (build_events, sinks, iters): (bool, Vec<BuildEventSink>, Vec<BuildEventIter>),
        (execution_logs, execlog_sinks, execlog_iters): (bool, Vec<ExecLogSink>, Vec<ExecLogIter>),
        workspace_events: bool,
        flags: Vec<String>,
        startup_flags: Vec<String>,
        stdout: Stdio,
        stderr: Stdio,
        directory: Option<String>,
        announce: AnnounceSpawn,
        signals: Arc<Signals>,
        cancellation: &CancellationToken,
        rt: AsyncRuntime,
    ) -> Result<Build, std::io::Error> {
        let (pid, version) = super::info::server_info(&signals)?;

        let span = tracing::info_span!(
            "ctx.bazel.build",
            build_events = build_events,
            workspace_events = workspace_events,
            execution_logs = execution_logs,
            flags = ?flags
        );
        let _enter = span.enter();

        let targets: Vec<String> = targets.into_iter().collect();

        // Read before `flags` is moved into the command: a deployment wired to an
        // Aspect BES backend, a Workflows runner, or a generated bazelrc already
        // asks Bazel for the compact log, and the backend reads the file at that
        // path. Adding a second `--execution_log_compact_file` below would win on
        // last-write-wins and silently repoint Bazel away from it.
        //
        // An empty value clears the flag rather than naming a file, so it is not a
        // request; treating it as a path would have the reader tail "" and report a
        // clean empty stream. A relative value is joined to the spawn's directory,
        // because that is what Bazel resolves it against while the reader runs from
        // the CLI's own cwd.
        let injected_execlog_path = bazelrc::last_flag_value(&flags, EXECUTION_LOG_COMPACT_FILE)
            .filter(|path| !path.is_empty())
            .map(|path| match &directory {
                Some(dir) if std::path::Path::new(&path).is_relative() => {
                    std::path::Path::new(dir).join(path)
                }
                _ => PathBuf::from(path),
            });

        let mut cmd = super::bazel_command();
        cmd.args(startup_flags);
        cmd.arg(verb);
        cmd.args(flags);

        if let Some(directory) = directory {
            cmd.current_dir(directory);
        }

        // File sinks share the BES reader's raw-bytes path (preserves
        // bazel's byte-for-byte output); gRPC sinks run as broadcaster
        // subscriber threads.
        let mut file_sinks: Vec<(String, Arc<FileSignal>)> = vec![];
        let mut grpc_sinks: Vec<BuildEventSink> = vec![];
        for sink in sinks {
            if let Some(path) = sink.file_path() {
                let signal = sink.bind_file().map_err(io::Error::other)?;
                file_sinks.push((path, signal));
            } else {
                grpc_sinks.push(sink);
            }
        }

        // Reserve the BES FIFO inode now (before `cmd.spawn()`) so bazel can
        // find the path when it opens the BEP file. The reader-side thread
        // is started later — once we have the spawned child's pid in hand
        // for the per-invocation liveness check.
        //
        // Bazel's default action publishing (failed actions only) is all any
        // consumer of the stream reads; per-action data comes from the
        // execution log, so `--build_event_publish_all_actions` is not asked
        // for.
        let bes_path = if build_events {
            let p = BuildEventStream::reserve_path()?;
            cmd.arg("--build_event_binary_file_upload_mode=fully_async")
                .arg("--build_event_binary_file")
                .arg(&p);
            Some(p)
        } else {
            None
        };

        let workspace_event_stream = if workspace_events {
            let (out, stream) = WorkspaceEventStream::spawn_with_pipe(pid)?;
            cmd.arg("--experimental_workspace_rules_log_file").arg(&out);
            Some(stream)
        } else {
            None
        };

        // Split execlog sinks: compact paths go to the tee reader inside the stream thread;
        // decoded File sinks are spawned separately against the decoded receiver.
        let mut compact_paths: Vec<String> = vec![];
        let mut decoded_sinks: Vec<ExecLogSink> = vec![];
        for sink in execlog_sinks {
            match &sink {
                ExecLogSink::CompactFile { path } => compact_paths.push(path.clone()),
                ExecLogSink::File { .. } => decoded_sinks.push(sink),
            }
        }

        // Reserved before the spawn so bazel can be told where to write, but the
        // reader is started after it — once the client pid exists (see the BES
        // reader below for the same split, and why).
        let execlog_path = if execution_logs {
            match injected_execlog_path {
                // Something already named the path and put the flag on the command
                // line. Tail that file and leave the flag alone; every CompactFile
                // sink is served by the tee rather than by Bazel writing directly,
                // so each still gets its copy — except one naming this very file,
                // which Bazel is already writing and the tee would truncate under it.
                Some(out) => {
                    compact_paths.retain(|path| std::path::Path::new(path) != out);
                    Some(out)
                }
                // Nothing asked yet, so this call owns the flag. A CompactFile sink
                // lends its path, letting Bazel write straight to the caller's
                // destination with no temp file or tee step for that copy.
                None => {
                    let direct_path = if compact_paths.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(compact_paths.remove(0)))
                    };
                    let out = ExecLogStream::reserve_path(direct_path);
                    cmd.arg(EXECUTION_LOG_COMPACT_FILE).arg(&out);
                    Some(out)
                }
            }
        } else {
            None
        };

        // A log at the reserved path from an *earlier* run has to go before Bazel
        // starts, because `galvanize::StreamingFile::open` polls only for the
        // path's existence. Left in place the reader opens the stale file
        // immediately, races Bazel's truncation of it and loses: it delivers the
        // previous build's entries — spawns that never ran, inputs that no longer
        // exist — and misses this build's.
        //
        // Rare while every reader got a fresh UUID temp path; routine now that a
        // path already on the command line is reused, which on a Workflows runner
        // or an Aspect-wired deployment is one fixed path per build. Removing it
        // is what makes the existence poll mean "wait for *this* build's log";
        // Bazel creates the file itself, so there is nothing to put back.
        if let Some(path) = &execlog_path {
            if let Err(err) = std::fs::remove_file(path) {
                if err.kind() != io::ErrorKind::NotFound {
                    errln!(
                        "WARNING: could not clear the previous execution log at {}: {err}. \
                         Entries from an earlier build may be reported as this one's.",
                        path.display(),
                    );
                }
            }
        }

        cmd.arg("--"); // separate flags from target patterns (not strictly necessary for build & test verbs but good form)
        cmd.args(targets);

        crate::trace!("exec: {:?}", cmd.get_args());
        announce_spawn(announce, version.as_ref(), &cmd);

        cmd.stdout(stdout);
        cmd.stderr(stderr);
        cmd.stdin(Stdio::null());

        // Bound to `cancellation` (the root unless the task said otherwise):
        // cancelling it sends the client one SIGINT.
        let (child, bound) = children::spawn(
            &mut cmd,
            &signals,
            cancellation,
            None,
            children::Kind::Bazel,
        )
        .map_err(|e| io::Error::other(format!("failed to spawn bazel: {e}")))?;

        // Now that we have the spawned child's pid, start the BES reader.
        // The child pid is the per-invocation liveness signal the BES thread
        // uses to detect aspect-build/aspect-cli#1060 — a hung post-
        // REMOTE_CACHE_EVICTED state. The server (daemon) pid passed to
        // galvanize stays alive across invocations and cannot signal
        // end-of-build, which is why we want a separate per-invocation pid.
        let build_event_stream = match bes_path {
            Some(p) => Some(BuildEventStream::spawn(p, pid, child.id(), file_sinks)?),
            None => None,
        };

        // Same two-pid split for the execution log: the daemon writes the file, but
        // only the client can say the invocation is over and none is coming.
        // A decoded `File` sink or an iterator handle is a consumer that must see
        // every entry, so the producer blocks rather than dropping. Both are known
        // here, before Bazel writes anything, which is the whole reason the iterator
        // is passed in rather than fetched from the returned handle.
        let lossless = !decoded_sinks.is_empty() || !execlog_iters.is_empty();
        let mut execlog_stream = match execlog_path {
            Some(p) => Some(ExecLogStream::spawn_with_file(
                p,
                pid,
                child.id(),
                compact_paths,
                lossless,
            )?),
            None => None,
        };

        // Bind subscribers BEFORE the BES reader unblocks. The reader is
        // currently parked in `Pipe::open` waiting for bazel's JVM startup
        // to open the FIFO write end, so subscriptions registered here
        // win the warm-daemon race against the early event burst.
        if !iters.is_empty() {
            let stream = build_event_stream.as_ref().ok_or_else(|| {
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
            let stream = build_event_stream.as_ref().unwrap();
            for sink in grpc_sinks {
                sink.bind_grpc(rt.clone(), stream, invocation_id.clone())
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

        // Decoded execlog file sinks belong to the execlog stream — joined
        // (and write errors propagated) inside `execlog_stream.join()`.
        if let Some(stream) = execlog_stream.as_mut() {
            for sink in decoded_sinks {
                if let ExecLogSink::File { path } = sink {
                    let recv = stream.receiver().ok_or_else(|| {
                        io::Error::other("execution log stream has no subscriber to clone")
                    })?;
                    stream.attach_file_sink(ExecLogSink::spawn_file(recv, path));
                }
            }
        }

        // One receiver clone each: the channel is a broadcast ring, so the handles
        // read the same entries the file sinks do rather than competing for them.
        if !execlog_iters.is_empty() {
            let stream = execlog_stream.as_ref().ok_or_else(|| {
                // `partition_execution_log` turns the stream on for any list, so a
                // list holding a handle always has one. Stated, not unwrapped.
                io::Error::other(
                    "ctx.bazel.build/test: execution_log list contained `iterator()` handles \
                     but no execution log stream is configured",
                )
            })?;
            bind_execlog_iters(stream, &execlog_iters)?;
        }

        // Every consumer has cloned what it needs, so the stream must not go on
        // holding an unread subscriber of its own — see `ExecLogStream::recv`. The
        // exception is a stream with no consumer at all, whose subscriber is what
        // a later `build.execution_logs()` would take.
        if lossless {
            if let Some(stream) = execlog_stream.as_mut() {
                stream.take_initial_subscriber();
            }
        }
        // The tracing sink only emits via `tracing::event!` and never fails;
        // detach its JoinHandle so `build.wait()` stays bazel-only.
        if build_events {
            let _ = tracing_sink::Tracing::spawn(build_event_stream.as_ref().unwrap().subscribe());
        }

        drop(_enter);
        Ok(Self {
            child: RefCell::new(child),
            build_event_stream: RefCell::new(build_event_stream),
            workspace_event_stream: RefCell::new(workspace_event_stream),
            execlog_stream: RefCell::new(execlog_stream),
            execlog_iters,
            sink_invocation_id: RefCell::new(sink_invocation_id),
            bound,
            signals,
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
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: values::Heap<'v>) -> bool {
        matches!(attribute, "sink_invocation_id")
    }
}

#[starlark_module]
pub(crate) fn build_methods(registry: &mut MethodsBuilder) {
    // Creates an iterable `ExecutionLogIterator` type. Takes the stream's one
    // subscriber rather than cloning it, so it can be called only once and only
    // when nothing else reads the log — see the error below.
    fn execution_logs<'v>(this: values::Value<'v>) -> anyhow::Result<ExecutionLogIterator> {
        let build = this.downcast_ref::<Build>().unwrap();
        let mut execlog_stream = build.execlog_stream.borrow_mut();
        let execlog_stream = execlog_stream.as_mut().ok_or(anyhow::anyhow!(
            "call `ctx.bazel.build` with `execution_log = true` in order to receive execution log events."
        ))?;

        // Take the stream's subscriber rather than clone it. Cloning leaves the
        // original in place with nobody reading it, which pins the broadcast
        // ring's slowest tail at entry zero and silently truncates this iterator
        // at the channel capacity — 1000 entries, however fast the caller reads.
        let recv = execlog_stream
            .take_initial_subscriber()
            .ok_or(anyhow::anyhow!(
                "this build's execution log already has a consumer: either \
                 `execution_logs()` was called twice, or the build was configured \
                 with a decoded `execution_log.file(...)` sink or an \
                 `execution_log.iterator()` handle. A second subscriber cannot be \
                 added — nothing would drain it, and an undrained subscriber stalls \
                 the log for every other consumer. Pass an \
                 `execution_log.iterator()` handle in `execution_log=[...]` and \
                 iterate that: it can be combined with sinks, and it is the only \
                 form guaranteed not to drop entries."
            ))?;
        Ok(ExecutionLogIterator::new(recv))
    }

    // Creates an iterable `WorkspaceEventIterator` type.
    // Every call to this function will return a new iterator.
    fn workspace_events<'v>(this: values::Value<'v>) -> anyhow::Result<WorkspaceEventIterator> {
        let build = this.downcast_ref::<Build>().unwrap();
        let event_stream = build.workspace_event_stream.borrow();
        let event_stream = event_stream.as_ref().ok_or(anyhow::anyhow!(
            "call `ctx.bazel.build` with `workspace_events = true` in order to receive workspace events."
        ))?;

        Ok(WorkspaceEventIterator::new(event_stream.receiver()))
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
    /// going through its token.
    fn interrupt<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        // Nothing after a reap: the pid may be someone else's by then.
        if !build.bound.exited() {
            let pid = build.child.borrow().id();
            children::os::interrupt(children::Target::Pid(pid));
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
            children::try_wait(&mut build.child.borrow_mut(), &build.bound)?
        } else {
            build.wait_child(Some(Duration::from_millis(timeout_ms as u64)))?
        };
        Ok(match status {
            Some(status) => NoneOr::Other(BuildStatus {
                success: status.success(),
                code: status.code(),
            }),
            None => NoneOr::None,
        })
    }

    /// Block until the Bazel invocation finishes and return a `BuildStatus`.
    ///
    /// After `wait()` returns, the execution log pipe has been closed and the
    /// producer thread has exited. Calling `execution_logs()` after `wait()`
    /// will fail — the stream is consumed as part of the wait. Iterate
    /// `execution_logs()` **before** calling `wait()` if you need to process
    /// entries.
    ///
    /// The same applies, more sharply, to an `execution_log.iterator()` handle:
    /// `wait()` releases every handle still bound to this build before joining
    /// the log's producer, so a handle stops yielding from here on and anything
    /// it had not been drained of is gone. That is deliberate — a bound handle is
    /// a subscriber the producer blocks for, so a handle nobody drained would
    /// otherwise park the producer and hang this `wait()` — but it does mean the
    /// drain belongs *before* the call, not after.
    ///
    /// `build_events()` remains usable after `wait()` for replaying historical
    /// events, because the build event stream retains its buffer.
    ///
    /// The wait ends the task instead if `ctx.cancellation.root` is
    /// cancelled meanwhile and no `intercept()` is in effect; the client has
    /// its SIGINT by then.
    fn wait<'v>(this: values::Value<'v>) -> anyhow::Result<BuildStatus> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;

        // Re-enter the span so trace coverage includes the full build lifecycle
        let span = build.span.borrow().clone();
        let _enter = span.enter();

        let result = build
            .wait_child(None)?
            .expect("a wait without a timeout reports a status");

        // Wait for BES stream to complete.
        // Note: We don't take() the stream here so that build_events() can still
        // be called after wait() to get historical events.
        if let Some(ref mut event_stream) = *build.build_event_stream.borrow_mut() {
            match event_stream.join() {
                Ok(_) => {}
                Err(err) => anyhow::bail!("build event stream thread error: {}", err),
            }
        }

        // Wait for Workspace event stream to complete.
        let workspace_event_stream = build.workspace_event_stream.take();
        if let Some(workspace_event_stream) = workspace_event_stream {
            match workspace_event_stream.join() {
                Ok(_) => {}
                Err(err) => anyhow::bail!("workspace event stream thread error: {}", err),
            }
        };

        // Release any iterator handle still bound. A bound handle forces the
        // producer's blocking sends, so one the task never drained would park the
        // reader thread on a full channel and hang the join below. The AXL side
        // drains before calling `wait()` (see `bazel/exec_log.axl`); this is the
        // net under a task that does not, trading dropped entries for a build
        // that ends.
        for iter in &build.execlog_iters {
            iter.release();
        }

        // Wait for Execlog stream to complete.
        let execlog_stream = build.execlog_stream.take();
        if let Some(execlog_stream) = execlog_stream {
            match execlog_stream.join() {
                Ok(_) => {}
                Err(err) => anyhow::bail!("execlog stream thread error: {}", err),
            }
        };

        // Drop the span to end the trace
        drop(build.span.replace(tracing::Span::none()));

        Ok(BuildStatus {
            success: result.success(),
            code: result.code(),
        })
    }
}

#[cfg(test)]
mod tests {
    //! End-to-end coverage of `ctx.bazel.build` via the `basil` fake-bazel
    //! binary, selected per-test via `--scenario=<name>`.

    use axl_proto::tools::protos::ExecLogEntry;
    use prost::Message;

    /// Serializes the tests that drive the execution log through basil.
    ///
    /// They set `BASIL_SERVER_PID`, which is process-wide, so running two at
    /// once would have one clobber the other's daemon stand-in and the loser
    /// would read a dead pid — which is a passing result for some of these tests
    /// and a failing one for others, i.e. flaky either way.
    static EXECLOG_BASIL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `body` with a live process standing in for the Bazel daemon.
    ///
    /// The execlog reader asks two pids whether more bytes are coming: the
    /// client, whose death means no log is coming at all, and the daemon, which
    /// holds the file open while writing it. basil's `info` invocation is a
    /// separate short-lived process, so by the time the reader looks the pid it
    /// reported is already reaped — `galvanize::StreamingFile::open` then reads a
    /// not-yet-created file as "the writer is gone" and ends the stream empty,
    /// before basil has written a byte. A live pid that holds nothing open is the
    /// right stand-in: it keeps `open` waiting for the file, and end-of-file
    /// still terminates the read cleanly.
    #[cfg(unix)]
    fn with_daemon_stand_in<T>(body: impl FnOnce() -> T) -> T {
        let _serial = EXECLOG_BASIL.lock().unwrap_or_else(|e| e.into_inner());
        let mut daemon = std::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("spawn the daemon stand-in");
        // SAFETY: process-wide env mutation, serialized against the other
        // execlog-through-basil tests by `EXECLOG_BASIL`. A concurrent non-execlog
        // bazel test reading this as its server pid gets a live process that holds
        // none of its paths open — the same answer the dead pid it reads today
        // produces.
        unsafe {
            std::env::set_var("BASIL_SERVER_PID", daemon.id().to_string());
        }
        let out = body();
        let _ = daemon.kill();
        let _ = daemon.wait();
        // SAFETY: as above.
        unsafe {
            std::env::remove_var("BASIL_SERVER_PID");
        }
        out
    }

    /// `build.execution_logs()` must not stop at the decoded channel's capacity.
    ///
    /// The stream hands this iterator its one subscriber rather than cloning it,
    /// because an unread clone left behind caps the ring (see
    /// `ExecLogStream::recv`). With one, a caller receives exactly the first
    /// `CHANNEL_CAPACITY` entries and then a clean end of stream — no error, and
    /// no dependence on how fast it reads. In a real log, whose leading records
    /// are inputs rather than spawns, such a prefix can contain no spawns at all,
    /// so the failure is a confidently wrong answer rather than a short one.
    ///
    /// What this does *not* assert is that every entry arrives. `execution_log =
    /// True` is the lossy strategy by contract: the reader drops entries that get
    /// more than the capacity ahead of the consumer, and against a fake bazel
    /// whose log is already complete on disk it certainly does. The guarantee is
    /// that there is no fixed ceiling. `execution_log.iterator()` is the lossless
    /// form, covered below.
    #[cfg(unix)]
    #[test]
    fn execution_logs_is_not_capped_at_the_channel_capacity() {
        let capacity = super::super::stream::execlog::channel_capacity() as u32;

        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("report.txt");

        let exit = with_daemon_stand_in(|| {
            crate::test::eval(&format!(
                r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
        stderr = None,
    )
    seen = 0
    highest = 0
    for entry in build.execution_logs():
        if entry.id <= highest:
            return 2  # ids must advance; a replay or reorder is a different bug
        highest = entry.id
        seen += 1
    status = build.wait()
    if not status.success: return 1
    ctx.std.fs.create({report:?}).write("%d %d\n" % (seen, highest))
    return 0

Test = task(implementation = _impl)
"#,
                report = report.to_str().unwrap(),
            ))
            .with_fake_bazel()
            .run_task(0)
            .expect("run_task")
        });
        assert_eq!(exit, Some(0), "the build should have succeeded");

        let text = std::fs::read_to_string(&report).expect("the task should have reported");
        let mut parts = text.split_whitespace();
        let seen: u32 = parts.next().unwrap().parse().unwrap();
        let highest: u32 = parts.next().unwrap().parse().unwrap();

        assert!(
            seen > capacity,
            "expected more than the channel capacity ({capacity}); got {seen}, which is \
             the prefix an unread subscriber would cap this at",
        );
        assert!(
            highest > capacity,
            "expected entries from beyond the capacity, not just a bigger prefix; \
             highest id was {highest}",
        );
    }

    /// The lossless form: a decoded `file()` sink and a live `iterator()` handle on
    /// one build, over a log longer than the channel capacity.
    ///
    /// Both consumers must see **every** entry — a handle switches the reader to
    /// blocking sends, and the channel is a broadcast, so the two do not compete.
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
            return 2  # a gap: the lossless path dropped something
    status = build.wait()
    if not status.success: return 1
    if seen != 3000: return 3
    return 0

Test = task(implementation = _impl)
"#,
            sink = sink_path.to_str().unwrap(),
        );

        // The timeout is here to catch the old deadlock, not to bound a healthy
        // run, which finishes in well under a second.
        let result = with_daemon_stand_in(move || {
            crate::test::with_timeout(Duration::from_secs(60), move || {
                crate::test::eval(&script).with_fake_bazel().run_task(0)
            })
        });

        match result {
            None => panic!("timed out: a file sink plus a live handle deadlocked"),
            Some(exit) => assert_eq!(
                exit.expect("run_task"),
                Some(0),
                "a handle alongside a file sink must see all 3000 entries, contiguously",
            ),
        }

        // The sink's writer is joined inside `wait()`, so its file is complete by
        // now: 3000 length-delimited entries, not a prefix.
        let written = std::fs::read(&sink_path).expect("the sink should have written a file");
        let mut buf = written.as_slice();
        let mut count = 0;
        while !buf.is_empty() {
            let entry = ExecLogEntry::decode_length_delimited(&mut buf)
                .expect("the sink should hold whole entries");
            count += 1;
            assert_eq!(entry.id, count, "the sink should hold a contiguous run");
        }
        assert_eq!(count, 3000, "the decoded file sink should be complete");
    }

    /// A deployment wired to an Aspect BES backend, a Workflows runner and a
    /// generated bazelrc each put `--execution_log_compact_file` on the command
    /// line themselves, and the backend reads the file at *that* path. Adding a
    /// second one wins on last-write-wins and silently repoints Bazel, so the
    /// consumer tails a path nothing writes and the backend's file never appears.
    #[test]
    fn an_already_requested_execution_log_is_reused_not_repointed() {
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
    # Zero means the flag was added a second time: bazel wrote the path above
    # and the reader tailed a freshly minted temp file instead.
    if seen != 3000: return 3
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );

        let exit =
            with_daemon_stand_in(move || crate::test::eval(&script).with_fake_bazel().run_task(0));
        assert_eq!(
            exit.expect("run_task"),
            Some(0),
            "the reader must tail the path already on the command line",
        );
        assert!(
            asked.exists(),
            "bazel must still write the log where the deployment asked, at {}",
            asked.display(),
        );
    }

    /// `--execution_log_compact_file=` clears the flag rather than naming a
    /// file, so it is not a path to reuse.
    ///
    /// An rc file setting the flag for a config and a command line clearing it
    /// is an ordinary way to reach this. Read back as `Some("")` the reader
    /// tails `""`, which cannot exist, and reports a clean end of stream — every
    /// hook fires zero times and the build passes.
    #[cfg(unix)]
    #[test]
    fn an_empty_execution_log_flag_is_not_a_reusable_path() {
        let script = r#"
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
    # Zero means the empty value was taken for a path and tailed.
    if seen != 3000: return 3
    return 0

Test = task(implementation = _impl)
"#;
        let exit = with_daemon_stand_in(|| crate::test::eval(script).with_fake_bazel().run_task(0));
        assert_eq!(
            exit.expect("run_task"),
            Some(0),
            "a cleared flag must leave this call owning the log, not tailing \"\"",
        );
    }

    /// A relative `--execution_log_compact_file` is Bazel's to resolve, and
    /// Bazel resolves it against the cwd `Build::spawn` gives it.
    ///
    /// Used verbatim, the reader tails `<cli-cwd>/<path>` while Bazel writes
    /// `<directory>/<path>` — nothing to read, and a clean empty stream again.
    /// `format` and `gazelle` both pass a `directory` and both consume the log.
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
    # Zero means the reader tailed the path relative to the CLI's own cwd.
    if seen != 3000: return 3
    return 0

Test = task(implementation = _impl)
"#,
            dir = dir.path().to_str().unwrap(),
        );

        let exit =
            with_daemon_stand_in(|| crate::test::eval(&script).with_fake_bazel().run_task(0));
        assert_eq!(
            exit.expect("run_task"),
            Some(0),
            "the reader must tail the path bazel resolved it to",
        );
        assert!(
            dir.path().join("relative.zstd").exists(),
            "bazel writes a relative path under its own cwd",
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

    /// A log left at the reserved path by an earlier build is not this build's.
    ///
    /// `StreamingFile::open` polls for the path to exist, so a stale file is
    /// opened at once and read to its end while Bazel truncates and rewrites the
    /// path underneath — the reader keeps the old inode. The consumer is then
    /// handed the previous run's entries: spawns that never ran, inputs that are
    /// gone, and a `kinds = ["spawn"]` hook told about actions this build did not
    /// execute. Nothing reports an error; the build passes.
    ///
    /// The stale ids are 90001-upwards so the assertion names the build the
    /// entries came from rather than counting them. `execlog_after_bes` publishes
    /// this build's log two seconds after the event stream closes, which puts the
    /// reader's open firmly in the window where only the stale file exists — the
    /// race, made one-sided.
    ///
    /// A path reused across builds is the normal case this reaches: a Workflows
    /// runner or an Aspect-wired deployment names one, and the CLI now tails that
    /// path rather than minting a fresh one.
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
    if highest > 3000: return 2
    if seen != 3000: return 3
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );

        // The timeout catches a reader parked on the stale inode rather than
        // bounding a healthy run, which takes a little over two seconds.
        let result = with_daemon_stand_in(move || {
            crate::test::with_timeout(std::time::Duration::from_secs(120), move || {
                crate::test::eval(&script).with_fake_bazel().run_task(0)
            })
        });
        match result {
            None => panic!("timed out: the reader never reached this build's log"),
            Some(exit) => assert_eq!(
                exit.expect("run_task"),
                Some(0),
                "the consumer must receive this build's log, not the one already \
                 at the path",
            ),
        }
    }

    /// `build.execution_logs()` takes the stream's one subscriber rather than
    /// cloning it, so a second consumer is refused rather than silently capping
    /// the log for everyone.
    ///
    /// The documented breaking change of this API: a build configured with a
    /// decoded sink or an `execution_log.iterator()` handle — which is what any
    /// `exec_log_event` hook adds — can no longer also call this.
    #[cfg(unix)]
    #[test]
    fn execution_logs_is_refused_beside_an_iterator_handle() {
        let script = r#"
def _impl(ctx):
    entries = bazel.execution_log.iterator()
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [entries],
        stderr = None,
    )
    build.execution_logs()
    return 0

Test = task(implementation = _impl)
"#;
        let err = with_daemon_stand_in(|| {
            crate::test::eval(script)
                .with_fake_bazel()
                .run_task(0)
                .expect_err("a second consumer must be refused")
        });
        let err = err.to_string();
        assert!(
            err.contains("already has a consumer") && err.contains("execution_log.iterator()"),
            "the refusal should name the replacement: {err}",
        );
    }

    /// The same refusal for two calls on a build with no other consumer: the
    /// first took the subscriber, so there is none left to hand out.
    #[cfg(unix)]
    #[test]
    fn execution_logs_is_refused_a_second_time() {
        let script = r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
        stderr = None,
    )
    first = build.execution_logs()
    second = build.execution_logs()
    return 0

Test = task(implementation = _impl)
"#;
        let err = with_daemon_stand_in(|| {
            crate::test::eval(script)
                .with_fake_bazel()
                .run_task(0)
                .expect_err("the second call must be refused")
        });
        assert!(
            err.to_string().contains("already has a consumer"),
            "unexpected error: {err}",
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
        use super::super::stream::execlog::ExecLogStream;

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
            true,
        )
        .expect("spawn the reader");

        let signals = crate::engine::cancellation::Signals::new();
        let already = super::ExecLogIter::new(signals.clone(), None);
        already
            .bind(stream.receiver().expect("a subscriber to clone"))
            .expect("the first bind should succeed");
        let fresh = super::ExecLogIter::new(signals, None);

        let err = super::bind_execlog_iters(&stream, &[fresh.clone(), already])
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
    }

    /// Iter handle subscribed pre-spawn receives every event from a clean
    /// build, even on the warm-daemon path that drops late subscribers.
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

    /// A mistyped flag reaches Bazel, which rejects the command line and
    /// exits without ever opening the BEP file. The build must report that
    /// exit code instead of waiting forever on a writer that cannot come.
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
        execution_log = True,
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
        // keep it generous so full-suite pool contention can't trip it
        // (a healthy run finishes in well under a second).
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
            None => panic!("build hung past 5s on REMOTE_CACHE_EVICTED with no retry (bug 1060)"),
            Some(r) => {
                let exit = r.expect("run_task");
                assert_eq!(exit, Some(0));
            }
        }
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
)"#
        )
        .expect("snippet should validate");
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

    #[test]
    fn version_line_text() {
        use super::version_line;
        assert_eq!(
            version_line(Some(&semver::Version::new(9, 0, 1))),
            "Bazel 9.0.1"
        );
        assert_eq!(
            version_line(None),
            "Bazel development version (version-conditional flags assume latest)"
        );
    }

    #[test]
    fn render_command_joins_program_and_args() {
        use super::render_command;
        let mut cmd = std::process::Command::new("bazel");
        cmd.args(["--bazelrc=/dev/null", "build", "--", "//foo:bar"]);
        assert_eq!(
            render_command(&cmd),
            "bazel --bazelrc=/dev/null build -- //foo:bar"
        );
    }

    #[test]
    fn render_command_redacts_env_secrets() {
        // Delegates to stream::redaction; this asserts the wiring (secret env
        // values are hidden, the command shape is preserved). The redaction
        // rules themselves are covered in stream::redaction's own tests.
        use super::render_command;
        let mut cmd = std::process::Command::new("bazel");
        cmd.args(["build", "--action_env=DB_PASSWORD=hunter2", "//foo"]);
        let rendered = render_command(&cmd);
        assert!(rendered.starts_with("bazel build --action_env=DB_PASSWORD="));
        assert!(rendered.ends_with(" //foo"));
        assert!(!rendered.contains("hunter2"), "secret leaked: {rendered}");
    }
}
