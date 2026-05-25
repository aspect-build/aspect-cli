//! gRPC server scaffolding exposed to `.axl` via `@bazel//grpc.axl`.
//!
//! Surface:
//! - `grpc.Server(endpoint = ...)` — builder.
//! - `srv.add(<service>)` — attach a `Service` value from a proto module.
//! - `srv.serve()` — non-blocking; binds synchronously, returns a `ServerHandle`.
//! - `h.endpoint` — resolved endpoint string.
//! - `h.drain_and_quit(timeout = None)` — graceful shutdown.
//! - `grpc.Status(code = ..., message = ...)` — status value.
//! - `grpc.Code.<NAME>` — canonical codes, defined as a struct in `grpc.axl`.
//! - Handler arg: `rpc` (`abort`, `cancelled`).
//! - Handler arg for server-streaming: `stream` (`push`, `complete`, `cancelled`).

use std::fmt;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{NoSerialize, StarlarkValue, Value, starlark_value};

pub mod context;
pub mod server;
pub mod service;
pub mod services;
pub mod status;
pub mod stream;

#[cfg(test)]
mod tests;

pub use context::{GrpcRpcContext, RpcContextInner};
pub use server::{GrpcServer, GrpcServerHandle};
pub use service::{GrpcService, ServiceHandle};
pub use status::{GrpcStatus, code_from_i32, into_tonic_status};
pub use stream::{DynSender, GrpcStream};

/// Returned by `__builtins__.grpc()`. Exposes `Server` and `Status`
/// constructors for `@bazel//grpc.axl` to re-export.
#[derive(Debug, Clone, Copy, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuiltinsGrpc;

impl fmt::Display for BuiltinsGrpc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<BuiltinsGrpc>")
    }
}

starlark_simple_value!(BuiltinsGrpc);

#[starlark_value(type = "BuiltinsGrpc")]
impl<'v> StarlarkValue<'v> for BuiltinsGrpc {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(builtins_grpc_methods)
    }
}

#[starlark_module]
fn builtins_grpc_methods(registry: &mut MethodsBuilder) {
    /// `grpc.Server(endpoint = ...)` constructor.
    fn Server<'v>(
        this: Value<'v>,
        #[starlark(require = named)] endpoint: String,
    ) -> anyhow::Result<server::GrpcServer> {
        let _ = this;
        Ok(server::GrpcServer::new(endpoint))
    }

    /// `grpc.Status(code = ..., message = ...)` constructor.
    fn Status<'v>(
        this: Value<'v>,
        #[starlark(require = named)] code: i32,
        #[starlark(require = named)] message: String,
    ) -> anyhow::Result<status::GrpcStatus> {
        let _ = this;
        Ok(status::status_ctor(code, message))
    }
}

/// Hand the privileged `BuiltinsGrpc` namespace value back to a `@std`/`@bazel`
/// caller. Public `.axl` modules access the contents via
/// `load("@bazel//grpc.axl", "grpc")`.
pub fn make_builtins_grpc(eval: &mut Evaluator<'_, '_, '_>) -> anyhow::Result<BuiltinsGrpc> {
    super::builtins::check_std_context_pub(eval)?;
    Ok(BuiltinsGrpc)
}

#[starlark_module]
fn register_types(globals: &mut GlobalsBuilder) {
    const GrpcServer: starlark::values::starlark_value_as_type::StarlarkValueAsType<
        server::GrpcServer,
    > = starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
    const GrpcServerHandle: starlark::values::starlark_value_as_type::StarlarkValueAsType<
        server::GrpcServerHandle,
    > = starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
    const GrpcStatus: starlark::values::starlark_value_as_type::StarlarkValueAsType<
        status::GrpcStatus,
    > = starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
    const GrpcRpc: starlark::values::starlark_value_as_type::StarlarkValueAsType<
        context::GrpcRpcContext,
    > = starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
    const GrpcStream: starlark::values::starlark_value_as_type::StarlarkValueAsType<
        stream::GrpcStream,
    > = starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
    const GrpcService: starlark::values::starlark_value_as_type::StarlarkValueAsType<
        service::GrpcService,
    > = starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
}

pub fn register_globals(globals: &mut GlobalsBuilder) {
    register_types(globals);
    // NB: per-service `<Name>Service` constructors are NOT registered here —
    // they're namespaced under `_proto.<package>.*` by `engine/mod.rs` so
    // they're only visible from inside `@bazel//proto/*.axl` shims, which
    // re-export them under canonical names.
}
