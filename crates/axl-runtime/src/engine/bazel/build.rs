use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::JoinHandle;

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
use starlark::values::starlark_value;

use axl_proto::build_event_stream::BuildEvent;

use crate::engine::r#async::rt::AsyncRuntime;

use super::iter::ExecutionLogIterator;
use super::iter::WorkspaceEventIterator;
use super::sink::execlog::ExecLogSink;
use super::sink::grpc;
use super::sink::retry::{RetryConfig, SinkOutcome};
use super::sink::tracing as tracing_sink;
use super::stream::BuildEventStream;
use super::stream::ExecLogStream;
use super::stream::Subscriber;
use super::stream::WorkspaceEventStream;

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
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(build_status_methods)
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

// =========================================================================
// `bazel.build_events.{grpc, file}(...)` — sink handles.
//
// Each handle is a stateful value the caller creates and hands to
// `ctx.bazel.build(build_events=[...])`. State is shared via `Arc<Mutex>` so
// the original Starlark value and the clone unpacked into `Build::spawn`
// point to the same state machine:
//
//   Pending(config) → Live(per-kind handles) → Done { failed, error }
//
// Once a build is spawned, the caller can `sink.wait()` to block until that
// sink finishes flushing, then inspect `sink.failed` / `sink.error` to
// decide whether to surface a sink-induced task failure.
// `build.wait()` does NOT wait for caller-passed sinks — sink flush is the
// caller's responsibility, not the build's.
// =========================================================================

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

/// Lifecycle state of a sink. Handles are reusable across multiple
/// `ctx.bazel.build(...)` calls (the retry loop in `bazel_runner.axl`
/// relies on this) — each cycle goes Idle → Live → Idle. The Outcome
/// of the most recent cycle is stored separately so `failed` / `error`
/// can be queried after `wait()` without re-locking.
enum SinkPhase {
    /// Ready to bind. Either freshly constructed, or just finished a wait().
    Idle,
    /// Bound to a build; `wait()` will finalize.
    Live(SinkLive),
}

enum SinkLive {
    /// gRPC sink: owns the worker thread's JoinHandle; `wait()` joins it.
    Grpc {
        join: JoinHandle<SinkOutcome>,
    },
    /// File sink: blocks on a one-shot signal driven by the BES reader
    /// thread. All file sinks attached to the same build share the BES
    /// stream's overall result — per-file error attribution would require
    /// splitting MultiWriter, which isn't worth the complexity yet.
    File {
        signal: Arc<FileSignal>,
    },
}

/// Outcome of the most recent `wait()`. Reset on every `bind`; populated
/// when `wait()` returns. `times_bound > 0` means "this handle has been
/// used at least once" — needed to distinguish "never bound" from "freshly
/// bound but already waited" for the `done` attribute.
#[derive(Debug, Default)]
struct SinkOutcomeState {
    failed: bool,
    error: Option<String>,
    times_bound: usize,
}

/// One-shot completion signal for a file sink. The BES reader thread fills
/// `result` and notifies the condvar when writes are flushed; the handle's
/// `wait()` blocks on it. Exposed for the BES stream module to call
/// `complete` on its end.
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

    /// Called by the BES stream once writes are flushed (or have failed).
    /// Idempotent; subsequent calls are no-ops.
    pub fn complete(&self, result: Result<(), String>) {
        let mut guard = self.result.lock().unwrap();
        if guard.is_none() {
            *guard = Some(result);
            self.cv.notify_all();
        }
    }

    /// Block until `complete` has been called, then return the result.
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
    /// Immutable spec. Cloned per bind to spawn a fresh worker.
    #[allocative(skip)]
    config: Arc<SinkConfig>,
    /// Current lifecycle position. Cycles Idle → Live → Idle across re-binds.
    #[allocative(skip)]
    phase: Arc<Mutex<SinkPhase>>,
    /// Outcome of the most recent `wait()`. Reset on every `bind`.
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

    /// True iff this sink wraps a file kind. Used by `Build::spawn` to route
    /// file paths through the BES reader thread.
    fn file_path(&self) -> Option<String> {
        match &*self.config {
            SinkConfig::File { path } => Some(path.clone()),
            _ => None,
        }
    }

    /// Bind a gRPC config to a freshly spawned worker thread. Errors if the
    /// handle is currently `Live` (caller forgot to `wait()` before re-binding)
    /// or if this isn't a gRPC sink.
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
        // Reset outcome at bind time so attrs reflect the current cycle, not
        // a prior cycle's failure.
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

    /// Bind a file config to a `FileSignal` the BES reader thread will
    /// complete after flushing writes. Returns the signal so the caller
    /// can attach it to the BES stream.
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

    // `Ok(None)` (not `Err`) on type mismatch so this can compose with
    // `Either<BuildEventSink, BuildEventIter>` — Either's UnpackValue
    // tries the first branch and falls through on `Ok(None)`.
    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<BuildEventSink>().cloned())
    }
}

