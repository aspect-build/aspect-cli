use std::collections::HashMap;

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

use crate::engine::r#async::rt::AsyncRuntime;
use allocative::Allocative;
use anyhow::Context;
use axl_proto;
use derive_more::Display;

mod build;
mod iter;
mod query;
mod stream;
mod stream_sink;
mod stream_tracing;

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
        #[starlark(require = named, default = NoneOr::None)] command: NoneOr<values::StringValue>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<build::Build> {
        let build_events = match build_events {
            Either::Left(events) => (events, vec![]),
            Either::Right(sinks) => (true, sinks.items),
        };
        let rt = AsyncRuntime::from_eval(eval)?;
        let command = command.into_option().map_or("build", |verb| verb.as_str());
        if command != "build" && command != "test" {
            anyhow::bail!("command can only be set to `build` or `test`.")
        }
        let build = build::Build::spawn(
            command,
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
    const build_event_iterator: StarlarkValueAsType<iter::BuildEventIterator> =
        StarlarkValueAsType::new();
    const execution_log_iterator: StarlarkValueAsType<iter::ExecutionLogIterator> =
        StarlarkValueAsType::new();
    const workspace_event_iterator: StarlarkValueAsType<iter::WorkspaceEventIterator> =
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
        builder.namespace("workspace_event", axl_proto::workspace_log_toplevels);
    });

    builder.namespace("build_events", |builder| {
        sink_toplevels(builder);
    });
}
