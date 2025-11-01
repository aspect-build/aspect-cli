use starlark::{
    environment::GlobalsBuilder, starlark_module,
    values::starlark_value_as_type::StarlarkValueAsType,
};

mod bazel;
mod config;
mod config_context;
mod delivery;
mod globals;
mod http;
mod std;
mod template;
mod types;
mod wasm;

pub mod r#async;
pub mod context;
pub mod task;
pub mod task_arg;
pub mod task_args;
pub mod task_context;

#[starlark_module]
fn register_type_toplevels(builder: &mut GlobalsBuilder) {
    const http_response: StarlarkValueAsType<http::HttpResponse> = StarlarkValueAsType::new();
    const http: StarlarkValueAsType<http::Http> = StarlarkValueAsType::new();
    const task_arg: StarlarkValueAsType<task_arg::TaskArg> = StarlarkValueAsType::new();
    const task_args: StarlarkValueAsType<task_args::TaskArgs> = StarlarkValueAsType::new();
    const task_context: StarlarkValueAsType<task_context::TaskContext> = StarlarkValueAsType::new();
    const task: StarlarkValueAsType<task::Task> = StarlarkValueAsType::new();
    const template: StarlarkValueAsType<template::Template> = StarlarkValueAsType::new();
}

#[starlark_module]
fn wasm_toplevels(builder: &mut GlobalsBuilder) {
    const wasm_callable: StarlarkValueAsType<wasm::WasmCallable> = StarlarkValueAsType::new();
    const wasm_exports: StarlarkValueAsType<wasm::WasmExports> = StarlarkValueAsType::new();
    const wasm_instance: StarlarkValueAsType<wasm::WasmInstance> = StarlarkValueAsType::new();
    const wasm_memory: StarlarkValueAsType<wasm::WasmMemory> = StarlarkValueAsType::new();
    const wasm: StarlarkValueAsType<wasm::Wasm> = StarlarkValueAsType::new();
}

pub fn register_toplevels(builder: &mut GlobalsBuilder) {
    register_type_toplevels(builder);
    r#async::register_toplevels(builder);
    builder.namespace("bazel", bazel::register_toplevels);
    builder.namespace("wasm", wasm_toplevels);
    builder.namespace("std", std::register_toplevels);
    task_arg::register_toplevels(builder);
    task::register_toplevels(builder);
    globals::register_toplevels(builder);
    config::register_toplevels(builder);
    config_context::register_toplevels(builder);
}
