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
use super::iter::OutputEventIterator;
use super::iter::WorkspaceEventIterator;
use super::sink::execlog::ExecLogSink;
use super::sink::grpc;
use super::sink::retry::{RetryConfig, SinkOutcome, SinkStats};
use super::sink::tracing as tracing_sink;
use super::stream::BuildEventStream;
use super::stream::ExecLogStream;
use super::stream::Subscriber;
use super::stream::WorkspaceEventStream;
use super::stream::output::LineProcessor;
use super::stream::processors::{
    CollapseRepeats, LineMatcher, MatchResponder, OutputSignals, PendingMatch,
};

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
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(build_event_sink_methods)
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
    /// Block until this sink finishes flushing. Idempotent.
    fn wait<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
        let sink = this
            .downcast_ref_err::<BuildEventSink>()
            .into_anyhow_result()?;
        // Take Live out of the Mutex so we don't hold the lock across the
        // blocking join.
        let live = {
            let mut phase = sink.phase.lock().unwrap();
            match std::mem::replace(&mut *phase, SinkPhase::Idle) {
                SinkPhase::Live(live) => live,
                SinkPhase::Idle => return Ok(NoneOr::None),
            }
        };
        let (outcome, stats): (Result<(), String>, SinkStats) = match live {
            SinkLive::Grpc { join } => match join.join() {
                Ok((stats, Ok(()))) => (Ok(()), stats),
                Ok((stats, Err(e))) => (Err(e.last_error), stats),
                Err(_) => (
                    Err("sink worker thread panicked".to_string()),
                    SinkStats::default(),
                ),
            },
            SinkLive::File { signal } => (signal.wait(), SinkStats::default()),
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
        Ok(NoneOr::None)
    }
}

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

    /// Subscribe the iterator's receiver. Must run before bazel opens the
    /// BEP FIFO so the early burst is buffered.
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
            // Take recv out of the Mutex so we don't hold the lock across
            // the blocking call.
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

    /// Non-blocking pop. Returns `None` when empty or disconnected. Honors
    /// the `kinds=` filter.
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

/// How the captured stderr fd is allocated for a build.
///
/// `Pipe` is a plain anonymous pipe for non-TTY contexts (CI, redirected
/// output): Bazel emits clean newline-terminated lines. `Pty` allocates a
/// pseudo-terminal so Bazel keeps its live curses UI, forwarded near-verbatim.
/// The mode is chosen when the processor is constructed (see
/// `bazel.output.processor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    Pipe,
    Pty,
}

/// Starlark handle passed as `ctx.bazel.build(output = ...)` to enable stderr
/// capture + forwarding. Created via `bazel.output.processor(...)`; carries
/// only configuration (patterns are pre-compiled regexes), so one handle can
/// be reused across retry attempts — each `Build::spawn` builds a fresh
/// pipeline and signal state from it.
///
/// `Build::spawn` reads the capture mode to allocate the child's stderr and
/// assembles the processing chain from the remaining fields (see
/// `super::stream::processors`): observer `LineMatcher` stages first (so they
/// see every record as Bazel wrote it), then the `MatchResponder` (which may
/// rewrite records), then `CollapseRepeats`.
#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.output.OutputProcessor>")]
pub struct OutputProcessor {
    #[allocative(skip)]
    mode: CaptureMode,
    /// Fold runs of identical lines into one + a repeat-count annotation.
    collapse_repeats: bool,
    /// `(id, regex)` observer patterns; hits are recorded on `OutputSignals`
    /// and exposed via `build.output_matches()`.
    #[allocative(skip)]
    match_patterns: Vec<(String, regex::Regex)>,
    /// Regexes that latch `OutputSignals::fatal` (exposed via
    /// `build.output_fatal` / `output_fatal_line`).
    #[allocative(skip)]
    fatal_patterns: Vec<(String, regex::Regex)>,
    /// `(id, regex)` interactive patterns: a matching line is held until the
    /// consumer of `build.output_events()` answers keep/replace/drop, or
    /// `respond_timeout` fires (fail-open: original line forwards).
    #[allocative(skip)]
    respond_patterns: Vec<(String, regex::Regex)>,
    #[allocative(skip)]
    respond_timeout: std::time::Duration,
}

