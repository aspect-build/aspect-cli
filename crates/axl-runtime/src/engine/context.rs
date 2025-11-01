use std::{collections::HashMap, path::PathBuf};

use starlark_derive::ProvidesStaticType;

use super::r#async::rt::AsyncRuntime;

/// A context object which we pass to the Starlark interpreter which allows us
/// to track state (tools, cache, ...) around the Starlark evaluation.
///
/// This is at least related to the config struct that we want to be able to
/// manipulate in .aspect/config.axl:config middleware.
#[derive(Debug, ProvidesStaticType, Clone)]
pub struct AxlContext {
    // TODO: Need a config value for the current global config
    //
    // TODO: Need a workspace/repo root value
    //
    // TODO: Need a user config value
    //
    // TODO: Need a user cache value
    //
    // TODO: Need a Bazel output_base / cache root value
    //
    // TODO: Want some sort of config_builder ctx type we can use which at an
    // absolute minimum relates to this type if not directly constructing it.
    pub runtime: AsyncRuntime,
    pub tools: HashMap<String, PathBuf>,
}
