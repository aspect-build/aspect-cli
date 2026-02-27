use std::collections::HashMap;

use allocative::Allocative;
use derive_more::Display;
use either::Either;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::tuple::UnpackTuple;
use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

use crate::engine::store::AxlStore;
use axl_proto;

mod build;
mod execlog_sink;
mod health_check;
mod iter;
mod query;
mod stream;
mod stream_sink;
mod stream_tracing;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.Bazel>")]
pub struct Bazel {}

starlark_simple_value!(Bazel);

#[starlark_value(type = "bazel.Bazel")]
impl<'v> values::StarlarkValue<'v> for Bazel {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(bazel_methods)
    }
}

#[starlark_module]
pub(crate) fn bazel_methods(registry: &mut MethodsBuilder) {
    /// Build targets Bazel within AXL with ctx.bazel.build().
    /// The result is a `Build` object, which has `artifacts()` (TODO),
    /// `failures()` (TODO), and a `events()` functions that provide
    /// iterators to the artifacts, failures, events respectively.
    ///
    /// Running `ctx.bazel.build()` does not block the Starlark thread. Explicitly
    /// call `.wait()` on the `Build` object to wait until the invocation finishes.
    ///
    /// You can pass in a single target or target pattern to build.
    ///
    /// # Arguments
    /// * `execution_log`: Enable Bazel execution log collection. Pass `True` to
    ///   enable the in-memory decoded iterator (accessible via `build.execution_logs()`),
    ///   or pass a list of sinks such as `[execution_log.compact_file(path = "out.binpb.zst")]`
    ///   to write the log to one or more files. Sinks and the iterator can be combined:
    ///   passing a list of sinks still allows calling `build.execution_logs()` to iterate
    ///   entries in-process.
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _fancy_build_impl(ctx):
    ///     io = ctx.std.io
    ///     build = ctx.bazel.build(
    ///         "//target/to:build"
    ///         build_events = True,
    ///     )
    ///     for event in build.build_events():
    ///         if event.type == "progress":
    ///             io.stdout.write(event.payload.stdout)
    ///             io.stderr.write(event.payload.stderr)
    /// ```
    fn build<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(args)] targets: UnpackTuple<values::StringValue>,
        #[starlark(require = named, default = Either::Left(false))] build_events: Either<
            bool,
            UnpackList<build::BuildEventSink>,
        >,
        #[starlark(require = named, default = false)] workspace_events: bool,
        #[starlark(require = named, default = Either::Left(false))] execution_log: Either<
            bool,
            UnpackList<execlog_sink::ExecLogSink>,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            values::StringValue,
        >,
        #[starlark(require = named, default = UnpackList::default())] startup_flags: UnpackList<
            values::StringValue,
        >,
        #[starlark(require = named, default = false)] inherit_stdout: bool,
        #[starlark(require = named, default = true)] inherit_stderr: bool,
        #[starlark(require = named, default = NoneOr::None)] current_dir: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = match build_events {
            Either::Left(events) => (events, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let execution_log = match execution_log {
            Either::Left(b) => (b, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let store = AxlStore::from_eval(eval)?;
        let build = build::Build::spawn(
            "build",
            targets.items.iter().map(|f| f.as_str().to_string()),
            build_events,
            execution_log,
            workspace_events,
            flags.items.iter().map(|f| f.as_str().to_string()).collect(),
            startup_flags
                .items
                .iter()
                .map(|f| f.as_str().to_string())
                .collect(),
            inherit_stdout,
            inherit_stderr,
            current_dir.into_option(),
            store.rt,
        )?;
        Ok(build)
    }

    /// Build & test Bazel targets within AXL with ctx.bazel.test().
    /// The result is a `Build` object, which has `artifacts()` (TODO),
    /// `failures()` (TODO), and a `events()` functions that provide
    /// iterators to the artifacts, failures, events respectively.
    ///
    /// Running `ctx.bazel.test()` does not block the Starlark thread. Explicitly
    /// call `.wait()` on the `Build` object to wait until the invocation finishes.
    ///
    /// You can pass in a single target or target pattern to test.
    ///
    /// # Arguments
    /// * `execution_log`: Enable Bazel execution log collection. Pass `True` to
    ///   enable the in-memory decoded iterator (accessible via `build.execution_logs()`),
    ///   or pass a list of sinks such as `[execution_log.compact_file(path = "out.binpb.zst")]`
    ///   to write the log to one or more files. Sinks and the iterator can be combined:
    ///   passing a list of sinks still allows calling `build.execution_logs()` to iterate
    ///   entries in-process.
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _fancy_test_impl(ctx):
    ///     io = ctx.std.io
    ///     test = ctx.bazel.test(
    ///         "//target/to:test"
    ///         build_events = True,
    ///     )
    ///     for event in test.build_events():
    ///         if event.type == "progress":
    ///             io.stdout.write(event.payload.stdout)
    ///             io.stderr.write(event.payload.stderr)
    /// ```
    fn test<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(args)] targets: UnpackTuple<values::StringValue>,
        #[starlark(require = named, default = Either::Left(false))] build_events: Either<
            bool,
            UnpackList<build::BuildEventSink>,
        >,
        #[starlark(require = named, default = false)] workspace_events: bool,
        #[starlark(require = named, default = Either::Left(false))] execution_log: Either<
            bool,
            UnpackList<execlog_sink::ExecLogSink>,
        >,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            values::StringValue,
        >,
        #[starlark(require = named, default = UnpackList::default())] startup_flags: UnpackList<
            values::StringValue,
        >,
        #[starlark(require = named, default = false)] inherit_stdout: bool,
        #[starlark(require = named, default = true)] inherit_stderr: bool,
        #[starlark(require = named, default = NoneOr::None)] current_dir: NoneOr<String>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = match build_events {
            Either::Left(events) => (events, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let execution_log = match execution_log {
            Either::Left(b) => (b, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let store = AxlStore::from_eval(eval)?;
        let test = build::Build::spawn(
            "test",
            targets.items.iter().map(|f| f.as_str().to_string()),
            build_events,
            execution_log,
            workspace_events,
            flags.items.iter().map(|f| f.as_str().to_string()).collect(),
            startup_flags
                .items
                .iter()
                .map(|f| f.as_str().to_string())
                .collect(),
            inherit_stdout,
            inherit_stderr,
            current_dir.into_option(),
            store.rt,
        )?;
        Ok(test)
    }

    /// The query system provides a programmatic interface for analyzing build dependencies
    /// and target relationships. Queries are constructed using a chain API and are lazily
    /// evaluated only when `.eval()` is explicitly called.
    ///
    /// The entry point is `ctx.bazel.query()`, which returns a `query` for creating initial
    /// query expressions. Most operations operate on `query` objects, which represent
    /// sets of targets that can be filtered, transformed, and combined.
    ///
    /// **Example**
    ///
    /// ```starlark
    /// **Query** dependencies of a target
    /// deps = ctx.bazel.query().targets("//myapp:main").deps()
    /// all_deps = deps.eval()
    ///
    /// **Chain** multiple operations
    /// sources = ctx.bazel.query().targets("//myapp:main")
    ///     .deps()
    ///     .kind("source file")
    ///     .eval()
    /// ```
    fn query<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<query::Query> {
        Ok(query::Query::new())
    }

    /// Probe the Bazel server to determine whether it is responsive.
    ///
    /// Runs `bazel --noblock_for_lock info server_pid`. If the server is
    /// unresponsive, attempts recovery by killing the server process and
    /// re-checking.
    ///
    /// Returns a `HealthCheckResult` with `.success`, `.healthy`, `.message`,
    /// and `.exit_code` attributes.
    ///
    /// **Examples**
    ///
    /// ```python
    /// def _health_probe_impl(ctx):
    ///     result = ctx.bazel.health_check()
    ///     if not result.healthy:
    ///         fail("Bazel server is unhealthy")
    /// ```
    fn health_check<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] output_base: NoneOr<String>,
    ) -> anyhow::Result<health_check::HealthCheckResult> {
        Ok(health_check::run(output_base.into_option().as_deref()))
    }
}

