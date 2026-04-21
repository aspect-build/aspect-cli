use std::cell::RefCell;
use std::collections::HashMap;
use std::env::var;
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

use super::execlog_sink::ExecLogSink;
use super::iter::BuildEventIterator;
use super::iter::ExecutionLogIterator;
use super::iter::WorkspaceEventIterator;
use super::stream::BuildEventStream;
use super::stream::ExecLogStream;
use super::stream::WorkspaceEventStream;
use super::stream_sink::GrpcEventStreamSink;
use super::stream_tracing::TracingEventStreamSink;

fn debug_mode() -> bool {
    match var("ASPECT_DEBUG") {
        Ok(val) => !val.is_empty(),
        _ => false,
    }
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

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative, Clone)]
#[display("<bazel.build.BuildEventSink>")]
pub enum BuildEventSink {
    Grpc {
        uri: String,
        metadata: HashMap<String, String>,
    },
    File {
        path: String,
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
    ) -> JoinHandle<()> {
        match self {
            BuildEventSink::Grpc { uri, metadata } => {
                // Use subscribe_realtime() since sinks subscribe at stream creation
                // and don't need history replay.
                GrpcEventStreamSink::spawn(
                    rt,
                    stream.subscribe(),
                    uri.clone(),
                    metadata.clone(),
                    invocation_id,
                )
            }
            BuildEventSink::File { .. } => {
                unreachable!("File sinks are handled as raw file paths, not subscriber threads")
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

    #[allocative(skip)]
    sink_handles: RefCell<Vec<JoinHandle<()>>>,

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

        let mut cmd = Command::new("bazel");
        cmd.args(startup_flags);
        cmd.arg(verb);
        cmd.args(flags);

        if let Some(current_dir) = current_dir {
            cmd.current_dir(current_dir);
        }

        // Split BES sinks: File sinks accumulate raw pipe bytes in memory and
        // are written after the FIFO closes; subscriber sinks (Grpc, etc.) get
        // a real-time channel subscription.
        let mut bes_file_paths: Vec<String> = vec![];
        let mut bes_subscriber_sinks: Vec<BuildEventSink> = vec![];
        for sink in sinks {
            match &sink {
                BuildEventSink::File { path } => bes_file_paths.push(path.clone()),
                _ => bes_subscriber_sinks.push(sink),
            }
        }

        let build_event_stream = if build_events {
            let (out, stream) = BuildEventStream::spawn_with_pipe(pid, bes_file_paths)?;
            cmd.arg("--build_event_publish_all_actions")
                .arg("--build_event_binary_file_upload_mode=fully_async")
                .arg("--build_event_binary_file")
                .arg(&out);
            Some(stream)
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

        // Build Event sinks for forwarding the build events.
        //
        // Generate ONE invocation_id and hand it to every sink so all backends
        // key this invocation under the same UUID. This lets us build a single
        // "View invocation" URL that works on whichever backend a user checks.
        // Without this, each sink would mint its own UUID and we'd have no way
        // to know which one corresponded to any particular viewer URL.
        let mut sink_handles: Vec<JoinHandle<()>> = vec![];
        let sink_invocation_id: Option<String> = if !bes_subscriber_sinks.is_empty() {
            let invocation_id = uuid::Uuid::new_v4().to_string();
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
            sink_handles.push(TracingEventStreamSink::spawn(
                rt,
                build_event_stream.as_ref().unwrap().subscribe(),
            ))
        }

        cmd.arg("--"); // separate flags from target patterns (not strictly necessary for build & test verbs but good form)
        cmd.args(targets);

        if debug_mode() {
            eprintln!("exec: {:?}", cmd.get_args());
        }

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

        drop(_enter);
        Ok(Self {
            child: RefCell::new(child),
            build_event_stream: RefCell::new(build_event_stream),
            workspace_event_stream: RefCell::new(workspace_event_stream),
            execlog_stream: RefCell::new(execlog_stream),
            sink_handles: RefCell::new(sink_handles),
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
    // Creates an iterable `BuildEventIterator` type.
    // Every call to this function will return a new iterator.
    // TODO: explain backpressure and build events sinks falling behind on poor network conditions.
    fn build_events<'v>(this: values::Value<'v>) -> anyhow::Result<BuildEventIterator> {
        let build = this.downcast_ref::<Build>().unwrap();
        let event_stream = build.build_event_stream.borrow();
        let event_stream = event_stream.as_ref().ok_or(anyhow::anyhow!(
            "call `ctx.bazel.build` with `build_events = true` in order to receive build events."
        ))?;

        Ok(BuildEventIterator::new(event_stream.subscribe()))
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

        let handles = build.sink_handles.take();
        for handle in handles {
            match handle.join() {
                Ok(_) => continue,
                Err(err) => anyhow::bail!("one of the sinks failed: {:#?}", err),
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