impl OutputProcessor {
    pub fn new(
        mode: CaptureMode,
        collapse_repeats: bool,
        match_patterns: Vec<(String, regex::Regex)>,
        fatal_patterns: Vec<(String, regex::Regex)>,
        respond_patterns: Vec<(String, regex::Regex)>,
        respond_timeout: std::time::Duration,
    ) -> Self {
        Self {
            mode,
            collapse_repeats,
            match_patterns,
            fatal_patterns,
            respond_patterns,
            respond_timeout,
        }
    }

    pub fn mode(&self) -> CaptureMode {
        self.mode
    }

    /// Assemble the per-invocation pipeline, its shared signal state, and —
    /// when `respond_patterns` are configured — the receiver
    /// `build.output_events()` hands to the consumer.
    fn build_pipeline(
        &self,
    ) -> (
        Vec<Box<dyn LineProcessor>>,
        Arc<OutputSignals>,
        Option<std::sync::mpsc::Receiver<PendingMatch>>,
    ) {
        let signals = Arc::new(OutputSignals::default());
        let mut chain: Vec<Box<dyn LineProcessor>> = vec![];
        if !self.fatal_patterns.is_empty() {
            let s = signals.clone();
            chain.push(Box::new(LineMatcher::new(
                self.fatal_patterns.clone(),
                Box::new(move |_id, line| s.set_fatal(line)),
            )));
        }
        if !self.match_patterns.is_empty() {
            let s = signals.clone();
            chain.push(Box::new(LineMatcher::new(
                self.match_patterns.clone(),
                Box::new(move |id, line| s.record_match(id, line)),
            )));
        }
        let events = if self.respond_patterns.is_empty() {
            None
        } else {
            let (tx, rx) = std::sync::mpsc::channel();
            chain.push(Box::new(MatchResponder::new(
                self.respond_patterns.clone(),
                tx,
                self.respond_timeout,
            )));
            Some(rx)
        };
        if self.collapse_repeats {
            chain.push(Box::new(CollapseRepeats::new()));
        }
        (chain, signals, events)
    }
}

impl<'v> AllocValue<'v> for OutputProcessor {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for OutputProcessor {
    type Error = anyhow::Error;
    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        let v = value
            .downcast_ref_err::<OutputProcessor>()
            .into_anyhow_result()?;
        Ok(Some(v.clone()))
    }
}

#[starlark_value(type = "bazel.output.OutputProcessor")]
impl<'v> values::StarlarkValue<'v> for OutputProcessor {}

