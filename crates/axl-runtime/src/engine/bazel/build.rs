use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::thread::JoinHandle;

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::StarlarkResultExt;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;

use crate::engine::r#async::rt::AsyncRuntime;

use super::iter::BuildEventIterator;
use super::iter::ExecutionLogIterator;
use super::iter::WorkspaceEventIterator;
use super::sink::execlog::ExecLogSink;
use super::sink::grpc;
use super::sink::retry::{ErrorStrategy, RetryConfig, SinkError, SinkOutcome};
use super::sink::tracing as tracing_sink;
use super::stream::BuildEventStream;
use super::stream::CappedSubscriber;
use super::stream::ExecLogStream;
use super::stream::WorkspaceEventStream;
use axl_proto::build_event_stream::BuildEvent;

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

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<bazel.build.BuildEventSink>")]
pub enum BuildEventSink {
    Grpc {
        uri: String,
        metadata: HashMap<String, String>,
        #[allocative(skip)]
        retry: RetryConfig,
    },
    File {
        path: String,
    },
    /// AXL-side iterator subscription. Carries an explicit `buffer_cap`
    /// so the runtime knows the consumer's intent and can drop the
    /// subscription rather than buffer unboundedly if the AXL task
    /// falls behind or never drains.
    Local {
        buffer_cap: usize,
    },
}

starlark_simple_value!(BuildEventSink);

#[starlark_value(type = "bazel.build.BuildEventSink")]
impl<'v> values::StarlarkValue<'v> for BuildEventSink {}

impl<'v> UnpackValue<'v> for BuildEventSink {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let value = value
            .downcast_ref_err::<BuildEventSink>()
            .into_anyhow_result()?;
        Ok(Some(value.clone()))
    }
}

