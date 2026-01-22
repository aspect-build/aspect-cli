use std::cell::RefCell;
use std::collections::HashMap;
use std::env::var;
use std::io;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::rc::Rc;
use std::thread::JoinHandle;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::ValueLike;

use crate::engine::r#async::rt::AsyncRuntime;

use super::helpers::format_bazel_command;
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
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
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
}

starlark_simple_value!(BuildEventSink);

#[starlark_value(type = "bazel.build.BuildEventSink")]
impl<'v> values::StarlarkValue<'v> for BuildEventSink {}

impl<'v> UnpackValue<'v> for BuildEventSink {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let value = value.downcast_ref_err::<BuildEventSink>()?;
        Ok(Some(value.clone()))
    }
}

impl BuildEventSink {
    fn spawn(&self, rt: AsyncRuntime, stream: &BuildEventStream) -> JoinHandle<()> {
        match self {
            BuildEventSink::Grpc { uri, metadata } => {
                // Use subscribe_realtime() since sinks subscribe at stream creation
                // and don't need history replay.
                GrpcEventStreamSink::spawn(
                    rt,
                    stream.subscribe_realtime(),
                    uri.clone(),
                    metadata.clone(),
                )
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

    #[allocative(skip)]
    child: Rc<RefCell<Child>>,

    #[allocative(skip)]
    span: RefCell<tracing::span::EnteredSpan>,
}

impl Build {
    pub fn pid() -> io::Result<u32> {
        let mut cmd = Command::new("bazel");
        cmd.arg("info");
        cmd.arg("server_pid");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        cmd.stdin(Stdio::null());
        let c = cmd.spawn()?.wait_with_output()?;
        if !c.status.success() {
            return Err(io::Error::other(anyhow!("failed to determine Bazel pid")));
        }
        let bytes: [u8; 4] = c.stdout[0..4].try_into().unwrap();
        Ok(u32::from_be_bytes(bytes))
    }

    // TODO: this should return a thiserror::Error
    pub fn spawn(
        verb: &str,
        targets: impl IntoIterator<Item = String>,
        (build_events, sinks): (bool, Vec<BuildEventSink>),
        execution_logs: bool,
        workspace_events: bool,
        flags: Vec<String>,
        startup_flags: Vec<String>,
        inherit_stdout: bool,
        inherit_stderr: bool,
        current_dir: Option<String>,
        rt: AsyncRuntime,
    ) -> Result<Build, std::io::Error> {
        let pid = Self::pid()?;

        let span = tracing::info_span!(
            "ctx.bazel.build",
            build_events = build_events,
            workspace_events = workspace_events,
            execution_logs = execution_logs,
            flags = ?flags
        )
        .entered();

        let targets: Vec<String> = targets.into_iter().collect();

        if debug_mode() {
            eprintln!(
                "running {}",
                format_bazel_command(&startup_flags, verb, &flags, &targets)
            );
        }

        let mut cmd = Command::new("bazel");
        cmd.args(startup_flags);
        cmd.arg(verb);

        if let Some(current_dir) = current_dir {
            cmd.current_dir(current_dir);
        }

        let build_event_stream = if build_events {
            let (out, stream) = BuildEventStream::spawn_with_pipe(pid)?;
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

        let execlog_stream = if execution_logs {
            let (out, stream) = ExecLogStream::spawn_with_pipe(pid)?;
            cmd.arg("--execution_log_compact_file").arg(&out);
            Some(stream)
        } else {
            None
        };

        // Build Event sinks for forwarding the build events
        let mut sink_handles: Vec<JoinHandle<()>> = vec![];
        for sink in sinks {
            let handle = sink.spawn(rt.clone(), build_event_stream.as_ref().unwrap());
            sink_handles.push(handle);
        }
        if build_events {
            // Use subscribe_realtime() since this subscribes at stream creation
            // and doesn't need history replay.
            sink_handles.push(TracingEventStreamSink::spawn(
                rt,
                build_event_stream.as_ref().unwrap().subscribe_realtime(),
            ))
        }

        cmd.args(flags);
        cmd.arg("--"); // separate flags from target patterns (not strictly necessary for build & test verbs but good form)
        cmd.args(targets);

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

        let child = cmd.spawn()?;

        Ok(Self {
            child: Rc::new(RefCell::new(child)),
            build_event_stream: RefCell::new(build_event_stream),
            workspace_event_stream: RefCell::new(workspace_event_stream),
            execlog_stream: RefCell::new(execlog_stream),
            sink_handles: RefCell::new(sink_handles),
            span: RefCell::new(span),
        })
    }
}

impl<'v> AllocValue<'v> for Build {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.build.Build")]
impl<'v> values::StarlarkValue<'v> for Build {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(build_methods)
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
            "call `ctx.bazel.build` with `execution_logs = true` in order to receive execution log events."
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
        let build = this.downcast_ref_err::<Build>()?;
        let status = build.child.borrow_mut().try_wait()?;
        Ok(match status {
            Some(status) => NoneOr::Other(BuildStatus {
                success: status.success(),
                code: status.code(),
            }),
            None => NoneOr::None,
        })
    }

    fn wait<'v>(this: values::Value<'v>) -> anyhow::Result<BuildStatus> {
        let build = this.downcast_ref_err::<Build>()?;
        let result = build.child.borrow_mut().wait()?;

        // TODO: consider adding a wait_events() method for granular control.

        // Wait for BES stream to complete.
        // Note: We don't take() the stream here so that build_events() can still
        // be called after wait() to get historical events.
        if let Some(ref mut event_stream) = *build.build_event_stream.borrow_mut() {
            match event_stream.join() {
                Ok(_) => {}
                // TODO: tell the user which one and why
                Err(err) => anyhow::bail!("build event stream thread error: {}", err),
            }
        }

        // Wait for Workspace event stream to complete.
        let workspace_event_stream = build.workspace_event_stream.take();
        if let Some(workspace_event_stream) = workspace_event_stream {
            match workspace_event_stream.join() {
                Ok(_) => {}
                // TODO: tell the user which one and why
                Err(err) => anyhow::bail!("workspace event stream thread error: {}", err),
            }
        };

        // Wait for Execlog stream to complete.
        let execlog_stream = build.execlog_stream.take();
        if let Some(execlog_stream) = execlog_stream {
            match execlog_stream.join() {
                Ok(_) => {}
                // TODO: tell the user which one and why
                Err(err) => anyhow::bail!("execlog stream thread error: {}", err),
            }
        };

        let handles = build.sink_handles.take();
        for handle in handles {
            match handle.join() {
                Ok(_) => continue,
                // TODO: tell the user which one and why
                Err(err) => anyhow::bail!("one of the sinks failed: {:#?}", err),
            }
        }
        // BES ends here

        let span = build.span.replace(tracing::trace_span!("build").entered());
        span.exit();
        Ok(BuildStatus {
            success: result.success(),
            code: result.code(),
        })
    }
}
