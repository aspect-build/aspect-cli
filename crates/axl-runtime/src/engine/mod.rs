use axl_proto::build::bazel::remote::execution as remote_execution;
use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

mod bazel;
mod globals;
mod http;
mod std;
mod template;
mod types;
mod wasm;

pub mod r#async;
pub mod config;
pub mod store;
pub mod task;
pub mod task_arg;
pub mod task_args;
pub mod task_context;

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const ConfigContext: StarlarkValueAsType<config::ConfigContext> = StarlarkValueAsType::new();
    const Http: StarlarkValueAsType<http::Http> = StarlarkValueAsType::new();
    const HttpResponse: StarlarkValueAsType<http::HttpResponse> = StarlarkValueAsType::new();
    const Task: StarlarkValueAsType<task::Task> = StarlarkValueAsType::new();
    const TaskArg: StarlarkValueAsType<task_arg::TaskArg> = StarlarkValueAsType::new();
    const TaskArgs: StarlarkValueAsType<task_args::TaskArgs> = StarlarkValueAsType::new();
    const TaskContext: StarlarkValueAsType<task_context::TaskContext> = StarlarkValueAsType::new();
    const Template: StarlarkValueAsType<template::Template> = StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);

    globals::register_globals(globals);
    r#async::register_globals(globals);
    task::register_globals(globals);

    globals.namespace("args", task_arg::register_globals);
    globals.namespace("bazel", bazel::register_globals);
    globals.namespace("remote", |g| {
        g.namespace("execution", |g| {
            remote_execution::action_cache_service(g);
            remote_execution::v2_toplevels(g);
        });
    });
    globals.namespace("std", std::register_globals);
    globals.namespace("wasm", wasm::register_wasm_types);
}
