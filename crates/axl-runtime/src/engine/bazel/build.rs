use crate::errln;
use std::cell::Cell;
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
use super::stream::execlog::ReaderOutcome;
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

    /// Subscribe the iterator's receiver. Must run before the reader reaches
    /// bazel's first events so the early burst is buffered. The `kinds=` filter is
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

/// Every BES payload kind a `kinds=` list can name, as
/// `(tag, type_name, legacy_aliases)`.
///
/// `tag` is this runtime's payload discriminant: the integer
/// [`payload_discriminant`] returns and the set `kinds=` is matched against.
/// It is deliberately opaque and is *not* the BEP proto field number (the two
/// numberings diverge for most kinds); only the two sides agreeing matters,
/// which `payload_tags_agree_with_the_kind_table` pins.
///
/// `type_name` is the snake_case name of the payload *message*, which is
/// exactly the string `type(event.payload)` returns in AXL. Every kind is
/// reachable under that name, so a `kinds=` filter can be written with the
/// same literal the `type(event.payload) == "..."` test uses. Closing that
/// gap is why this table exists: `"action_completed"` was accepted while
/// `type()` reported `action_executed`, so the filter read correctly and the
/// comparison never matched.
///
/// `legacy_aliases` are the other spellings `kinds=` has always accepted —
/// the BEP `payload` oneof field name (`action`, `completed`, …), the
/// `event.kind` string where it differs (`action_completed`,
/// `target_completed`, …), and `named_set`, which is neither: it is the
/// `BuildEventId` oneof *field* name, accepted from before the others and kept
/// for that reason alone. They are a public AXL surface that third-party
/// `.aspect/*.axl` passes, so they stay accepted — frozen by
/// `the_spellings_accepted_before_the_payload_names_still_resolve`, which holds
/// its own literal list because a test derived from this table cannot notice a
/// row losing an alias.
const EVENT_KINDS: &[(i32, &str, &[&str])] = &[
    (3, "progress", &[]),
    (4, "aborted", &[]),
    (5, "build_started", &["started"]),
    (6, "pattern_expanded", &["expanded"]),
    (7, "target_configured", &["configured"]),
    (8, "action_executed", &["action", "action_completed"]),
    (9, "target_complete", &["completed", "target_completed"]),
    (10, "test_result", &[]),
    (11, "build_finished", &["finished"]),
    (12, "unstructured_command_line", &[]),
    (13, "command_line", &["structured_command_line"]),
    (14, "options_parsed", &[]),
    (15, "named_set_of_files", &["named_set"]),
    (16, "workspace_status", &[]),
    (17, "fetch", &[]),
    (19, "configuration", &[]),
    (20, "test_summary", &[]),
    (21, "build_tool_logs", &[]),
    (22, "build_metrics", &[]),
    (24, "build_metadata", &[]),
    (25, "workspace_config", &["workspace_info"]),
    (26, "target_summary", &[]),
    (27, "convenience_symlinks_identified", &[]),
    (28, "exec_request_constructed", &["exec_request"]),
    (30, "test_progress", &[]),
];

/// The tail of a `kinds=` rejection: every accepted spelling, with the
/// `type(event.payload)` names first because those are the ones that also
/// work as a `type()` comparison literal.
fn event_kind_help() -> String {
    let mut types: Vec<&str> = EVENT_KINDS.iter().map(|(_, name, _)| *name).collect();
    types.sort_unstable();
    let mut legacy: Vec<&str> = EVENT_KINDS
        .iter()
        .flat_map(|(_, _, aliases)| aliases.iter().copied())
        .collect();
    legacy.sort_unstable();
    format!(
        "name a payload the way `type(event.payload)` reports it — {} — or use \
         one of the older aliases, also accepted: {}",
        types.join(", "),
        legacy.join(", "),
    )
}