#[starlark_value(type = "bazel.build.BuildEventSink")]
impl<'v> values::StarlarkValue<'v> for BuildEventSink {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(build_event_sink_methods)
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let phase = self.phase.lock().unwrap();
        let outcome = self.outcome.lock().unwrap();
        match attribute {
            // True after at least one bind/wait cycle has completed.
            "done" => Some(heap.alloc(
                matches!(*phase, SinkPhase::Idle) && outcome.times_bound > 0,
            )),
            // Last cycle's failure flag. False before first wait, and reset
            // at every bind, so this reflects the most recent attempt only.
            "failed" => Some(heap.alloc(outcome.failed)),
            "error" => Some(match &outcome.error {
                Some(e) => heap.alloc_str(e).to_value(),
                None => Value::new_none(),
            }),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        matches!(attribute, "done" | "failed" | "error")
    }
}

#[starlark_module]
pub(crate) fn build_event_sink_methods(registry: &mut MethodsBuilder) {
    /// Block until this sink finishes flushing. gRPC sinks join their worker
    /// thread; file sinks wait for the BES reader to close their file.
    /// Idempotent — calling `wait()` on an idle sink returns immediately.
    /// After `wait()`, inspect `.failed` and `.error` to decide whether to
    /// surface a sink failure as a task failure. The handle is then ready
    /// to be re-bound (the `bazel_runner.axl` retry loop relies on this).
    fn wait<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
        let sink = this
            .downcast_ref_err::<BuildEventSink>()
            .into_anyhow_result()?;
        // Take the Live payload out of the Mutex so we don't hold the lock
        // across the blocking join; replace with Idle so concurrent waiters
        // see a consistent state.
        let live = {
            let mut phase = sink.phase.lock().unwrap();
            match std::mem::replace(&mut *phase, SinkPhase::Idle) {
                SinkPhase::Live(live) => live,
                SinkPhase::Idle => return Ok(NoneOr::None),
            }
        };
        let outcome: Result<(), String> = match live {
            SinkLive::Grpc { join } => match join.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e.last_error),
                Err(_) => Err("sink worker thread panicked".to_string()),
            },
            SinkLive::File { signal } => signal.wait(),
        };
        let (failed, error) = match outcome {
            Ok(()) => (false, None),
            Err(e) => (true, Some(e)),
        };
        let mut out = sink.outcome.lock().unwrap();
        out.failed = failed;
        out.error = error;
        Ok(NoneOr::None)
    }
}

// =========================================================================
// `bazel.build_events.iterator(...)` — AXL-side iterator handle.
//
// The handle is created by the caller *before* `ctx.bazel.build(...)`, passed
// in via the `build_events=[...]` list, and then iterated. The runtime
// subscribes the underlying receiver inside `Build::spawn` — before bazel
// opens the BEP FIFO — so the early burst (`build_started`,
// `target_completed`, `named_set_of_files`) is buffered for the consumer
// regardless of how slow the AXL task is to start iterating.
//
// State is shared via `Arc<Mutex<IterState>>` so a `BuildEventIter` cloned
// out of the `build_events=[...]` list (for use by `Build::spawn`) and the
// original Starlark value (held by the caller for iteration) point to the
// same state machine. The broadcaster doesn't know or care about this
// handle's buffering policy — it fire-and-forgets into the receiver's
// unbounded mpsc channel and that's it.
// =========================================================================

#[derive(Clone)]
struct IterConfig {
    /// `None` means no filter — every event yields. `Some(set)` keeps only
    /// events whose payload tag is in the set.
    kinds: Option<Arc<HashSet<i32>>>,
}

enum IterState {
    /// Created but not yet bound to a build.
    Pending,
    /// `Build::spawn` subscribed us; iteration reads from `recv`.
    Live { recv: Subscriber<BuildEvent> },
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
    pub fn new(kinds: Option<HashSet<i32>>) -> Self {
        Self {
            config: IterConfig {
                kinds: kinds.map(Arc::new),
            },
            state: Arc::new(Mutex::new(IterState::Pending)),
        }
    }

