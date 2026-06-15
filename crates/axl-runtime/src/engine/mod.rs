use axl_proto::build::bazel::remote::execution as remote_execution;
use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

mod aspect;
mod r#async;
mod hash;
mod http;
mod std;
mod template;
mod wasm;

pub mod feature;
pub mod grpc;
pub mod names;
pub mod r#trait;

pub mod arg;
pub mod arguments;
pub(crate) mod bazel;
pub(crate) mod builtins;
pub mod config_context;
pub mod feature_context;
pub mod feature_map;
pub mod store;
pub mod task;
pub mod task_context;
pub mod task_info;
pub mod task_map;
pub mod telemetry;
pub mod trait_map;
pub mod util;

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const ConfigContext: StarlarkValueAsType<config_context::ConfigContext> =
        StarlarkValueAsType::new();
    const FeatureContext: StarlarkValueAsType<feature_context::FeatureContext> =
        StarlarkValueAsType::new();
    const Http: StarlarkValueAsType<http::Http> = StarlarkValueAsType::new();
    const HttpResponse: StarlarkValueAsType<http::HttpResponse> = StarlarkValueAsType::new();
    const Task: StarlarkValueAsType<task::Task> = StarlarkValueAsType::new();
    const Arg: StarlarkValueAsType<arg::Arg> = StarlarkValueAsType::new();
    const Arguments: StarlarkValueAsType<arguments::Arguments> = StarlarkValueAsType::new();
    const TaskContext: StarlarkValueAsType<task_context::TaskContext> = StarlarkValueAsType::new();
    const TaskInfo: StarlarkValueAsType<task_info::TaskInfo> = StarlarkValueAsType::new();
    const Template: StarlarkValueAsType<template::Template> = StarlarkValueAsType::new();
    const Telemetry: StarlarkValueAsType<telemetry::Telemetry> = StarlarkValueAsType::new();
    const Exporters: StarlarkValueAsType<telemetry::Exporters> = StarlarkValueAsType::new();
    const ExporterSpec: StarlarkValueAsType<telemetry::ExporterSpec> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);

    r#async::register_globals(globals);
    r#trait::register_globals(globals);
    task::register_globals(globals);
    task_info::register_globals(globals);
    feature::register_globals(globals);
    grpc::register_globals(globals);

    globals.namespace("args", arg::register_globals);
    globals.namespace("bazel", bazel::register_globals);
    // Proto-derived types live under a hidden `_proto.<package>` global
    // namespace. `@bazel//proto/<path>.axl` shims re-export them under
    // canonical names (e.g. `v2`, `bytestream`). User code should use the
    // shims; the `_proto` prefix is an implementation detail.
    globals.namespace("_proto", |g| {
        g.namespace("v2", |g| {
            remote_execution::action_cache_client_module(g);
            remote_execution::capabilities_client_module(g);
            remote_execution::execution_client_module(g);
            remote_execution::content_addressable_storage_client_module(g);
            remote_execution::v2_toplevels(g);
            grpc::services::register_v2_services(g);
        });
        g.namespace("bytestream", |g| {
            // `*_toplevels` functions are emitted as siblings of the
            // mod they wrap (not children), so `bytestream_toplevels`
            // lives at `axl_proto::google::bytestream_toplevels`, not
            // `axl_proto::google::bytestream::bytestream_toplevels`.
            axl_proto::google::bytestream_toplevels(g);
            axl_proto::google::byte_stream_client_module(g);
            grpc::services::register_bytestream_services(g);
        });
        g.namespace("longrunning", axl_proto::google::longrunning_toplevels);
        g.namespace("rpc", axl_proto::google::rpc_toplevels);
        g.namespace("semver", axl_proto::build::bazel::semver_toplevels);
        g.namespace("remote_logging", axl_proto::remote_logging_toplevels);
    });
    globals.namespace("aspect", aspect::register_globals);
    globals.namespace("std", std::register_globals);
    globals.namespace("wasm", wasm::register_wasm_types);
    builtins::register_globals(globals);
}