impl BuildEventSink {
    /// Spawn the sink. The same `invocation_id` is passed to every BES sink
    /// so downstream backends all index this invocation under one UUID.
    fn spawn(
        &self,
        rt: AsyncRuntime,
        stream: &BuildEventStream,
        invocation_id: String,
    ) -> JoinHandle<SinkOutcome> {
        match self {
            BuildEventSink::Grpc {
                uri,
                metadata,
                retry,
            } => {
                // Use subscribe_realtime() since sinks subscribe at stream creation
                // and don't need history replay.
                grpc::Grpc::spawn(
                    rt,
                    stream.subscribe(),
                    uri.clone(),
                    metadata.clone(),
                    invocation_id,
                    retry.clone(),
                )
            }
            BuildEventSink::File { .. } => {
                unreachable!("File sinks are handled as raw file paths, not subscriber threads")
            }
            BuildEventSink::Local { .. } => {
                unreachable!("Local sinks are popped out before sink.spawn() is called")
            }
        }
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

    /// Threads forwarding to BES, tracing, or file sinks. Every sink returns
    /// a `SinkOutcome` so `wait()` can surface failures per the sink's
    /// `error_strategy`. User-configured BES sinks default to `warn`;
    /// internal sinks (tracing emitter, execlog writer) default to `abort`,
    /// since their failures indicate a real bug rather than a flaky backend.
    #[allocative(skip)]
    sink_handles: RefCell<Vec<JoinHandle<SinkOutcome>>>,

    /// Pre-subscribed receiver for the AXL-facing `build.build_events()`
    /// iterator. Created inside `Build::spawn` when (and only when) the
    /// caller declared intent via a `BuildEventSink::Local` entry — by
    /// passing `build_events=True` (sugar for a default Local sink) or
    /// `build_events=[bazel.build_events.local(...), ...]`. Subscribing
    /// here, before bazel opens the BEP FIFO and before remote sinks
    /// touch the network, guarantees the AXL receiver registers in time
    /// for the early burst (`build_started`, `target_completed`,
    /// `named_set_of_files`) regardless of subsequent task work.
    ///
    /// The subscription is capped: if the AXL consumer never starts
    /// draining (or falls behind by more than `buffer_cap` events), the
    /// broadcaster drops the subscription and the receiver sees
    /// `Disconnected` — bounding memory growth for misbehaving tasks.
    ///
    /// `None` when no Local sink was configured, or after the first
    /// `build_events()` call has consumed the receiver. Subsequent
    /// calls error rather than silently subscribing late.
    #[allocative(skip)]
    early_event_subscriber: RefCell<Option<CappedSubscriber<BuildEvent>>>,

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
        (build_events, sinks): (bool, Vec<BuildEventSink>),
        (execution_logs, execlog_sinks): (bool, Vec<ExecLogSink>),
        workspace_events: bool,
        flags: Vec<String>,
        startup_flags: Vec<String>,
        inherit_stdout: bool,
        inherit_stderr: bool,
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

        // Split BES sinks by kind:
        //   File   — accumulate raw pipe bytes in memory, written after FIFO closes.
        //   Local  — the AXL-side iterator subscription; at most one. Carries
        //            the buffer_cap used to bound undrained accumulation.
        //   Grpc/… — real-time broadcaster subscriptions; spawn their own threads.
        let mut bes_file_paths: Vec<String> = vec![];
        let mut bes_subscriber_sinks: Vec<BuildEventSink> = vec![];
        let mut local_sink_cap: Option<usize> = None;
        for sink in sinks {
            match &sink {
                BuildEventSink::File { path } => bes_file_paths.push(path.clone()),
                BuildEventSink::Local { buffer_cap } => {
                    if local_sink_cap.is_some() {
                        return Err(io::Error::other(
                            "ctx.bazel.build / ctx.bazel.test: multiple Local BES sinks \
                             configured. Pass at most one `bazel.build_events.local(...)` \
                             entry; the AXL task subscribes via `build.build_events()`.",
                        ));
                    }
                    local_sink_cap = Some(*buffer_cap);
                }
                _ => bes_subscriber_sinks.push(sink),
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

        // TODO: if not inheriting, we should pipe and make the streams available to AXL
        cmd.stdout(if inherit_stdout {
            Stdio::inherit()
        } else {
            Stdio::null()
        });
        cmd.stderr(if inherit_stderr {
            Stdio::inherit()
        } else {
            Stdio::null()
        });
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
            Some(p) => Some(BuildEventStream::spawn(p, pid, child.id(), bes_file_paths)?),
            None => None,
        };

        // Build Event sinks for forwarding the build events.
        //
        // Generate ONE invocation_id and hand it to every sink so all backends
        // key this invocation under the same UUID. This lets us build a single
        // "View invocation" URL that works on whichever backend a user checks.
        // Without this, each sink would mint its own UUID and we'd have no way
        // to know which one corresponded to any particular viewer URL.
        //
        // Subscribing here (after `cmd.spawn()` but before bazel's BEP open
        // unblocks the BES thread) is correct: the BES thread is currently
        // blocked in `Pipe::open` waiting for bazel to open the FIFO write
        // end, which won't happen until bazel finishes JVM startup — well
        // after these subscribe calls land.
        let mut sink_handles: Vec<JoinHandle<SinkOutcome>> = vec![];

        // Eagerly subscribe the AXL-facing receiver when the caller
        // declared intent via a `BuildEventSink::Local` entry (either
        // explicitly, or via the `build_events=True` sugar in the AXL
        // surface). Subscribing here — before bazel's BEP FIFO opens
        // and before remote sinks touch the network — closes the race
        // window where the early burst (`build_started`,
        // `target_completed`, `named_set_of_files`) was emitted before
        // a lazy `build_events()` subscribe in the AXL task could
        // register.
        //
        // The cap bounds memory growth: if the AXL task never drains
        // (or falls behind by more than `buffer_cap` events), the
        // broadcaster drops the subscription on the next overflow send
        // and the AXL iterator sees `Disconnected`.
        //
        // No Local sink → no eager subscribe; tasks that only pass
        // remote sinks pay nothing for buffering, and `build_events()`
        // errors if called (the caller didn't ask for local delivery).
        let early_event_subscriber: Option<CappedSubscriber<BuildEvent>> =
            match (build_event_stream.as_ref(), local_sink_cap) {
                (Some(s), Some(cap)) => Some(s.subscribe_capped(cap)),
                _ => None,
            };

        let sink_invocation_id: Option<String> = if !bes_subscriber_sinks.is_empty() {
            let invocation_id = uuid::Uuid::new_v4().to_string();
            let debug = std::env::var_os("ASPECT_DEBUG")
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            if debug {
                eprintln!(
                    "BES sinks: spawning {} subscriber sink(s) sink_invocation_id={}",
                    bes_subscriber_sinks.len(),
                    invocation_id
                );
            }
            for sink in bes_subscriber_sinks {
                let handle = sink.spawn(
                    rt.clone(),
                    build_event_stream.as_ref().unwrap(),
                    invocation_id.clone(),
                );
                sink_handles.push(handle);
            }
            Some(invocation_id)
        } else {
            None
        };

        // Decoded ExecLog File sinks — spawned after the execlog stream so the
        // receiver is valid. They disconnect naturally when execlog_stream is joined.
        for sink in decoded_sinks {
            if let ExecLogSink::File { path } = sink {
                let handle =
                    ExecLogSink::spawn_file(execlog_stream.as_ref().unwrap().receiver(), path);
                sink_handles.push(handle);
            }
        }
        if build_events {
            // Use subscribe_realtime() since this subscribes at stream creation
            // and doesn't need history replay.
            sink_handles.push(tracing_sink::Tracing::spawn(
                build_event_stream.as_ref().unwrap().subscribe(),
            ))
        }

        drop(_enter);
        Ok(Self {
            child: RefCell::new(child),
            build_event_stream: RefCell::new(build_event_stream),
            workspace_event_stream: RefCell::new(workspace_event_stream),
            execlog_stream: RefCell::new(execlog_stream),
            sink_handles: RefCell::new(sink_handles),
            sink_invocation_id: RefCell::new(sink_invocation_id),
            early_event_subscriber: RefCell::new(early_event_subscriber),
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
    // Creates an iterable `BuildEventIterator` type.
    // Every call to this function will return a new iterator.
    // TODO: explain backpressure and build events sinks falling behind on poor network conditions.
    fn build_events<'v>(this: values::Value<'v>) -> anyhow::Result<BuildEventIterator> {
        let build = this.downcast_ref::<Build>().unwrap();
        let _ = build
            .build_event_stream
            .borrow()
            .as_ref()
            .ok_or(anyhow::anyhow!(
                "call `ctx.bazel.build` with `build_events = True` (or include \
             `bazel.build_events.local(...)` in the sink list) to receive build events."
            ))?;

        // Hand over the pre-subscribed receiver created in `Build::spawn`.
        // The intent was declared up-front via a `BuildEventSink::Local`
        // entry, so the subscription registered before bazel opened the
        // BEP FIFO. Calling this method twice is an error: the cap
        // accounting belongs to one consumer; multi-consumer use needs
        // the user to subscribe their own broadcaster on top.
        let recv = build
            .early_event_subscriber
            .borrow_mut()
            .take()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "`build.build_events()` may be called at most once per build, and only when \
                 `ctx.bazel.build` was called with `build_events = True` (or an explicit \
                 `bazel.build_events.local(...)` in the sink list)."
                )
            })?;

        Ok(BuildEventIterator::new(recv))
    }

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