#[starlark_module]
fn register_build_events(globals: &mut GlobalsBuilder) {
    #[starlark(as_type = build::BuildEventSink)]
    fn grpc(
        #[starlark(require = named)] uri: String,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        metadata: UnpackDictEntries<String, String>,
    ) -> starlark::Result<build::BuildEventSink> {
        // TODO: validate endpoint
        Ok(build::BuildEventSink::Grpc {
            uri: uri.replace("grpcs://", "https://"),
            metadata: HashMap::from_iter(metadata.entries),
        })
    }

    fn file(#[starlark(require = named)] path: String) -> starlark::Result<build::BuildEventSink> {
        Ok(build::BuildEventSink::File { path })
    }
}

#[starlark_module]
fn register_execlog_sinks(globals: &mut GlobalsBuilder) {
    #[starlark(as_type = execlog_sink::ExecLogSink)]
    fn file(
        #[starlark(require = named)] path: String,
    ) -> starlark::Result<execlog_sink::ExecLogSink> {
        Ok(execlog_sink::ExecLogSink::File { path })
    }

    fn compact_file(
        #[starlark(require = named)] path: String,
    ) -> starlark::Result<execlog_sink::ExecLogSink> {
        Ok(execlog_sink::ExecLogSink::CompactFile { path })
    }
}

#[starlark_module]
fn register_build_types(globals: &mut GlobalsBuilder) {
    const Build: StarlarkValueAsType<build::Build> = StarlarkValueAsType::new();
    const BuildEventIterator: StarlarkValueAsType<iter::BuildEventIterator> =
        StarlarkValueAsType::new();
    const BuildEventSink: StarlarkValueAsType<build::BuildEventSink> = StarlarkValueAsType::new();
    const BuildStatus: StarlarkValueAsType<build::BuildStatus> = StarlarkValueAsType::new();
    const ExecutionLogIterator: StarlarkValueAsType<iter::ExecutionLogIterator> =
        StarlarkValueAsType::new();
    const WorkspaceEventIterator: StarlarkValueAsType<iter::WorkspaceEventIterator> =
        StarlarkValueAsType::new();
}

#[starlark_module]
fn register_execlog_types(globals: &mut GlobalsBuilder) {
    const ExecLogSink: StarlarkValueAsType<execlog_sink::ExecLogSink> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_query_types(globals: &mut GlobalsBuilder) {
    const Query: StarlarkValueAsType<query::Query> = StarlarkValueAsType::new();
    const TargetSet: StarlarkValueAsType<query::TargetSet> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Bazel: StarlarkValueAsType<Bazel> = StarlarkValueAsType::new();
    const HealthCheckResult: StarlarkValueAsType<health_check::HealthCheckResult> =
        StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);

    globals.namespace("query", |globals| {
        register_query_types(globals);
        axl_proto::blaze_query_toplevels(globals);
    });

    globals.namespace("build", |globals| {
        register_build_types(globals);
        globals.namespace("build_event", axl_proto::build_event_stream_toplevels);
        globals.namespace("execution_log", axl_proto::tools_protos_toplevels);
        globals.namespace("workspace_event", axl_proto::workspace_log_toplevels);
    });

    globals.namespace("build_events", |globals| {
        register_build_events(globals);
    });

    globals.namespace("execution_log", |globals| {
        register_execlog_types(globals);
        register_execlog_sinks(globals);
    });
}