    /// Called by `Build::spawn` to subscribe the iterator's receiver against
    /// the build event broadcaster. Errors if this handle was already bound
    /// to a build (single-use rule). Must run before bazel opens the BEP
    /// FIFO so the early burst is buffered.
    fn bind(&self, stream: &BuildEventStream) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap();
        match *state {
            IterState::Pending => {
                let recv = stream.subscribe();
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
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(build_event_iter_methods)
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // True once the stream has ended (clean close or `drain()`).
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
        // Loop because `kinds=` may filter out events.
        loop {
            // Take the receiver out of the state so we can call `recv()`
            // (blocking) without holding the Mutex. AXL is single-threaded,
            // so no other thread can observe the intermediate empty Live state.
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

            let result = recv.recv();
            match result {
                Ok(event) => {
                    // Put recv back before deciding whether to filter or yield.
                    *self.state.lock().unwrap() = IterState::Live { recv };
                    if matches_kinds(&event, self.config.kinds.as_ref()) {
                        return Some(event.alloc_value(heap));
                    }
                    // Otherwise loop and read the next event.
                }
                Err(_) => {
                    *self.state.lock().unwrap() = IterState::Done;
                    return None;
                }
            }
        }
    }

    unsafe fn iter_stop(&self) {}
}

#[starlark_module]
pub(crate) fn build_event_iter_methods(registry: &mut MethodsBuilder) {
    /// Stop iterating: unsubscribe, drop buffered events, transition to
    /// `done`. Idempotent; safe to call when already done. Use when the
    /// task already found what it needed and doesn't want to drain the
    /// rest of the stream.
    fn drain<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
        let iter = this
            .downcast_ref_err::<BuildEventIter>()
            .into_anyhow_result()?;
        let mut state = iter.state.lock().unwrap();
        if !matches!(*state, IterState::Done) {
            // Dropping the recv unsubscribes from the broadcaster on its next
            // send (the broadcaster's `retain` prunes senders whose receiver
            // is gone).
            *state = IterState::Done;
        }
        Ok(NoneOr::None)
    }

    /// Non-blocking pop. Returns `None` when the buffer is empty or the
    /// stream has ended. Used by tick-driven drain loops that want to
    /// process whatever's queued without blocking on the next event.
    /// Honors the `kinds=` filter set at construction.
    fn try_pop<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<BuildEvent>> {
        let iter = this
            .downcast_ref_err::<BuildEventIter>()
            .into_anyhow_result()?;
        let kinds = iter.config.kinds.clone();
        loop {
            let mut state = iter.state.lock().unwrap();
            let recv = match &*state {
                IterState::Live { recv } => recv,
                _ => return Ok(NoneOr::None),
            };
            match recv.try_recv() {
                Ok(event) => {
                    drop(state);
                    if matches_kinds(&event, kinds.as_ref()) {
                        return Ok(NoneOr::Other(event));
                    }
                    // Filtered out — loop and try the next event.
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(NoneOr::None),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    *state = IterState::Done;
                    return Ok(NoneOr::None);
                }
            }
        }
    }
}

fn matches_kinds(event: &BuildEvent, kinds: Option<&Arc<HashSet<i32>>>) -> bool {
    let Some(kinds) = kinds else {
        return true;
    };
    let Some(payload) = event.payload.as_ref() else {
        return false;
    };
    kinds.contains(&payload_discriminant(payload))
}

/// Map a `BuildEvent::Payload` variant to its proto field number, which is
/// what the auto-generated `bazel.build.build_event.*` constants resolve to.
/// Keeping this list close to `axl_proto::build_event_stream::build_event::Payload`
/// keeps a `kinds=[build_event.TargetComplete, ...]` filter cheap (no
/// per-event clone or string compare — just an integer set lookup).
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

#[derive(Debug, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<bazel.build.Build>")]
pub struct Build {
    #[allocative(skip)]
    build_event_stream: RefCell<Option<BuildEventStream>>,
    #[allocative(skip)]
    workspace_event_stream: RefCell<Option<WorkspaceEventStream>>,
    #[allocative(skip)]
    execlog_stream: RefCell<Option<ExecLogStream>>,