        // Resolve all sink threads. Each returns a `SinkOutcome` describing
        // its terminal state and how to surface a failure (per Bazel's BES
        // upload error policy plus `abort` for internal sinks).
        // `Warn` and `Ignore` already printed (or stayed silent) at the sink;
        // only `Abort` and `FailAtEnd` propagate further.
        let mut abort_msg: Option<String> = None;
        let mut fail_at_end = false;
        for handle in build.sink_handles.take() {
            match handle.join() {
                Ok(Ok(())) => continue,
                Ok(Err(SinkError {
                    strategy,
                    last_error,
                })) => match strategy {
                    ErrorStrategy::Abort => {
                        if abort_msg.is_none() {
                            abort_msg = Some(last_error);
                        }
                    }
                    ErrorStrategy::FailAtEnd => {
                        fail_at_end = true;
                    }
                    ErrorStrategy::Warn | ErrorStrategy::Ignore => {}
                },
                Err(err) => anyhow::bail!("sink thread panicked: {:#?}", err),
            }
        }

        // Drop the span to end the trace
        drop(build.span.replace(tracing::Span::none()));

        if let Some(msg) = abort_msg {
            anyhow::bail!("BES sink failure (abort): {}", msg);
        }

        let success = result.success() && !fail_at_end;
        let code = finalize_exit_code(result.code(), fail_at_end);

