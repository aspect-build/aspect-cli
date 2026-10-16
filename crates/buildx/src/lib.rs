//! buildx: Bazel's engine as a library inside this process, driven by a
//! Starlark script.
//!
//! The release's server jar is loaded into a JVM that this process creates
//! through the JNI Invocation API. No client, no gRPC server, no rc files and
//! no commands: the runtime is constructed the way `BlazeRuntime.main` would,
//! and loading and queries are driven on its `SkyframeExecutor` directly, with
//! results left on the heap and handed to the script as live `Ref`s.
//! Bazel classes are reached through `bindings`, which `blazon` generates per
//! Bazel version from `bindings.toml` at build time. `starlark` is the script surface,
//! shaped after Buck2's BXL.

pub mod bindings;
pub mod engine;
pub mod output_base;
pub mod starlark;
pub mod workspace;

pub use blazon::release::{Install, Release};
pub use engine::{Engine, EngineOptions, Live};
