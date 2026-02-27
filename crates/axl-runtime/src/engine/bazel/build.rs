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

    #[allocative(skip)]
    child: Rc<RefCell<Child>>,

    #[allocative(skip)]
    span: RefCell<tracing::span::EnteredSpan>,
}

impl Build {
    pub fn server_info() -> io::Result<(u32, semver::Version)> {
        let mut cmd = Command::new("bazel");
        cmd.arg("info");
        cmd.arg("server_pid");
        cmd.arg("release");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        cmd.stdin(Stdio::null());
        let c = cmd.spawn()?.wait_with_output()?;
        if !c.status.success() {
            return Err(io::Error::other(anyhow!(
                "failed to determine Bazel server info"
            )));
        }

        // When bazel info is called with multiple keys it emits "key: value" lines.
        let stdout = String::from_utf8_lossy(&c.stdout);
        let mut pid: Option<u32> = None;
        let mut version: Option<semver::Version> = None;
        for line in stdout.lines() {
            if let Some((key, value)) = line.split_once(": ") {
                match key.trim() {
                    "server_pid" => {
                        pid = value.trim().parse::<u32>().ok();
                    }
                    "release" => {
                        // Value is like "release 9.0.0" or "release 9.0.0-rc1"
                        let ver_str = value.trim().trim_start_matches("release ").trim();
                        // Strip pre-release suffix: "9.0.0-rc1" -> "9.0.0"
                        let ver_str = ver_str.split('-').next().unwrap_or(ver_str);
                        version = semver::Version::parse(ver_str)
                            .map_err(|e| {
                                io::Error::other(anyhow!(
                                    "failed to parse Bazel version '{}': {}",
                                    ver_str,
                                    e
                                ))
                            })
                            .ok();
                    }
                    _ => {}
                }
            }
        }

        let pid =
            pid.ok_or_else(|| io::Error::other(anyhow!("bazel info did not return server_pid")))?;
        let version = version.ok_or_else(|| {
            io::Error::other(anyhow!(
                "bazel info did not return a parseable release version"
            ))
        })?;

        Ok((pid, version))
    }

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
        let (pid, _) = Self::server_info()?;

        let span = tracing::info_span!(
            "ctx.bazel.build",
            build_events = build_events,
            workspace_events = workspace_events,
            execution_logs = execution_logs,
            flags = ?flags
        )
        .entered();

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

        // Build Event sinks for forwarding the build events
        let mut sink_handles: Vec<JoinHandle<()>> = vec![];
        for sink in bes_subscriber_sinks {
            let handle = sink.spawn(rt.clone(), build_event_stream.as_ref().unwrap());
            sink_handles.push(handle);
        }

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
                build_event_stream.as_ref().unwrap().subscribe_realtime(),
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
        let build = this.downcast_ref_err::<Build>()?;

        let result = build.child.borrow_mut().wait()?;

        // TODO: consider adding a wait_events() method for granular control.

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

        let span = build.span.replace(tracing::trace_span!("build").entered());
        span.exit();

        Ok(BuildStatus {
            success: result.success(),
            code: result.code(),
        })
    }
}