    /// Internal sink threads (tracing emitter, decoded execlog file
    /// writers). `build.wait()` joins these so the program doesn't exit
    /// before they finish. Caller-passed gRPC/file sinks are NOT here —
    /// their JoinHandles live on the `BuildEventSink` value the caller
    /// holds, and the caller drives them via `sink.wait()`.
    #[allocative(skip)]
    internal_sink_handles: RefCell<Vec<JoinHandle<SinkOutcome>>>,

    /// The shared invocation_id that every gRPC BES sink uses when forwarding
    /// this build's events. `None` when no BES sinks were configured; `Some`
    /// otherwise — in which case all sinks indexed this invocation under this
    /// single UUID (one call to `uuid::Uuid::new_v4()` shared across sinks).
    /// Differs from Bazel's own `build_started.uuid`; that UUID is generated
    /// server-side by the Bazel process, while this one is minted here before
    /// we see `build_started`, so the forwarded stream can start immediately.
    #[allocative(skip)]
    sink_invocation_id: RefCell<Option<String>>,

    #[allocative(skip)]
    child: RefCell<Child>,

    #[allocative(skip)]
    span: RefCell<tracing::Span>,
}

impl Build {
    // TODO: this should return a thiserror::Error
    pub fn spawn(
        verb: &str,
        targets: impl IntoIterator<Item = String>,
        (build_events, sinks, iters): (bool, Vec<BuildEventSink>, Vec<BuildEventIter>),
        (execution_logs, execlog_sinks): (bool, Vec<ExecLogSink>),
        workspace_events: bool,
        flags: Vec<String>,
        startup_flags: Vec<String>,
        stdout: Stdio,
        stderr: Stdio,
        current_dir: Option<String>,
        rt: AsyncRuntime,
    ) -> Result<Build, std::io::Error> {
        let (pid, _) = super::info::server_info()?;

        let span = tracing::info_span!(
            "ctx.bazel.build",
            build_events = build_events,
            workspace_events = workspace_events,
            execution_logs = execution_logs,
            flags = ?flags
        );
        let _enter = span.enter();

        let targets: Vec<String> = targets.into_iter().collect();

        let mut cmd = Command::new(super::bazel_binary());
        cmd.args(startup_flags);
        cmd.arg(verb);
        cmd.args(flags);

        if let Some(current_dir) = current_dir {
            cmd.current_dir(current_dir);
        }

        // Split BES sinks by kind. File sinks accumulate raw pipe bytes in
        // memory and are written after the FIFO closes; gRPC sinks subscribe
        // to the broadcaster in a worker thread. The handle holds Pending
        // config that we transition to Live below — file handles need their
        // FileSignal attached to the BES stream so wait() blocks correctly.
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
        let bes_path = if build_events {
            let p = BuildEventStream::reserve_path()?;
            cmd.arg("--build_event_publish_all_actions")
                .arg("--build_event_binary_file_upload_mode=fully_async")
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

        let execlog_stream = if execution_logs {
            // If there is a CompactFile sink, let Bazel write directly to its path
            // so no separate temp file or tee step is needed for that copy.
            let direct_path = if compact_paths.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(compact_paths.remove(0)))
            };
            let (out, stream) = ExecLogStream::spawn_with_file(
                pid,
                direct_path,
                compact_paths,
                !decoded_sinks.is_empty(),
            )?;
            cmd.arg("--execution_log_compact_file").arg(&out);
            Some(stream)
        } else {
            None
        };

        cmd.arg("--"); // separate flags from target patterns (not strictly necessary for build & test verbs but good form)
        cmd.args(targets);

        crate::trace!("exec: {:?}", cmd.get_args());

        cmd.stdout(stdout);
        cmd.stderr(stderr);
        cmd.stdin(Stdio::null());

        let child = cmd
            .spawn()
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

        // Eagerly bind every iterator handle BEFORE the BES reader thread
        // gets to read anything. The reader thread is currently blocked in
        // `Pipe::open` waiting for bazel to open the FIFO write end (which
        // happens after bazel's JVM startup — many ms later), so subscribing
        // here closes the warm-daemon race where a lazy subscribe could miss
        // the early burst (`build_started`, `target_completed`,
        // `named_set_of_files`).
        if !iters.is_empty() {
            let stream = build_event_stream.as_ref().ok_or_else(|| {
                io::Error::other(
                    "ctx.bazel.build/test: build_events list contained `iterator()` handles \
                     but no BEP stream is configured. Either include them when build_events \
                     is omitted/True, or remove them.",
                )
            })?;
            for iter in &iters {
                iter.bind(stream).map_err(io::Error::other)?;
            }
        }