        Ok(BuildStatus { success, code })
    }
}

/// Compute the surfaced exit code for `wait()`.
///
/// Sink-induced `fail_at_end` failures synthesize the reserved code 36 so
/// callers can distinguish a sink failure from a real Bazel failure. We
/// only rewrite when Bazel actually exited cleanly (`Some(0)`); a `None`
/// means the child was killed by a signal and that abnormal termination
/// must be preserved unchanged, and a nonzero exit is a genuine Bazel
/// failure that already conveys the right signal to callers.
fn finalize_exit_code(bazel_code: Option<i32>, fail_at_end: bool) -> Option<i32> {
    if fail_at_end && bazel_code == Some(0) {
        Some(36)
    } else {
        bazel_code
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
    #[test]
    fn build_events_reach_axl_for_success_scenario() {
        let exit = crate::test::eval(
            r#"
def _impl(ctx):
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = True,
        inherit_stderr = False,
    )
    started = 0
    finished = 0
    other = 0
    for event in build.build_events():
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
    build = ctx.bazel.build(
        flags = ["--scenario=cache_evicted_no_retry"],
        build_events = True,
        inherit_stderr = False,
    )
    for _ in build.build_events():
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

    // --- bazel.build_events.grpc validation ---
    //
    // These exercise the Starlark surface of the failure-knob feature.
    // `.check()` runs the snippet through eval_module — the call lives at
    // module level so the function's parameter validation is the *only*
    // thing under test. No basil, no real network.

    #[test]
    fn grpc_rejects_unknown_error_strategy() {
        let err = crate::axl_check!(
            r#"bazel.build_events.grpc(uri = "http://localhost:1", error_strategy = "nope")"#
        )
        .expect_err("expected validation error")
        .to_string();
        assert!(
            err.contains("error_strategy") && err.contains("nope"),
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
    error_strategy = "fail_at_end",
)"#
        )
        .expect("snippet should validate");
    }

    // --- error_strategy end-to-end ---
    //
    // The tests below feed an unparseable URI into the gRPC sink so
    // `Channel::from_shared` returns `InvalidEndpoint` (non-retryable) and
    // the sink terminates without touching the network. We deliberately
    // avoid a real TCP target — connect-refused timing varies by platform
    // and would make these tests flaky.
    //
    // basil is told to run the "success" scenario, so bazel itself exits
    // 0; only the sink path differs across these tests.

    /// `error_strategy = "ignore"`: terminal sink error is suppressed.
    /// `wait()` returns success and the original bazel exit code.
    #[test]
    fn grpc_error_strategy_ignore_swallows_terminal_failure() {
        use std::time::Duration;
        let result = crate::test::with_timeout(Duration::from_secs(15), || {
            crate::test::eval(
                r#"
def _impl(ctx):
    sink = bazel.build_events.grpc(
        uri = "not a uri",
        max_retries = 0,
        retry_min_delay = "0s",
        error_strategy = "ignore",
    )
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [sink],
        inherit_stderr = False,
    )
    status = build.wait()
    if not status.success: return 1
    if status.code != 0: return 2
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

    /// `error_strategy = "fail_at_end"`: bazel succeeded but the sink
    /// failed terminally — `wait()` reports `success = False` with the
    /// reserved exit code 36 so callers can distinguish a sink-induced
    /// failure from a genuine build failure.
    #[test]
    fn grpc_error_strategy_fail_at_end_reports_code_36() {
        use std::time::Duration;
        let result = crate::test::with_timeout(Duration::from_secs(15), || {
            crate::test::eval(
                r#"
def _impl(ctx):
    sink = bazel.build_events.grpc(
        uri = "not a uri",
        max_retries = 0,
        retry_min_delay = "0s",
        error_strategy = "fail_at_end",
    )
    build = ctx.bazel.build(
        flags = ["--scenario=success"],
        build_events = [sink],
        inherit_stderr = False,
    )
    status = build.wait()
    if status.success: return 1
    if status.code != 36: return 2
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

    /// `error_strategy = "fail_at_end"` + bazel killed by signal: the
    /// sink failure must not mask the abnormal termination. wait()
    /// reports `success = False` (sink failed) but `code = None`
    /// (preserve signal-kill), not `Some(36)`. Regression for the
    /// `result.code().unwrap_or(0) == 0` bug that conflated signal
    /// kills with clean exits and overwrote them with the synthetic
    /// fail_at_end code.
    #[test]
    fn grpc_error_strategy_fail_at_end_preserves_signal_kill() {
        use std::time::Duration;
        let result = crate::test::with_timeout(Duration::from_secs(15), || {
            crate::test::eval(
                r#"
def _impl(ctx):
    sink = bazel.build_events.grpc(
        uri = "not a uri",
        max_retries = 0,
        retry_min_delay = "0s",
        error_strategy = "fail_at_end",
    )
    build = ctx.bazel.build(
        flags = ["--scenario=signal_killed_sigkill"],
        build_events = [sink],
        inherit_stderr = False,
    )
    status = build.wait()
    if status.success: return 1
    if status.code != None: return 2
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

    /// `error_strategy = "fail_at_end"` + genuine non-zero Bazel exit:
    /// preserve Bazel's exit code rather than overwriting with the
    /// synthetic 36, so callers can still see e.g. `code = 2` (build
    /// failure) or `code = 39` (REMOTE_CACHE_EVICTED).
    #[test]
    fn grpc_error_strategy_fail_at_end_preserves_genuine_bazel_failure() {
        use std::time::Duration;
        let result = crate::test::with_timeout(Duration::from_secs(15), || {
            crate::test::eval(
                r#"
def _impl(ctx):
    sink = bazel.build_events.grpc(
        uri = "not a uri",
        max_retries = 0,
        retry_min_delay = "0s",
        error_strategy = "fail_at_end",
    )
    build = ctx.bazel.build(
        flags = ["--scenario=nonzero_exit"],
        build_events = [sink],
        inherit_stderr = False,
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
        .expect("test hung");
        assert_eq!(result.expect("run_task"), Some(0));
    }

    // --- finalize_exit_code ---
    //
    // Unit-level coverage of the wait() exit-code mapping. Complements
    // the e2e tests above by covering the matrix of (bazel exit, sink
    // outcome) combinations without the cost of spawning basil for each.

    #[test]
    fn finalize_exit_code_preserves_signal_kill_under_fail_at_end() {
        // `None` from `ExitStatus::code()` means the child was killed by a
        // signal. fail_at_end must not mask that as the synthetic 36;
        // callers need to see the abnormal termination unchanged.
        assert_eq!(super::finalize_exit_code(None, true), None);
    }

    #[test]
    fn finalize_exit_code_preserves_signal_kill_without_fail_at_end() {
        assert_eq!(super::finalize_exit_code(None, false), None);
    }

    #[test]
    fn finalize_exit_code_rewrites_clean_exit_to_36_under_fail_at_end() {
        // Bazel succeeded but a sink reported terminal failure. wait()
        // exposes 36 so callers can distinguish a sink-induced failure
        // from a genuine build failure.
        assert_eq!(super::finalize_exit_code(Some(0), true), Some(36));
    }

    #[test]
    fn finalize_exit_code_preserves_clean_exit_without_fail_at_end() {
        assert_eq!(super::finalize_exit_code(Some(0), false), Some(0));
    }

    #[test]
    fn finalize_exit_code_preserves_genuine_bazel_failure_under_fail_at_end() {
        // A real Bazel failure (any non-zero code) is already meaningful;
        // fail_at_end must not overwrite it with 36 or callers can't tell
        // the original failure mode (e.g. 2 = build failure, 39 =
        // REMOTE_CACHE_EVICTED) from a sink failure.
        assert_eq!(super::finalize_exit_code(Some(2), true), Some(2));
        assert_eq!(super::finalize_exit_code(Some(39), true), Some(39));
    }

    #[test]
    fn finalize_exit_code_preserves_genuine_bazel_failure_without_fail_at_end() {
        assert_eq!(super::finalize_exit_code(Some(2), false), Some(2));
    }
}
