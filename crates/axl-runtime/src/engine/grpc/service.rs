use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::{NoSerialize, StarlarkValue, starlark_value};
use tonic::service::Routes;

/// Implemented by macro-generated per-proto-service dispatch types.
///
/// The `service_server` proc-macro emits one impl per proto service. The
/// dispatch owns the Starlark callables for each RPC and any shared state,
/// and `install` clones the dispatch into a fresh tonic-generated
/// `*Server<T>` and registers it on the given `Routes`.
pub trait ServiceHandle: Send + Sync + std::fmt::Debug {
    /// Canonical fully-qualified proto service path,
    /// e.g. `"build.bazel.remote.execution.v2.ActionCache"`.
    fn proto_service_name(&self) -> &'static str;

    /// Register this service on the given tonic `Routes`. Implementations
    /// clone their internal dispatch state and call `routes.add_service(...)`.
    fn install(&self, routes: Routes) -> Routes;
}

/// Starlark wrapper around an `Arc<dyn ServiceHandle>`. Service constructors
/// (`v2.ActionCacheService(on_get_action_result = ...)`, etc.) return this
/// type so `srv.add(...)` can downcast to a known Starlark type and access
/// the underlying handle via `inner()`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct GrpcService {
    #[allocative(skip)]
    inner: Arc<dyn ServiceHandle>,
}

impl GrpcService {
    pub fn new(inner: Arc<dyn ServiceHandle>) -> Self {
        Self { inner }
    }
    pub fn inner(&self) -> &Arc<dyn ServiceHandle> {
        &self.inner
    }
}

impl fmt::Display for GrpcService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<grpc.Service {}>", self.inner.proto_service_name())
    }
}

starlark_simple_value!(GrpcService);

#[starlark_value(type = "grpc.Service")]
impl<'v> StarlarkValue<'v> for GrpcService {}
