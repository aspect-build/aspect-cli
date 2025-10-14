use std::cell::RefCell;
use std::collections::HashMap;
use std::env::temp_dir;
use std::fs;
use std::fs::File;

use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::thread::JoinHandle;

use allocative::Allocative;
use anyhow::Context;
use derive_more::Display;
use nix::sys::stat::Mode;
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

use super::iterator::BuildEventIterator;
use super::iterator::ExecutionLogIterator;
use super::stream::EventStream;
use super::stream_sink::GrpcEventStreamSink;
use super::stream_tracing::TracingEventStreamSink;

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<build_status>")]
pub struct BuildStatus {
    success: bool,
    code: Option<i32>,
}

impl<'v> AllocValue<'v> for BuildStatus {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "build_status")]
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
#[display("<build_event_sink>")]
pub enum BuildEventSink {
    Grpc {
        uri: String,
        metadata: HashMap<String, String>,
    },
}

starlark_simple_value!(BuildEventSink);

#[starlark_value(type = "build_event_sink")]
impl<'v> values::StarlarkValue<'v> for BuildEventSink {}

impl<'v> UnpackValue<'v> for BuildEventSink {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let value = value.downcast_ref_err::<BuildEventSink>()?;
        Ok(Some(value.clone()))
    }
}

impl BuildEventSink {
    fn spawn(&self, rt: AsyncRuntime, stream: &EventStream) -> JoinHandle<()> {
        match self {
            BuildEventSink::Grpc { uri, metadata } => {
                GrpcEventStreamSink::spawn(rt, stream.receiver(), uri.clone(), metadata.clone())
            }
        }
    }
}

#[derive(Debug, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<build>")]
pub struct Build {
    #[allocative(skip)]
    event_stream: RefCell<Option<EventStream>>,
    execlog_out: Option<PathBuf>,

    #[allocative(skip)]
    sink_handles: RefCell<Vec<JoinHandle<()>>>,

    #[allocative(skip)]
    child: RefCell<Child>,

    #[allocative(skip)]
    span: RefCell<tracing::span::EnteredSpan>,
}

impl Build {
    // TODO: this should return a thiserror::Error
    pub fn spawn(
        verb: &str,
        targets: impl IntoIterator<Item = String>,
        (events, sinks): (bool, Vec<BuildEventSink>),
        execution_logs: bool,
        flags: Vec<String>,
        rt: AsyncRuntime,
    ) -> Result<Build, std::io::Error> {
        let span = tracing::info_span!(
            "ctx.bazel.build",
            events = events,
            execution_logs = execution_logs,
            flags = ?flags
        )
        .entered();

        let mut cmd = Command::new("bazel");
        cmd.arg(verb);

        let event_stream = if events {
            let out = temp_dir().join("build_event_out.bin");
            let _ = fs::remove_file(&out);
            match nix::unistd::mkfifo(&out, Mode::S_IRWXO | Mode::S_IRWXU | Mode::S_IRWXG) {
                Ok(_) => {}
                Err(_) => todo!("failed to create pipe, implement the fallback mechanism"),
            };
            cmd.arg("--build_event_publish_all_actions")
                .arg("--build_event_binary_file_upload_mode=fully_async")
                .arg("--build_event_binary_file")
                .arg(&out);
            Some(EventStream::spawn(out))
        } else {
            None
        };

        let mut sink_handles: Vec<JoinHandle<()>> = vec![];

        for sink in sinks {
            let handle = sink.spawn(rt.clone(), event_stream.as_ref().unwrap());
            sink_handles.push(handle);
        }

        if events {
            sink_handles.push(TracingEventStreamSink::spawn(
                rt,
                event_stream.as_ref().unwrap().receiver(),
            ))
        }

        let execlog_out = if execution_logs {
            let out = temp_dir().join("compact_exec_log.bin");
            let _ = fs::remove_file(&out);
            // TODO: figure out async buffered reading in a separate read thread.
            //
            // match nix::unistd::mkfifo(&out, Mode::S_IRWXO | Mode::S_IRWXU | Mode::S_IRWXG) {
            //     Ok(_) => {}
            //     Err(_) => todo!("failed to create pipe, implement the fallback mechanism"),
            // };
            cmd.arg("--execution_log_compact_file").arg(&out);
            Some(out)
        } else {
            None
        };

        cmd.args(targets);
        cmd.args(flags);
        cmd.stderr(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stdin(Stdio::null());

        let child = cmd.spawn()?;

        Ok(Self {
            child: RefCell::new(child),
            event_stream: RefCell::new(event_stream),
            sink_handles: RefCell::new(sink_handles),
            execlog_out: execlog_out,
            span: RefCell::new(span),
        })
    }
}

impl<'v> AllocValue<'v> for Build {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "build")]
impl<'v> values::StarlarkValue<'v> for Build {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(build_methods)
    }
}

#[starlark_module]
pub(crate) fn build_methods(registry: &mut MethodsBuilder) {
    fn events<'v>(this: values::Value<'v>) -> anyhow::Result<BuildEventIterator> {
        let build = this.downcast_ref::<Build>().unwrap();
        let event_stream = build.event_stream.borrow();
        let event_stream = event_stream.as_ref().ok_or(anyhow::anyhow!(
            "call `ctx.bazel.build` with `events = true` in order to receive build events."
        ))?;

        Ok(BuildEventIterator::new(event_stream.receiver()))
    }

    fn execution_logs<'v>(this: values::Value<'v>) -> anyhow::Result<ExecutionLogIterator> {
        let build = this.downcast_ref::<Build>().unwrap();
        let stream_out = build.execlog_out.as_ref().ok_or(anyhow::anyhow!(
            "call `ctx.bazel.build` with `execution_logs = true` in order to receive execution logs."
        ))?;
        // wait until bes file is created.
        while !stream_out.exists() {}
        let file = File::open(stream_out).context("failed to read execution logs")?;
        Ok(ExecutionLogIterator::new(file)?)
    }

    fn wait<'v>(this: values::Value<'v>) -> anyhow::Result<BuildStatus> {
        let build = this.downcast_ref_err::<Build>()?;
        let result = build.child.borrow_mut().wait()?;

        // TODO: consider adding a wait_events() method for granular control.
        // Wait for BES sinks to complete thier tasks.
        let event_stream = build.event_stream.take();
        if let Some(event_stream) = event_stream {
            match event_stream.join() {
                Ok(_) => {}
                // TODO: tell the user which one and why
                Err(err) => anyhow::bail!("event stream thread failed: {}", err),
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
        //
        let span = build.span.replace(tracing::trace_span!("build").entered());
        span.exit();
        Ok(BuildStatus {
            success: result.success(),
            code: result.code(),
        })
    }
}
