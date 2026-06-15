use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{
    Heap, NoSerialize, StarlarkValue, Value, ValueLike, dict::AllocDict, none::NoneType,
    starlark_value,
};

use super::status::{code_from_i32, into_tonic_status};

/// Per-RPC framework object passed to every handler as `rpc`.
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct GrpcRpcContext {
    #[allocative(skip)]
    inner: Arc<RpcContextInner>,
}

#[derive(Debug)]
pub struct RpcContextInner {
    /// Set when the handler calls `rpc.abort(...)`. The macro-generated
    /// dispatch picks this up after the handler returns and converts to a
    /// tonic `Status`.
    abort: Mutex<Option<tonic::Status>>,

    /// Polled by handlers via `rpc.cancelled()`. Set by [`CancelGuard`]
    /// when tonic drops the dispatch future (unary / client-streaming), or
    /// by the stream plumbing when the response channel's receiver is gone
    /// (server-streaming).
    cancelled: AtomicBool,

    /// ASCII request metadata captured before the dispatch consumed the
    /// `tonic::Request`. Binary (`-bin`) entries are skipped — no `.axl`
    /// use case yet.
    metadata: Vec<(String, String)>,
}

impl RpcContextInner {
    pub fn new(metadata: Vec<(String, String)>) -> Arc<Self> {
        Arc::new(Self {
            abort: Mutex::new(None),
            cancelled: AtomicBool::new(false),
            metadata,
        })
    }

    pub fn take_abort(&self) -> Option<tonic::Status> {
        self.abort.lock().unwrap().take()
    }

    pub fn mark_cancelled(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }
}

/// Flips the cancelled flag when dropped without being disarmed.
///
/// The macro-generated trait methods hold one of these across the
/// `__dispatch_*` await: tonic drops the dispatch future when the client
/// cancels or disconnects, which drops the guard, which lets the (detached)
/// `spawn_blocking` handler observe the cancellation via `rpc.cancelled()`.
pub struct CancelGuard {
    inner: Option<Arc<RpcContextInner>>,
}

impl CancelGuard {
    pub fn new(inner: Arc<RpcContextInner>) -> Self {
        Self { inner: Some(inner) }
    }

    /// Consume the guard without marking the RPC cancelled. Called after
    /// the dispatch completed normally.
    pub fn disarm(mut self) {
        self.inner = None;
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.mark_cancelled();
        }
    }
}

/// Collect the ASCII entries of a tonic `MetadataMap` as owned pairs.
/// Used by the macro-generated dispatch to capture request metadata before
/// `into_inner()` consumes the request envelope.
pub fn metadata_pairs(md: &tonic::metadata::MetadataMap) -> Vec<(String, String)> {
    md.iter()
        .filter_map(|kv| match kv {
            tonic::metadata::KeyAndValueRef::Ascii(k, v) => {
                Some((k.as_str().to_string(), v.to_str().ok()?.to_string()))
            }
            tonic::metadata::KeyAndValueRef::Binary(..) => None,
        })
        .collect()
}

impl GrpcRpcContext {
    pub fn new(inner: Arc<RpcContextInner>) -> Self {
        Self { inner }
    }
}

impl fmt::Display for GrpcRpcContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<rpc>")
    }
}

starlark_simple_value!(GrpcRpcContext);

#[starlark_value(type = "grpc.rpc")]
impl<'v> StarlarkValue<'v> for GrpcRpcContext {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(grpc_rpc_methods)
    }
}

#[starlark_module]
fn grpc_rpc_methods(registry: &mut MethodsBuilder) {
    /// Abort the current RPC with the given status.
    ///
    /// Two forms:
    /// - `rpc.abort(code, message)` — positional short form, code is a
    ///   `grpc.Code.<NAME>` int.
    /// - `rpc.abort(status = grpc.Status(...))` — keyword full form for
    ///   when you need details.
    ///
    /// Side-effect only. Handler must `return` afterward.
    fn abort<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] code: Option<Value<'v>>,
        #[starlark(require = pos)] message: Option<&str>,
        #[starlark(require = named)] status: Option<Value<'v>>,
    ) -> anyhow::Result<NoneType> {
        let ctx = this
            .downcast_ref::<GrpcRpcContext>()
            .ok_or_else(|| anyhow::anyhow!("abort called on non-rpc value"))?;

        let tonic_status = match (code, message, status) {
            (Some(c), Some(m), None) => {
                let code = c.unpack_i32().ok_or_else(|| {
                    anyhow::anyhow!(
                        "rpc.abort: code must be an int (grpc.Code.<NAME>), got {}",
                        c.get_type()
                    )
                })?;
                tonic::Status::new(code_from_i32(code), m.to_string())
            }
            (None, None, Some(s)) => into_tonic_status(s)?,
            _ => {
                return Err(anyhow::anyhow!(
                    "rpc.abort: use rpc.abort(code, message) or rpc.abort(status = grpc.Status(...))"
                ));
            }
        };

        *ctx.inner.abort.lock().unwrap() = Some(tonic_status);
        Ok(NoneType)
    }

    /// Returns `True` if the client has cancelled the call.
    fn cancelled<'v>(this: Value<'v>) -> anyhow::Result<bool> {
        let ctx = this
            .downcast_ref::<GrpcRpcContext>()
            .ok_or_else(|| anyhow::anyhow!("cancelled called on non-rpc value"))?;
        Ok(ctx.inner.is_cancelled())
    }

    /// Request metadata (headers) as a `dict[str, str]`. ASCII entries
    /// only; binary (`-bin`) entries are not exposed. Includes transport
    /// headers like `grpc-timeout` when the client set a deadline.
    fn metadata<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let ctx = this
            .downcast_ref::<GrpcRpcContext>()
            .ok_or_else(|| anyhow::anyhow!("metadata called on non-rpc value"))?;
        Ok(heap.alloc(AllocDict(
            ctx.inner
                .metadata
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        )))
    }
}
