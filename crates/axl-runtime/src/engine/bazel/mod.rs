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
use starlark::values::dict::UnpackDictEntries;
use starlark::values::list::UnpackList;
use starlark::values::starlark_value;
use starlark::values::tuple::UnpackTuple;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

use crate::engine::store::AxlStore;
use axl_proto;

mod build;
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
    /// # Examples
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
        #[starlark(require = named, default = false)] execution_logs: bool,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            values::StringValue,
        >,
        #[starlark(require = named, default = UnpackList::default())] startup_flags: UnpackList<
            values::StringValue,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = match build_events {
            Either::Left(events) => (events, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let store = AxlStore::from_eval(eval)?;
        let build = build::Build::spawn(
            "build",
            targets.items.iter().map(|f| f.as_str().to_string()),
            build_events,
            execution_logs,
            workspace_events,
            flags.items.iter().map(|f| f.as_str().to_string()).collect(),
            startup_flags
                .items
                .iter()
                .map(|f| f.as_str().to_string())
                .collect(),
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
    /// # Examples
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
        #[starlark(require = named, default = false)] execution_logs: bool,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            values::StringValue,
        >,
        #[starlark(require = named, default = UnpackList::default())] startup_flags: UnpackList<
            values::StringValue,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = match build_events {
            Either::Left(events) => (events, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let store = AxlStore::from_eval(eval)?;
        let test = build::Build::spawn(
            "test",
            targets.items.iter().map(|f| f.as_str().to_string()),
            build_events,
            execution_logs,
            workspace_events,
            flags.items.iter().map(|f| f.as_str().to_string()).collect(),
            startup_flags
                .items
                .iter()
                .map(|f| f.as_str().to_string())
                .collect(),
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
    /// # Example
    ///
    /// ```starlark
    /// # Query dependencies of a target
    /// deps = ctx.bazel.query().targets("//myapp:main").deps()
    /// all_deps = deps.eval()
    ///
    /// # Chain multiple operations
    /// sources = ctx.bazel.query().targets("//myapp:main")
    ///     .deps()
    ///     .kind("source file")
    ///     .eval()
    /// ```
    fn query<'v>(#[allow(unused)] this: values::Value<'v>) -> starlark::Result<query::Query> {
        Ok(query::Query::new())
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
fn register_query_types(globals: &mut GlobalsBuilder) {
    const Query: StarlarkValueAsType<query::Query> = StarlarkValueAsType::new();
    const TargetSet: StarlarkValueAsType<query::TargetSet> = StarlarkValueAsType::new();
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const Bazel: StarlarkValueAsType<Bazel> = StarlarkValueAsType::new();
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
}