fn matches_kinds(event: &BuildEvent, kinds: Option<&Arc<HashSet<i32>>>) -> bool {
    let Some(kinds) = kinds else {
        return true;
    };
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
        eprintln!("{grey}INFO: {}{reset}", version_line(version));
    }
    if announce.command {
        eprintln!("{grey}INFO: Spawning: {}{reset}", render_command(cmd));
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

    /// Captured-stderr forwarder, present only when the build was spawned with
    /// `output = bazel.output.processor(...)`. Joined in `wait()` after the
    /// child is reaped so all forwarded stderr is flushed before the task
    /// prints its terminal summary.
    #[allocative(skip)]
    output_stream: RefCell<Option<super::stream::OutputStream>>,

    /// Shared state the capture pipeline's matcher stages feed (fatal flag,
    /// match hits); backs the `output_fatal` / `output_fatal_line` attributes
    /// and `output_matches()`. `None` when capture is off.
    #[allocative(skip)]
    output_signals: Option<Arc<OutputSignals>>,

    /// Held-line events from the pipeline's `MatchResponder`, present only
    /// when `respond_patterns` were configured. Taken (single-use) by
    /// `output_events()`; a matched line stays held until the consumer
    /// responds or the responder's fail-open timeout fires.
    #[allocative(skip)]
    output_events: RefCell<Option<std::sync::mpsc::Receiver<PendingMatch>>>,

    /// Shared UUID every gRPC sink indexes this invocation under. Minted
    /// before bazel emits `build_started` so forwarders can start
    /// immediately; distinct from Bazel's `build_started.uuid`.
    #[allocative(skip)]
    sink_invocation_id: RefCell<Option<String>>,

    #[allocative(skip)]
    child: RefCell<Child>,

    /// RAII guard that registers the bazel client PID with `bazel::live`
    /// for the lifetime of the build. On OS-level shutdown signals to
    /// aspect-cli, the binary's signal handler iterates the live registry
    /// and forwards SIGINT to each registered client so bazel subprocesses
    /// don't outlive aspect-cli.
    ///
    /// Wrapped in `RefCell<Option<…>>` so `wait()` / `try_wait()` can
    /// `.take()` it the moment the child is observed exited. Otherwise the
    /// PID stays in the registry until the Starlark `Build` object is
    /// garbage-collected — and if the OS reuses the PID in that window,
    /// the shutdown handler would SIGINT/SIGKILL an unrelated process.
    #[allocative(skip)]
    live_guard: RefCell<Option<super::live::LiveBazelGuard>>,

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
        output: Option<OutputProcessor>,
        directory: Option<String>,
        announce: AnnounceSpawn,
        rt: AsyncRuntime,
    ) -> Result<Build, std::io::Error> {
        let (pid, version) = super::info::server_info()?;

        let span = tracing::info_span!(
            "ctx.bazel.build",
            build_events = build_events,
            workspace_events = workspace_events,
            execution_logs = execution_logs,
            flags = ?flags
        );
        let _enter = span.enter();

        let targets: Vec<String> = targets.into_iter().collect();

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

        let mut execlog_stream = if execution_logs {
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
        announce_spawn(announce, version.as_ref(), &cmd);

        cmd.stdout(stdout);
        // When capturing, the child's stderr goes to a runtime-owned pipe/PTY
        // instead of the resolved `stderr` Stdio; the `OutputStream` started
        // after spawn reads, processes, and forwards it to the real stderr.
        let mut capture = match &output {
            Some(p) => {
                let (child_stderr, capture) = super::capture::Capture::open(p.mode())?;
                cmd.stderr(child_stderr);
                Some(capture)
            }
            None => {
                cmd.stderr(stderr);
                None
            }
        };
        cmd.stdin(Stdio::null());

        let child = cmd
            .spawn()
            .map_err(|e| io::Error::other(format!("failed to spawn bazel: {e}")))?;

        // Start forwarding captured stderr now that the child holds the write
        // end. Drop the parent's PTY-slave copy first (release_after_spawn) so
        // the master read can observe EOF when the child exits — otherwise the
        // forwarder thread would hang forever in `wait()`.
        //
        // The forwarder writes to the real stderr on its own thread. Any
        // aspect-cli stderr written on the main thread while the build runs
        // (e.g. status lines) races it on a separate handle, so callers that
        // capture output should route their own stderr elsewhere for the
        // build's duration.
        let (output_stream, output_signals, output_events) = match capture.take() {
            Some(mut c) => {
                c.release_after_spawn();
                let (chain, signals, events) = output
                    .as_ref()
                    .expect("capture is only allocated when output is set")
                    .build_pipeline();
                let stream =
                    super::stream::OutputStream::spawn(c.reader, Box::new(std::io::stderr()), chain);
                (Some(stream), Some(signals), events)
            }
            None => (None, None, None),
        };

        // Register the bazel client with the live-subprocess registry so
        // aspect-cli's OS-signal handler can forward SIGINT to it on
        // CI cancellation. The guard is stored on `Self` and unregisters
        // when the `Build` is dropped (after `wait()`).
        let live_guard = super::live::register(child.id());

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

        // Decoded execlog file sinks belong to the execlog stream — joined
        // (and write errors propagated) inside `execlog_stream.join()`.
        if let Some(stream) = execlog_stream.as_mut() {
            for sink in decoded_sinks {
                if let ExecLogSink::File { path } = sink {
                    stream.attach_file_sink(ExecLogSink::spawn_file(stream.receiver(), path));
                }
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
            output_stream: RefCell::new(output_stream),
            output_signals,
            output_events: RefCell::new(output_events),
            sink_invocation_id: RefCell::new(sink_invocation_id),
            live_guard: RefCell::new(Some(live_guard)),
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
            // Whether a fatal_pattern from `bazel.output.processor(...)`
            // matched a captured stderr line. False when capture is off.
            "output_fatal" => Some(heap.alloc(
                self.output_signals
                    .as_ref()
                    .is_some_and(|s| s.fatal()),
            )),
            // The first fatal line matched, or None.
            "output_fatal_line" => {
                let line = self.output_signals.as_ref().and_then(|s| s.fatal_line());
                Some(match line {
                    Some(l) => heap.alloc_str(&l).to_value(),
                    None => values::Value::new_none(),
                })
            }
            // Millis of captured-stderr silence (since the last byte, or since
            // spawn if none arrived). 0 when capture is off or after `wait()`
            // has drained the stream — poll it mid-build, alongside child
            // liveness, to detect a hung invocation.
            "output_idle_ms" => {
                let idle = self
                    .output_stream
                    .borrow()
                    .as_ref()
                    .map(|s| s.idle_ms())
                    .unwrap_or(0);
                Some(heap.alloc(idle.min(i32::MAX as u64) as i32))
            }
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: values::Heap<'v>) -> bool {
        matches!(
            attribute,
            "sink_invocation_id" | "output_fatal" | "output_fatal_line" | "output_idle_ms"
        )
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

    /// The held-line event stream for this build's `respond_patterns`
    /// (see `bazel.output.processor`). Single-use: the first call takes the
    /// stream; drain it with `try_pop()` during the build and answer each
    /// event with `keep()` / `replace(text)` / `drop()`. A matched line is
    /// not forwarded to the terminal until answered (or the fail-open
    /// timeout forwards the original), so drain promptly.
    fn output_events<'v>(this: values::Value<'v>) -> anyhow::Result<OutputEventIterator> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        let recv = build.output_events.borrow_mut().take().ok_or_else(|| {
            anyhow::anyhow!(
                "output_events() is single-use and requires `output = \
                 bazel.output.processor(respond_patterns = {{...}})` on the build"
            )
        })?;
        Ok(OutputEventIterator::new(recv))
    }

    /// Snapshot of `(id, line)` hits for the `match_patterns` configured on
    /// this build's `bazel.output.processor(...)`. Safe to call during the
    /// build (hits so far) or after `wait()` (all hits). Empty when capture
    /// is off or nothing matched. At most 1024 hits are recorded per
    /// invocation; lines are truncated to 2048 bytes.
    fn output_matches<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<(String, String)>> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        Ok(build
            .output_signals
            .as_ref()
            .map(|s| s.matches())
            .unwrap_or_default())
    }

    fn try_wait<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<BuildStatus>> {
        let build = this.downcast_ref_err::<Build>().into_anyhow_result()?;
        let status = build.child.borrow_mut().try_wait()?;
        Ok(match status {
            Some(status) => {
                // Child has been reaped — release the PID registration
                // immediately so a reused PID can't be targeted by a
                // later shutdown-signal escalation.
                build.live_guard.borrow_mut().take();
                NoneOr::Other(BuildStatus {
                    success: status.success(),
                    code: status.code(),
                })
            }
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

        // Child has been reaped — release the PID registration before any
        // other work in this function. Otherwise the PID could be reused
        // by the OS while we drain BES/execlog sinks, and a CI cancel in
        // that window would target an unrelated process.
        build.live_guard.borrow_mut().take();

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

        // Drain the captured-stderr forwarder. The child has exited (reaped
        // above), so its stderr write end is closed and the reader reaches
        // EOF; joining here guarantees all forwarded stderr is flushed before
        // the caller (`_emit_terminal`) prints the task's terminal summary.
        let output_stream = build.output_stream.take();
        if let Some(mut output_stream) = output_stream {
            match output_stream.join() {
                Ok(_) => {}
                Err(err) => anyhow::bail!("output stream thread error: {}", err),
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

    /// `OutputProcessor::build_pipeline` assembles matcher stages before the
    /// collapser, so matchers see every record — including the repeats the
    /// collapser folds away.
    #[test]
    fn output_pipeline_matchers_see_collapsed_repeats() {
        use crate::engine::bazel::stream::OutputStream;
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct SharedSink(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedSink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let rx = |p: &str| regex::Regex::new(p).unwrap();
        let processor = super::OutputProcessor::new(
            super::CaptureMode::Pipe,
            true,
            vec![("warn".to_string(), rx("^WARNING:"))],
            vec![(
                "Server terminated abruptly".to_string(),
                rx("Server terminated abruptly"),
            )],
            vec![("secret".to_string(), rx("password"))],
            std::time::Duration::from_secs(5),
        );
        let (chain, signals, events) = processor.build_pipeline();

        // Consumer thread standing in for AXL: rewrite the matched line.
        let events = events.expect("respond_patterns configured");
        let consumer = std::thread::spawn(move || {
            for ev in events {
                let reply = ev.reply.clone();
                let _ = reply.send(super::super::stream::processors::Verdict::Replace(
                    b"(password elided)".to_vec(),
                ));
            }
        });

        let input = b"WARNING: flaky\nWARNING: flaky\nWARNING: flaky\npassword=hunter2\nServer terminated abruptly\n";
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut stream = OutputStream::spawn(
            Box::new(std::io::Cursor::new(input.to_vec())),
            Box::new(SharedSink(sink.clone())),
            chain,
        );
        stream.join().unwrap();
        consumer.join().unwrap();

        // Forwarded output: repeats collapsed, matched line replaced by the
        // consumer's verdict before display.
        let out = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
        assert_eq!(
            out,
            "WARNING: flaky\n(last line repeated 2 more times)\n(password elided)\nServer terminated abruptly\n"
        );

        // Matchers ran before the collapser: every repeat was a hit.
        let hits = signals.matches();
        assert_eq!(hits.len(), 3);
        assert!(hits.iter().all(|(id, _)| id == "warn"));

        // The fatal pattern latched with its line.
        assert!(signals.fatal());
        assert_eq!(
            signals.fatal_line().as_deref(),
            Some("Server terminated abruptly")
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

    /// The captured-output Starlark surface end-to-end against a real spawn:
    /// `bazel.output.processor(...)` arg unpacking, `output=` on
    /// `ctx.bazel.build`, and the `output_*` attributes / `output_matches()`.
    /// basil emits nothing on stderr in the success scenario, so signals stay
    /// at their defaults — the point is the plumbing, not the matching (the
    /// pipeline itself is covered by `output_pipeline_matchers_see_collapsed_repeats`).
    #[test]
    fn output_processor_starlark_surface() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    processor = bazel.output.processor(
        tty = False,
        collapse_repeats = True,
        match_patterns = {"warn": "WARNING:"},
        fatal_patterns = ["Server terminated abruptly"],
    )
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        output = processor,
    )
    status = build.wait()
    if not status.success: return 1
    if build.output_fatal: return 2
    if build.output_fatal_line != None: return 3
    if build.output_matches() != []: return 4
    if build.output_idle_ms < 0: return 5
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect("run_task");

        assert_eq!(exit, Some(0));
    }

    /// The interactive respond/replace flow end-to-end from Starlark: basil
    /// emits console lines on stderr; AXL drains `output_events()`, replaces
    /// the credential line and keeps the rest; observer patterns record hits.
    /// The blocking `for` over the iterator also proves the stream closes
    /// (and the loop exits) when the capture pipeline ends.
    #[test]
    fn output_events_respond_and_replace() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    processor = bazel.output.processor(
        tty = False,
        match_patterns = {"warn": "^WARNING:"},
        respond_patterns = {"secret": "password"},
    )
    build = ctx.bazel.build(
        flags = ["--scenario=stderr_chatter"],
        output = processor,
    )
    events = build.output_events()
    replaced = 0
    kept = 0
    for ev in events:
        if ev.id == "secret":
            ev.replace("(password elided)")
            replaced += 1
        else:
            ev.keep()
            kept += 1
    status = build.wait()
    if not status.success: return 1
    if replaced != 1: return 2
    if kept != 0: return 3
    matches = build.output_matches()
    if len(matches) != 1: return 4
    if matches[0][0] != "warn": return 5
    if build.output_fatal: return 6
    return 0

Test = task(implementation = _impl)
"#,
        )
        .with_fake_bazel()
        .run_task(0)
        .expect("run_task");

        assert_eq!(exit, Some(0));
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
