use std::collections::HashMap;

use allocative::Allocative;
use anyhow::Context;
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
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::tuple::UnpackTuple;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;

use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

use axl_proto;

use crate::engine::r#async::rt::AsyncRuntime;

mod build;
mod execlog_stream;
mod iterator;
mod query;
mod stream;
mod stream_sink;
mod stream_tracing;
mod stream_util;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel>")]
pub struct Bazel {}

starlark_simple_value!(Bazel);

#[starlark_value(type = "bazel")]
impl<'v> values::StarlarkValue<'v> for Bazel {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(bazel_methods)
    }
}

#[starlark_module]
pub(crate) fn bazel_methods(registry: &mut MethodsBuilder) {
    /// Build targets within AXL with ctx.bazel.build().
    /// The result is a `build`, which has `artifacts()` (TODO),
    /// `failures()` (TODO), and a `events()` functions that provide
    /// iterators to the artifacts, failures, events respectively.
    ///
    /// Running `ctx.bazel.build()` does not block the starlark thread. Explicitly
    /// call `.wait()` on the `build` type to wait until the build finishes.
    ///
    /// You can pass in a single target or target pattern to build.
    ///
    /// # Examples
    ///
    /// ```python
    /// def impl(ctx):
    ///     io = ctx.std.io
    ///     build = ctx.bazel.build(
    ///         "//target/to:build"
    ///         events = True,
    ///     )
    ///     for event in build.events():
    ///         if event.type == "progress":
    ///             io.stdout.write(event.payload.stdout)
    ///             io.stderr.write(event.payload.stderr)
    /// ```
    fn build<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(args)] targets: UnpackTuple<values::StringValue>,
        #[starlark(require = named, default = Either::Left(false))] events: Either<
            bool,
            UnpackList<build::BuildEventSink>,
        >,

        #[starlark(require = named, default = true)] execution_logs: bool,
        #[starlark(require = named, default = UnpackList::default())] bazel_flags: UnpackList<
            values::StringValue,
        >,
        #[starlark(require = named, default = NoneOr::None)] bazel_verb: NoneOr<
            values::StringValue,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<build::Build> {
        let events = match events {
            Either::Left(events) => (events, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let rt = AsyncRuntime::from_eval(eval)?;
        let build = build::Build::spawn(
            bazel_verb
                .into_option()
                .map_or("build", |verb| verb.as_str()),
            targets.items.iter().map(|f| f.as_str().to_string()),
            events,
            execution_logs,
            bazel_flags
                .items
                .iter()
                .map(|f| f.as_str().to_string())
                .collect(),
            rt,
        )
        .context("failed to spawn build tool")?;
        Ok(build)
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
fn sink_toplevels(builder: &mut GlobalsBuilder) {
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
fn build_toplevels(builder: &mut GlobalsBuilder) {
    const build_events_iterator: StarlarkValueAsType<iterator::BuildEventIterator> =
        StarlarkValueAsType::new();
    const execution_log_iterator: StarlarkValueAsType<iterator::ExecutionLogIterator> =
        StarlarkValueAsType::new();
    const build: StarlarkValueAsType<build::Build> = StarlarkValueAsType::new();
    const build_status: StarlarkValueAsType<build::BuildStatus> = StarlarkValueAsType::new();
}

#[starlark_module]
fn query_toplevels(builder: &mut GlobalsBuilder) {
    const query: StarlarkValueAsType<query::Query> = StarlarkValueAsType::new();
    const target_set: StarlarkValueAsType<query::TargetSet> = StarlarkValueAsType::new();
}

#[starlark_module]
fn toplevels(builder: &mut GlobalsBuilder) {
    const bazel: StarlarkValueAsType<Bazel> = StarlarkValueAsType::new();
}

pub fn register_toplevels(builder: &mut GlobalsBuilder) {
    toplevels(builder);
    builder.namespace("query", |builder| {
        query_toplevels(builder);
        axl_proto::blaze_query_toplevels(builder);
    });

    builder.namespace("build", |builder| {
        build_toplevels(builder);
        builder.namespace("build_event", axl_proto::build_event_stream_toplevels);
        builder.namespace("execution_log", axl_proto::tools_protos_toplevels);
    });

    builder.namespace("build_events", |builder| {
        sink_toplevels(builder);
    });
}