/// Resolve one `kinds=` list element to a payload tag. Accepts the payload's
/// `type(event.payload)` name, any legacy alias, or the raw tag integer.
///
/// A tag no row claims is rejected rather than passed through, because it could
/// only ever filter to the empty set, and silently matching nothing is the
/// failure mode worth refusing.
///
/// This does **not** rescue someone who read an older docstring's claim that
/// these are BEP proto field numbers. They are this runtime's own numbering, and
/// 11 of the 25 real field numbers are themselves claimed tags meaning a
/// different kind — `7` (BEP `ActionExecuted`) selects `target_configured`,
/// `8` (`TargetComplete`) selects `action_executed`. Those stay silent, and no
/// validation can catch them. The integers are documented as opaque and
/// `event_kind_help()` lists only names, so there is no supported way to obtain
/// a correct one; the path survives for compatibility alone.
pub(super) fn parse_event_kind(value: values::Value) -> anyhow::Result<i32> {
    if let Some(n) = value.unpack_i32() {
        if !EVENT_KINDS.iter().any(|(tag, _, _)| *tag == n) {
            anyhow::bail!("unknown build_event payload tag {n}; {}", event_kind_help());
        }
        return Ok(n);
    }
    if let Some(s) = value.unpack_str() {
        for (tag, type_name, aliases) in EVENT_KINDS {
            if s == *type_name || aliases.contains(&s) {
                return Ok(*tag);
            }
        }
        anyhow::bail!("unknown build_event kind '{s}'; {}", event_kind_help());
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

/// Maps a payload variant to this runtime's payload tag — the integers
/// [`EVENT_KINDS`] lists, so `kinds=` matches by integer set lookup.
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
    /// its `kinds=` filter, a `Subscription` (an `Arc` to the shared state), and
    /// an `Arc<Signals>`.
    #[allocative(skip)]
    execlog_iters: Vec<ExecLogIter>,

    /// The `build.execution_logs()` iterator, once that call has handed out the
    /// stream's own subscriber. Held for exactly the reason `execlog_iters` is,
    /// and it is the only way to reach that subscriber: `execution_logs()` takes
    /// the receiver out of the stream, so `ExecLogStream::join()`'s own release
    /// valve no longer owns it. Without a clone here, an iteration the task
    /// abandoned — or claimed and never ran — parks the reader past the end of
    /// the build.
    #[allocative(skip)]
    execution_logs_iter: RefCell<Option<ExecutionLogIterator>>,

    /// Set by `wait()`, which takes `execlog_stream`. Without it a later
    /// `execution_logs()` cannot tell "this build has no execution log" from
    /// "its stream has already been joined", and would tell a caller who did
    /// pass `execution_log` to pass it.
    #[allocative(skip)]
    execlog_joined: Cell<bool>,

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
        // Asked with the same startup flags the build below will use, because
        // that is what decides which server answers. Bazel kills a running
        // server whose startup options differ from the ones it is handed, so a
        // pid read without them is the pid of a server the build then replaces —
        // and `pid` is the execution log's nominated holder. Dead, the reader
        // takes a log that does not exist yet as a log that never will and ends
        // the stream clean and empty: every `exec_log_event` hook fires zero
        // times, every sink writes nothing, and the build passes.
        //
        // The workspace reader takes this pid too, but it reads a FIFO:
        // `Pipe::open` blocks for a *writer* rather than for a pid, and the pid
        // is consulted only on an empty read, which on a blocking FIFO cannot
        // happen until every writer has closed — by which point "not open" is
        // the right answer whether the pid is alive or dead.
        //
        // The BES reader also reads a regular file, but it does *not* share the
        // exposure: `StreamingFile::open_restartable` deliberately does not
        // gate on holder liveness, and its read side treats a dead holder as
        // "nothing right now" rather than as the end. A stale pid there costs
        // patience, not events. The execution log is the caller still on
        // `StreamingFile::open`, where a stale pid is the end of the stream, so
        // this is the one place the pid has to be the right one.
        let (pid, version) = super::info::server_info_with_startup_flags(&signals, &startup_flags)?;

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
        // Aspect BES backend, a Workflows runner, or a `.bazelrc` already asks Bazel
        // for the compact log, and whoever asked reads the file at that path. Adding a
        // second `--execution_log_compact_file` below would win — a literal flag
        // outranks rc content — and silently repoint Bazel away from it.
        //
        // An empty value clears the flag rather than naming a file, so it is not a
        // request; treating it as a path would have the reader tail "" and report a
        // clean empty stream. A relative value is joined to the spawn's directory,
        // because that is what Bazel resolves it against while the reader runs from
        // the CLI's own cwd.
        let injected_execlog_path =
            bazelrc::last_flag_value(&flags, EXECUTION_LOG_COMPACT_FILE, verb)
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

        // Mint the BEP path now (before `cmd.spawn()`) so bazel has somewhere
        // to write. Nothing is created on disk: bazel writes a regular file.
        // The reader-side thread is started later — once we have the spawned
        // child's pid in hand for the per-invocation liveness check.
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

        // Split execlog sinks into the two paths that serve them: compact paths go
        // to the tee reader inside the stream thread, decoded paths to a writer
        // thread each, cloning a subscriber of the decoded stream. Both halves are
        // paths rather than sinks from here on, so neither loop below can be
        // reached by the other's variant.
        let mut compact_paths: Vec<String> = vec![];
        let mut decoded_paths: Vec<String> = vec![];
        for sink in execlog_sinks {
            match sink {
                ExecLogSink::CompactFile { path } => compact_paths.push(path),
                ExecLogSink::File { path } => decoded_paths.push(path),
            }
        }

        // Reserved before the spawn so bazel can be told where to write, but the
        // reader is started after it — once the client pid exists (see the BES
        // reader below for the same split, and why).
        let execlog_path = if execution_logs {
            let out = match injected_execlog_path {
                // Something already named the path and put the flag on the command
                // line. Tail that file and leave the flag alone.
                Some(out) => out,
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
                    out
                }
            };

            // Every remaining CompactFile sink is served by the tee rather than by
            // Bazel writing directly, so each still gets its copy — except one
            // naming the very file Bazel is writing, whose `File::create` would
            // truncate it mid-build. Both arms can hold one: a sink's path can
            // already be on the command line, and two sinks can name one path, in
            // which case the first lent it to Bazel above.
            compact_paths.retain(|path| std::path::Path::new(path) != out);
            Some(out)
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
        // The exposure is in reused paths: a `CompactFile` sink's path and one
        // already on the command line are both fixed per build, so on a Workflows
        // runner or an Aspect-wired deployment every build finds the last one's
        // log waiting. Removing it is what makes the existence poll mean "wait for
        // *this* build's log"; Bazel creates the file itself, so there is nothing
        // to put back.
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
        //
        // The producer blocks rather than dropping. Blocking cannot reach Bazel —
        // Bazel writes the log to a regular file and this process tails it — so a
        // consumer slower than the decoder only lengthens the `join()` inside
        // `wait()`. And that `wait()` releases every subscriber before joining, so
        // one nobody drains cannot park the reader past the end of the build.
        let named_consumers = !decoded_paths.is_empty() || !execlog_iters.is_empty();
        let mut execlog_stream = match execlog_path {
            Some(p) => Some(ExecLogStream::spawn_with_file(
                p,
                pid,
                child.id(),
                compact_paths,
            )?),
            None => None,
        };

        // Bind subscribers BEFORE the BES reader delivers anything. The reader
        // is currently polling for an event file that bazel does not create
        // until its JVM is up, so subscriptions registered here win the
        // warm-daemon race against the early event burst.
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
            for path in decoded_paths {
                let recv = stream.receiver().ok_or_else(|| {
                    io::Error::other("execution log stream has no subscriber to clone")
                })?;
                stream.attach_file_sink(ExecLogSink::spawn_file(recv, path));
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

        // Every named consumer has cloned what it needs, so the stream must not go
        // on holding an unread subscriber of its own — see `ExecLogStream::recv`.
        //
        // A stream no consumer was named for keeps its subscriber, because
        // `build.execution_logs()` may still claim it. Until something drains it
        // the producer sits at the channel capacity; `join()` releasing it is what
        // bounds that.
        if named_consumers && let Some(stream) = execlog_stream.as_mut() {
            stream.take_initial_subscriber();
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
            execution_logs_iter: RefCell::new(None),
            execlog_joined: Cell::new(false),
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
    /// An iterable over every entry of this build's decoded execution log.
    ///
    /// Takes the stream's one subscriber rather than cloning it, so it can be
    /// called once per build and only when nothing else decodes the log: a
    /// decoded `execution_log.file(...)` sink or an `execution_log.iterator()`
    /// handle is refused, with a message naming the replacement. A
    /// `compact_file(...)` sink is not one of those and combines freely.
    ///
    /// Iterate it before `build.wait()`. The producer blocks for this
    /// subscriber, which is what makes the iteration complete, and `wait()`
    /// releases it — so a `wait()` first ends the stream, and a call afterwards
    /// is refused.
    fn execution_logs<'v>(this: values::Value<'v>) -> anyhow::Result<ExecutionLogIterator> {
        let build = this.downcast_ref::<Build>().unwrap();
        let mut execlog_stream = build.execlog_stream.borrow_mut();
        let execlog_stream = execlog_stream.as_mut().ok_or_else(|| {
            if build.execlog_joined.get() {
                anyhow::anyhow!(
                    "this build's execution log was already joined by `build.wait()`, \
                     which ends the stream. Call `build.execution_logs()` and iterate \
                     it before `wait()`, or pass an `execution_log.iterator()` handle \
                     in `execution_log=[...]`, which `wait()` also releases but which \
                     can be created before the build and drained as it runs."
                )
            } else {
                anyhow::anyhow!(
                    "call `ctx.bazel.build` with `execution_log = true` in order to \
                     receive execution log events."
                )
            }
        })?;

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
                 `execution_log.iterator()` handle, each of which clones a \
                 subscriber of its own, after which the stream's own is dropped. \
                 Either way there is nothing left to hand out. Pass an \
                 `execution_log.iterator()` handle in \
                 `execution_log=[...]` and iterate that instead: a handle clones \
                 too, so it can be combined with sinks, and its `kinds=` filter \
                 runs in Rust before an entry becomes a Starlark value."
            ))?;
        let iter = ExecutionLogIterator::new(build.signals.clone(), recv);
        build.execution_logs_iter.replace(Some(iter.clone()));
        Ok(iter)
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
    /// Every decoded execution-log consumer belongs **before** this call, not
    /// merely preferably. `wait()` releases each one — the `execution_logs()`
    /// iterator and every still-bound `execution_log.iterator()` handle — and
    /// then takes the stream, so a consumer stops yielding from here on, anything
    /// it had not been drained of is gone, and a later `execution_logs()` finds
    /// no stream and says so.
    ///
    /// That is deliberate: each of those is a subscriber the producer blocks for,
    /// which is what makes the iteration complete, and releasing them is the only
    /// thing that lets a build nobody drained end. The drain therefore belongs
    /// before the call, not after.
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

        // Release every decoded consumer still holding a subscriber. The producer
        // blocks for each, so one the task never drained would park the reader
        // thread on a full channel and hang the join below. The AXL side drains
        // before calling `wait()` (see `bazel/exec_log.axl`); this is the net
        // under a task that does not, trading dropped entries for a build that
        // ends.
        //
        // `ExecLogStream::join()` releases the stream's own subscriber, but only
        // while the stream still owns it — `execution_logs()` takes it, so that
        // iterator has to be released here.
        for iter in &build.execlog_iters {
            iter.release();
        }
        if let Some(iter) = build.execution_logs_iter.take() {
            iter.release();
        }

        // Wait for Execlog stream to complete.
        //
        // This is the one place that has both the reader's verdict and bazel's exit
        // code, which is what makes the warning safe to print: a log that never
        // appeared is ordinary on a failed invocation (a mistyped target pattern
        // never reaches the phase that creates one, and bazel has already said so)
        // and an anomaly on a successful one. See `ReaderOutcome`.
        let execlog_stream = build.execlog_stream.take();
        build.execlog_joined.set(true);
        if let Some(execlog_stream) = execlog_stream {
            match execlog_stream.join() {
                Ok(ReaderOutcome::NeverAppeared) if result.success() => {
                    errln!(
                        "WARNING: bazel completed successfully but wrote no execution \
                         log, so `exec_log_event` hooks and execution-log sinks saw \
                         nothing for this invocation."
                    );
                }
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

    /// How long the daemon stand-in lives.
    ///
    /// It has to outlast the longest timeout any test passes to
    /// [`run_execlog_script`], because a timeout leaks the stand-in
    /// ([`DaemonStandIn::leak`]) and the build that outran the timeout keeps
    /// asking this pid whether more bytes are coming. A stand-in that died first
    /// would answer "the writer is gone" to a build still writing.
    #[cfg(unix)]
    const STAND_IN_LIFETIME: std::time::Duration = std::time::Duration::from_secs(300);

    /// A live process standing in for the Bazel daemon, plus the lock that
    /// serializes the tests using one. Torn down on drop, so a panicking body
    /// cannot leave `BASIL_SERVER_PID` naming a process it killed.
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
            // execlog-through-basil tests by `EXECLOG_BASIL`. A concurrent
            // non-execlog bazel test reading this as its server pid gets the
            // opposite answer from the reaped pid it reads today — `open` waits
            // for the file instead of giving up on it — but no such test
            // configures an execution log, so nothing consults the pid.
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
    /// can belongs in [`run_execlog_script`].
    #[cfg(unix)]
    fn with_daemon_stand_in<T>(body: impl FnOnce() -> T) -> T {
        let _stand_in = DaemonStandIn::new();
        body()
    }

    /// Run `script` as a task under a daemon stand-in, bounded by `timeout`, and
    /// return what `run_task` reported.
    ///
    /// Every execution-log test below fails by parking rather than by answering
    /// wrongly, so each needs a timeout, and a timeout fires while the work is
    /// still running — hence [`DaemonStandIn::leak`] rather than an ordinary
    /// teardown. `hang` explains the timeout.
    #[cfg(unix)]
    fn run_execlog_script(
        timeout: std::time::Duration,
        script: String,
        hang: &str,
    ) -> anyhow::Result<Option<u8>> {
        assert!(
            timeout < STAND_IN_LIFETIME,
            "the daemon stand-in ({STAND_IN_LIFETIME:?}) must outlast a leaked \
             test's timeout ({timeout:?}); see STAND_IN_LIFETIME",
        );
        let stand_in = DaemonStandIn::new();
        let result = crate::test::with_timeout(timeout, move || {
            crate::test::eval(&script).with_fake_bazel().run_task(0)
        });
        match result {
            Some(exit) => exit,
            None => {
                stand_in.leak();
                panic!("timed out after {timeout:?}: {hang}");
            }
        }
    }

    /// [`run_execlog_script`], asserting the task exited 0. `wrong` explains a
    /// task that ran and returned nonzero, which is the scripts' own `return 2` /
    /// `return 3` arms.
    #[cfg(unix)]
    fn run_execlog_task(timeout: std::time::Duration, script: String, hang: &str, wrong: &str) {
        let exit = run_execlog_script(timeout, script, hang).expect("run_task");
        assert_eq!(exit, Some(0), "{wrong}");
    }

    /// How many entries `--scenario=execlog_beyond_capacity` writes. Longer than
    /// `CHANNEL_CAPACITY`, which is the only length that can tell a stream that
    /// delivers everything apart from one that stops at the ring.
    #[cfg(unix)]
    const SCENARIO_ENTRIES: u32 = 3000;

    /// `execution_log = True` must deliver the whole log to `execution_logs()`.
    ///
    /// The stream hands this iterator its one subscriber rather than cloning it,
    /// because an unread clone left behind caps the ring (see
    /// `ExecLogStream::recv`). That subscriber is registered when the stream
    /// spawns, before Bazel writes anything, so the producer blocks for it exactly
    /// as it does for a sink or a handle: every entry arrives, contiguously, and
    /// how fast the caller reads decides only how long the build's `wait()` takes.
    ///
    /// The assertion is contiguity and the exact count, not just "more than the
    /// capacity". A short prefix is the shape the failure takes, and in a real log,
    /// whose leading records are inputs rather than spawns, a prefix can contain no
    /// spawns at all — a confidently wrong answer rather than an obviously short
    /// one. The timeout is because a blocking producer fails by parking.
    #[cfg(unix)]
    #[test]
    fn execution_logs_delivers_the_whole_log() {
        use std::time::Duration;

        let capacity = super::super::stream::execlog::channel_capacity() as u32;
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
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
        stderr = None,
    )
    seen = 0
    for entry in build.execution_logs():
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

        run_execlog_task(
            Duration::from_secs(60),
            script,
            "the execution log's producer parked",
            "every entry must arrive, with contiguous ids",
        );

        let text = std::fs::read_to_string(&report).expect("the task should have reported");
        let seen: u32 = text.trim().parse().unwrap();
        assert_eq!(
            seen, SCENARIO_ENTRIES,
            "`execution_log = True` must be lossless; a count short of the log is the \
             prefix a dropping producer leaves, and one short by a varying amount is \
             the race it leaves",
        );
    }

    /// `execution_log = True` on a task that never calls `execution_logs()`.
    ///
    /// This is the one behavioural cost of a blocking producer: with nobody
    /// draining, it parks at `CHANNEL_CAPACITY`. What unparks it is `wait()` ->
    /// `ExecLogStream::join()`, whose first act is to release the stream's own
    /// subscriber; the blocked `send` then fails, the reader records that no
    /// subscriber is left, and drains the remaining bytes without decoding them.
    /// So the build still ends — and because Bazel writes the log to a regular
    /// file that this process tails, the park never reached Bazel either.
    ///
    /// This covers the half where the stream still *owns* its subscriber, and so
    /// `join()` releases it. The tests below cover the subscribers `join()` cannot
    /// reach — the one `execution_logs()` took, and the clone an `iterator()`
    /// handle holds — each of which `wait()` has to release itself, so no one of
    /// these tests sees another's regression.
    ///
    /// The timeout is the whole assertion; a regression here is a hang.
    #[cfg(unix)]
    #[test]
    fn a_log_nobody_iterates_does_not_hang_the_build() {
        use std::time::Duration;

        run_execlog_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
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
    /// `execution_logs()` takes the stream's own subscriber, so from then on the
    /// receiver lives in the iterator and `ExecLogStream::join()`'s
    /// `self.recv.take()` releases nothing. A `break` leaves that subscriber in
    /// place, undrained, with the reader parked against it on a full ring — and
    /// `wait()` then joins a thread that can never return. `wait()` releasing the
    /// iterator is what makes the loop below one a task is allowed to write.
    ///
    /// Ten entries of the scenario's log, so the break lands while the producer is
    /// still writing rather than after it has finished.
    #[cfg(unix)]
    #[test]
    fn an_abandoned_execution_log_iteration_does_not_hang_the_build() {
        use std::time::Duration;

        run_execlog_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
        stderr = None,
    )
    seen = 0
    for entry in build.execution_logs():
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
            "an abandoned `execution_logs()` iteration parked the reader past the \
             end of the build",
            "a task that stops iterating early must still reach the end of its build",
        );
    }

    /// The same, for an iterator claimed and never iterated at all.
    ///
    /// Distinct from the test above rather than a weaker version of it: here the
    /// subscriber sits at entry zero, so the ring reads as full from the
    /// producer's first thousand entries onwards and the reader parks before the
    /// task does anything else. It is also the shape a task reaches by accident —
    /// binding `build.execution_logs()` to a name and then taking an early return.
    #[cfg(unix)]
    #[test]
    fn an_unused_execution_logs_iterator_does_not_hang_the_build() {
        use std::time::Duration;

        run_execlog_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
        stderr = None,
    )
    entries = build.execution_logs()
    status = build.wait()
    if not status.success: return 1
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "an `execution_logs()` iterator nobody iterated parked the reader past \
             the end of the build",
            "claiming the iterator and not using it must still end the build",
        );
    }

    /// The same, for an `execution_log.iterator()` handle the task never drained.
    ///
    /// `wait()` releases every still-bound handle for the same reason it releases
    /// the `execution_logs()` iterator: the producer blocks for each subscriber,
    /// so one nobody drained parks the reader on a full ring and the join never
    /// returns. Distinct from the two tests above, which cover the receiver
    /// `execution_logs()` took — a handle's subscriber is a clone the stream never
    /// owned, so neither release valve covers the other.
    #[cfg(unix)]
    #[test]
    fn an_undrained_iterator_handle_does_not_hang_the_build() {
        use std::time::Duration;

        run_execlog_task(
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

        run_execlog_task(
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

    /// The same `drain()` on the `build.execution_logs()` iterator, which carries
    /// it for the same reason a handle does.
    ///
    /// `done()` there is `is_drained` rather than "unsubscribed", so it reads true
    /// after the release either way; what this pins is that the release happens —
    /// the iteration ends and the build's `wait()` returns rather than joining a
    /// reader still parked on this subscriber.
    #[cfg(unix)]
    #[test]
    fn a_drained_execution_logs_iterator_stops_consuming_mid_build() {
        use std::time::Duration;

        run_execlog_task(
            Duration::from_secs(60),
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
        stderr = None,
    )
    entries = build.execution_logs()
    seen = 0
    for _ in entries:
        seen += 1
        if seen == 5:
            break
    entries.drain()
    entries.drain()            # idempotent
    if not entries.done(): return 3
    for _ in entries:
        return 4               # a drained iterator yields nothing
    status = build.wait()
    if not status.success: return 1
    if seen != 5: return 5
    return 0

Test = task(implementation = _impl)
"#
            .to_string(),
            "an `execution_logs()` iterator drained mid-build left the reader parked past \
             the end of the build",
            "a drained `execution_logs()` iterator must yield nothing more and let the \
             build end",
        );
    }

    /// `execution_logs()` after `wait()` is refused, and says why.
    ///
    /// `wait()` takes the stream, so there is nothing left to hand out. Iterating
    /// before `wait()` is a requirement, not a preference, so the refusal has to
    /// name that ordering rather than fall through to "pass `execution_log = true`",
    /// which is advice the caller has already taken.
    #[cfg(unix)]
    #[test]
    fn execution_logs_after_wait_names_the_ordering() {
        let script = r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
        stderr = None,
    )
    build.wait()
    build.execution_logs()
    return 0

Test = task(implementation = _impl)
"#;
        // The `wait()` here is the one in
        // `a_log_nobody_iterates_does_not_hang_the_build`, so that regression
        // reaches this test as a hang too — hence the timeout.
        let err = run_execlog_script(
            std::time::Duration::from_secs(60),
            script.to_string(),
            "`wait()` on an un-iterated log parked the reader",
        )
        .expect_err("a call after `wait()` must be refused")
        .to_string();
        assert!(
            err.contains("already joined by `build.wait()`") && err.contains("before `wait()`"),
            "the refusal should name the ordering, not ask for `execution_log = true` \
             again: {err}",
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
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = True,
        stderr = None,
    )
    seen = 0
    busy = 0
    for entry in build.execution_logs():
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

        run_execlog_task(
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
        run_execlog_task(
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
    /// line themselves, and the backend reads the file at *that* path. Adding a
    /// second one wins on last-write-wins and silently repoints Bazel, so the
    /// consumer tails a path nothing writes and the backend's file never appears.
    #[cfg(unix)]
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
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );

        run_execlog_task(
            std::time::Duration::from_secs(60),
            script,
            "the reader never reached the log at the path already on the command line",
            "the reader must tail the path already on the command line",
        );
        assert!(
            asked.exists(),
            "bazel must still write the log where the deployment asked, at {}",
            asked.display(),
        );
    }

    /// No `CompactFile` sink may name the file Bazel is writing.
    ///
    /// One sink's path is lent to Bazel so it writes the caller's destination
    /// directly; every other is served by the reader thread's `MultiTeeReader`,
    /// which `File::create`s its path. A sink naming the path Bazel was handed
    /// therefore truncates the log out from under it — and out from under the
    /// reader, which has already opened the file and now sees it stop producing
    /// bytes while the nominated holder is still alive.
    ///
    /// Two sinks with one path is the shape that reaches it here; a config
    /// appending a path the artifacts feature already appended is how it arrives
    /// in practice. The timeout is because the symptom is a stalled reader rather
    /// than a wrong answer.
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

        run_execlog_task(
            Duration::from_secs(60),
            script,
            "the tee truncated the log the reader was tailing",
            &format!("two sinks naming one path must still deliver all {SCENARIO_ENTRIES} entries"),
        );

        // And the file itself is whole: Bazel's own copy, not one the tee
        // re-created empty beside it.
        assert_eq!(
            compact_entry_count(&sink_path),
            SCENARIO_ENTRIES,
            "the compact log on disk should be complete",
        );
    }

    /// Two *distinct* compact sinks and no decoded consumer at all.
    ///
    /// The configuration nothing else here covers: with no decoded sink and no
    /// handle, `named_consumers` is false, so the stream keeps its own subscriber
    /// against a `build.execution_logs()` that never comes. Nothing drains it, so
    /// the producer sits at the channel capacity for the rest of the build and
    /// `join()` is what releases it.
    ///
    /// Compact sinks are tee'd from the raw bytes before decoding, so neither file
    /// depends on that subscriber — which is the assertion: both are complete, and
    /// the parked decoder cost the build nothing but a short `join()`. One path is
    /// the one lent to Bazel and the other is written by the reader thread's
    /// `MultiTeeReader`, so this covers both halves of the tee.
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

        run_execlog_task(
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

    /// A `compact_file` sink beside `build.execution_logs()` — the combination
    /// `ctx.bazel.build`'s own documentation advertises.
    ///
    /// A compact sink is not a decoded consumer: it is tee'd from the raw bytes
    /// before decoding, so it clones no subscriber and leaves the stream's own one
    /// for `execution_logs()` to take. Both therefore get everything, and the
    /// refusal that fires beside a decoded `file()` sink must not fire here.
    #[cfg(unix)]
    #[test]
    fn a_compact_sink_and_execution_logs_both_see_the_whole_log() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let sink_path = dir.path().join("compact.zstd");
        let script = format!(
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [bazel.execution_log.compact_file(path = {sink:?})],
        stderr = None,
    )
    seen = 0
    for entry in build.execution_logs():
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

        run_execlog_task(
            Duration::from_secs(60),
            script,
            "a compact sink plus `execution_logs()` deadlocked",
            &format!(
                "`execution_logs()` beside a compact sink must see all \
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
    /// file, so it is not a path to reuse.
    ///
    /// An rc file setting the flag for a config and a command line clearing it
    /// is an ordinary way to reach this. Read back as `Some("")` the reader
    /// tails `""`, which cannot exist, and reports a clean end of stream — every
    /// hook fires zero times and the build passes.
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
    # Zero means the empty value was taken for a path and tailed.
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#
        );
        run_execlog_task(
            std::time::Duration::from_secs(60),
            script,
            "the reader tailed \"\" and never ended",
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
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            dir = dir.path().to_str().unwrap(),
        );

        run_execlog_task(
            std::time::Duration::from_secs(60),
            script,
            "the reader tailed a path bazel never wrote",
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
    if highest > {SCENARIO_ENTRIES}: return 2
    if seen != {SCENARIO_ENTRIES}: return 3
    return 0

Test = task(implementation = _impl)
"#,
            asked = asked.to_str().unwrap(),
        );

        // The timeout catches a reader parked on the stale inode rather than
        // bounding a healthy run, which takes a little over two seconds.
        run_execlog_task(
            std::time::Duration::from_secs(120),
            script,
            "the reader never reached this build's log",
            "the consumer must receive this build's log, not the one already at the path",
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

    /// The same refusal beside a decoded `file()` sink, the other consumer that
    /// clones the stream's subscriber. A compact sink is not one — it is tee'd
    /// from the raw bytes before decoding — so `execution_logs()` still works
    /// alongside those.
    #[cfg(unix)]
    #[test]
    fn execution_logs_is_refused_beside_a_decoded_file_sink() {
        let dir = tempfile::tempdir().unwrap();
        let sink_path = dir.path().join("decoded.binpb");
        let script = format!(
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=execlog_beyond_capacity"],
        execution_log = [bazel.execution_log.file(path = {sink:?})],
        stderr = None,
    )
    build.execution_logs()
    return 0

Test = task(implementation = _impl)
"#,
            sink = sink_path.to_str().unwrap(),
        );
        let err = with_daemon_stand_in(move || {
            crate::test::eval(&script)
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

    /// Every accepted `kinds=` spelling, the tag it resolves to, and the
    /// string `type(event.payload)` reports for that payload — pinned
    /// together, because the whole point of [`EVENT_KINDS`] is that the three
    /// agree. A row drifting apart is the bug this table replaced: `kinds =
    /// ["action_completed"]` was accepted while `type(event.payload)`
    /// returned `action_executed`, so the filter read right and never matched.
    mod event_kinds {
        use super::super::*;
        use axl_proto::build_event_stream::build_event::Payload;
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

    /// A multi-attempt build through the whole stack: `ctx.bazel.build` →
    /// `Build::spawn`'s pid plumbing → `BuildEventStream` → the AXL iterator.
    /// The `cache_evicted_with_retry` scenario had been defined in basil with
    /// no consumer at all, so nothing covered this shape end to end.
    ///
    /// **What this does not cover.** Only attempt 1's two events reach the
    /// iterator, and that is a limit of the fake bazel rather than the reader.
    /// `await_restart` detects the next attempt by watching the *holder* reopen
    /// the file, and basil can never be the holder: the pid the reader is given
    /// comes from `bazel info server_pid`, which basil answers with either
    /// `BASIL_SERVER_PID` (a `sleep` stand-in that never touches the event
    /// file) or its own already-exited `info` pid. So the primary signal cannot
    /// fire here, the weaker shorter-file fallback loses the race against two
    /// attempts of near-identical length, and the stream ends when basil does.
    ///
    /// The retry mechanism itself is covered faithfully one layer down, by
    /// `stream::build_event::tests::test_transient_retry_delivers_events_from_both_attempts`,
    /// which uses a real child process as the holder and does see both
    /// attempts. What *this* test pins is the half that one cannot: that a
    /// build whose event file is truncated under the reader terminates
    /// promptly, with bazel's own exit code, and without hanging — which is
    /// what aspect-build/aspect-cli#1060 was.
    ///
    /// Making this see all four events needs basil to host the file in a
    /// process that outlives one attempt and that `info` can name.
    #[test]
    fn a_cache_eviction_retry_terminates_cleanly_end_to_end() {
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
    if count < 2: return 1
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
                "exit 1 = the iterator saw less than one whole attempt; \
                 exit 2 = bazel's exit code was not surfaced"
            ),
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