        // Bind each gRPC sink: spawn its worker thread and transfer the
        // JoinHandle into the handle's Live state so the caller can
        // `sink.wait()` later. The shared invocation_id is minted once
        // and reused across every gRPC sink so all BES backends index
        // this invocation under the same UUID.
        //
        // Subscribing here (after `cmd.spawn()` but before bazel's BEP open
        // unblocks the BES thread) is correct: the BES thread is currently
        // blocked in `Pipe::open` waiting for bazel to open the FIFO write
        // end, which won't happen until bazel finishes JVM startup.
        let debug = std::env::var_os("ASPECT_DEBUG")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let sink_invocation_id: Option<String> = if !grpc_sinks.is_empty() {
            let invocation_id = uuid::Uuid::new_v4().to_string();
            if debug {
                eprintln!(
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
                eprintln!(
                    "BES sinks: 0 gRPC sinks configured (skipping spawn). \
                     ASPECT_WORKFLOWS_BES_BACKEND={bes_backend} \
                     ASPECT_WORKFLOWS_BES_RESULTS_URL={bes_results}"
                );
            }
            None
        };

        // Internal sinks (not exposed as handles to AXL): the tracing
        // emitter and the decoded execlog file writers. `build.wait()`
        // joins these so the program doesn't exit before they finish;
        // user-passed gRPC/file handles, by contrast, are joined via
        // `sink.wait()` on the handle.
        let mut internal_sink_handles: Vec<JoinHandle<SinkOutcome>> = vec![];
        for sink in decoded_sinks {
            if let ExecLogSink::File { path } = sink {
                let handle =
                    ExecLogSink::spawn_file(execlog_stream.as_ref().unwrap().receiver(), path);
                internal_sink_handles.push(handle);
            }
        }
        if build_events {
            internal_sink_handles.push(tracing_sink::Tracing::spawn(
                build_event_stream.as_ref().unwrap().subscribe(),
            ));
        }

        drop(_enter);
        Ok(Self {
            child: RefCell::new(child),
            build_event_stream: RefCell::new(build_event_stream),
            workspace_event_stream: RefCell::new(workspace_event_stream),
            execlog_stream: RefCell::new(execlog_stream),
            internal_sink_handles: RefCell::new(internal_sink_handles),
            sink_invocation_id: RefCell::new(sink_invocation_id),
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
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(build_methods)
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
    // Creates an iterable `ExecutionLogIterator` type.
    // Every call to this function will return a new iterator.
    fn execution_logs<'v>(this: values::Value<'v>) -> anyhow::Result<ExecutionLogIterator> {
        let build = this.downcast_ref::<Build>().unwrap();
        let execlog_stream = build.execlog_stream.borrow();
        let execlog_stream = execlog_stream.as_ref().ok_or(anyhow::anyhow!(
            "call `ctx.bazel.build` with `execution_log = true` in order to receive execution log events."
        ))?;

        Ok(ExecutionLogIterator::new(execlog_stream.receiver()))
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

    fn try_wait<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<BuildStatus>> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        let status = build.child.borrow_mut().try_wait()?;
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
    /// `build_events()` remains usable after `wait()` for replaying historical
    /// events, because the build event stream retains its buffer.
    fn wait<'v>(this: values::Value<'v>) -> anyhow::Result<BuildStatus> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;

        // Re-enter the span so trace coverage includes the full build lifecycle
        let span = build.span.borrow().clone();
        let _enter = span.enter();

        let result = build.child.borrow_mut().wait()?;

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

        // Wait for Execlog stream to complete.
        let execlog_stream = build.execlog_stream.take();
        if let Some(execlog_stream) = execlog_stream {
            match execlog_stream.join() {
                Ok(_) => {}
                Err(err) => anyhow::bail!("execlog stream thread error: {}", err),
            }
        };

        // Drain INTERNAL sink threads (tracing emitter, decoded execlog
        // file writers). Caller-passed sinks aren't here — their
        // JoinHandles live on the BuildEventSink handle, and the caller
        // drives them via `sink.wait()`. Internal sink failures are
        // logged at the sink (see `sink/grpc.rs::finalize`) and are
        // otherwise non-fatal.
        for handle in build.internal_sink_handles.take() {
            match handle.join() {
                Ok(_) => continue,
                Err(err) => anyhow::bail!("internal sink thread panicked: {:#?}", err),
            }
        }

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
    //! End-to-end coverage of `ctx.bazel.build` through the AXL eval stack.
    //!
    //! Uses the `basil` fake-bazel binary (see `crates/basil`) via the
    //! `BAZEL_REAL` env var (bazelisk convention) so we never shell out to
    //! real bazel from a unit test. Each scenario is selected by the AXL
    //! caller via a `--scenario=<name>` flag that basil reads from its argv.
    /// Smoke test the BAZEL_REAL → basil → ctx.bazel.build path with a
    /// clean build that ends in `BuildFinished(0, last_message=true)`.
    /// Verifies basil is callable, the BES stream produces a clean exit,
    /// `build.wait()` reports success, AND that AXL's `build.build_events()`
    /// iterator actually receives the two events basil emitted (`Started`
    /// and `Finished`).
    /// The AXL iterator handle gets the early event burst on a warm daemon
    /// because `Build::spawn` subscribes it before bazel opens the BEP FIFO.
    /// Receives both `build_started` and `build_finished` from the success
    /// scenario, with no extras (kinds=… would also work here but a plain
    /// iter is the path most tasks take).
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

    /// Regression test for aspect-build/aspect-cli#1060.
    ///
    /// Bazel emits `BuildFinished(REMOTE_CACHE_EVICTED, last_message=true)`
    /// and exits without ever reconnecting to retry. The BES thread sets
    /// `expecting_retry = true` on the evicted finish, then must observe
    /// the writer pid is gone and close gracefully instead of looping on
    /// BrokenPipe forever. See the pid-liveness branch in
    /// `crates/axl-runtime/src/engine/bazel/stream/build_event.rs`.
    #[test]
    fn bug_1060_remote_cache_evicted_without_retry_does_not_hang() {
        use std::time::Duration;
        let result = crate::test::with_timeout(Duration::from_secs(5), || {
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

    /// `iterator()` is single-use: binding a handle that was already used by
    /// a prior `ctx.bazel.build(...)` call errors out so callers don't
    /// silently lose events.
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
    # second use of the same handle must error
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

    // --- bazel.build_events.grpc validation ---
    //
    // These exercise the Starlark surface of the failure-knob feature.
    // `.check()` runs the snippet through eval_module — the call lives at
    // module level so the function's parameter validation is the *only*
    // thing under test. No basil, no real network.

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

    #[test]
    fn grpc_rejects_zero_buffer_size() {
        let err = crate::axl_check!(
            r#"bazel.build_events.grpc(uri = "http://localhost:1", retry_max_buffer_size = 0)"#
        )
        .expect_err("expected validation error")
        .to_string();
        assert!(
            err.contains("retry_max_buffer_size") && err.contains("> 0"),
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
    retry_max_buffer_size = 16,
    timeout = "30s",
)"#
        )
        .expect("snippet should validate");
    }

    // --- iterator factory validation ---

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

    /// `kinds=` filter at the broadcaster boundary: events whose payload tag
    /// isn't in the allow-list are dropped before yielding. The success
    /// scenario emits `build_started` + `build_finished`; requesting only
    /// `build_finished` should yield exactly one event.
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

    /// `iter.drain()` makes subsequent iteration yield nothing and flips
    /// `done` to True; idempotent on a drained iter.
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
    # drain immediately; we don't care about the events
    iter.drain()
    iter.drain()  # idempotent
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

    // --- sink wait/done/failed/error ---

    /// Fresh sink: `done` is `False`, `failed` is `False`, `error` is `None`.
    /// `wait()` on an idle (unbound) sink is a no-op.
    #[test]
    fn sink_attrs_default_before_bind() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    sink = bazel.build_events.grpc(uri = "grpcs://example.com")
    if sink.done: return 1
    if sink.failed: return 2
    if sink.error != None: return 3
    # wait on never-bound sink is fine; stays idle, sets nothing.
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

    /// After binding to a build and waiting, a gRPC sink with an unparseable
    /// URI surfaces `failed = True` and a non-empty `error`. Bazel's exit
    /// code is unaffected — sink failure is the caller's call.
    #[test]
    fn sink_grpc_failure_surfaces_on_wait() {
        use std::time::Duration;
        let result = crate::test::with_timeout(Duration::from_secs(15), || {
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

    /// Re-binding a Live sink errors so callers don't accidentally pass the
    /// same handle to two concurrent builds. The retry loop in
    /// `bazel_runner.axl` avoids this by calling `sink.wait()` between
    /// attempts; this test verifies the explicit error path.
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
    # bind again without sink.wait() in between
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
