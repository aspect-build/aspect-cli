use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{
    NoSerialize, StarlarkValue, Value, ValueLike, none::NoneType, starlark_value,
};

use super::context::RpcContextInner;
use super::status::into_tonic_status;

/// Implemented per-response-type by macro-generated code. Encapsulates a
/// typed `mpsc::Sender<Result<T, Status>>` plus the downcast/clone needed
/// to convert an opaque Starlark `Value` into `T`.
///
/// Decouples the polymorphic Starlark `GrpcStream` value from the
/// concrete response type, so a single `GrpcStream` Starlark type works
/// for every server-streaming RPC.
pub trait DynSender: Send + Sync + std::fmt::Debug {
    /// Downcast `value` to the response type and forward to the channel.
    /// Blocks the caller on backpressure (the underlying `blocking_send`).
    /// Best-effort on cancellation: returns `Ok(())` after dropping the
    /// message so handler producer loops can keep going and detect
    /// cancellation via `stream.cancelled()`.
    fn send_value(&self, value: Value<'_>) -> anyhow::Result<()>;

    /// Close the channel. `None` = success. `Some(status)` = error
    /// trailing status. Idempotent.
    fn close(&self, status: Option<tonic::Status>);

    /// Whether the channel has already been closed (via `close` or
    /// completion).
    fn is_closed(&self) -> bool;

    /// Human-readable response type name for error messages (e.g.
    /// `"Operation"`). Used by `stream.push` to produce a helpful error
    /// when a handler returns the wrong type.
    fn response_type_name(&self) -> &'static str;
}

/// Response sink passed to server-streaming handlers as `stream`. Outlives
/// the handler call so long-running RPCs (Execute, WaitExecution) can
/// stash it and have other code push to it later.
///
/// The concrete response type is hidden behind a [`DynSender`] trait
/// object; pushes are type-checked at runtime by the macro-generated
/// sender impl.
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct GrpcStream {
    #[allocative(skip)]
    sender: Arc<dyn DynSender>,

    #[allocative(skip)]
    rpc: Arc<RpcContextInner>,
}

impl GrpcStream {
    pub fn new(sender: Arc<dyn DynSender>, rpc: Arc<RpcContextInner>) -> Self {
        Self { sender, rpc }
    }
}

impl fmt::Display for GrpcStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<stream {}>", self.sender.response_type_name())
    }
}

starlark_simple_value!(GrpcStream);

#[starlark_value(type = "grpc.stream")]
impl<'v> StarlarkValue<'v> for GrpcStream {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(grpc_stream_methods)
    }
}

#[starlark_module]
fn grpc_stream_methods(registry: &mut MethodsBuilder) {
    /// Push one message to the response stream. Runtime-typechecked
    /// against the RPC's response type; mismatched types raise.
    ///
    /// Blocks on backpressure. After the client has cancelled, push is
    /// best-effort (the message is dropped, no error raised) — producer
    /// loops should poll `stream.cancelled()` to bail out.
    fn push<'v>(this: Value<'v>, msg: Value<'v>) -> anyhow::Result<NoneType> {
        let s = this
            .downcast_ref::<GrpcStream>()
            .ok_or_else(|| anyhow::anyhow!("push called on non-stream value"))?;
        s.sender.send_value(msg)?;
        Ok(NoneType)
    }

    /// Close the stream. No-arg = success; `status =` = error.
    /// Subsequent `push` calls after `complete` raise.
    fn complete<'v>(
        this: Value<'v>,
        #[starlark(require = named)] status: Option<Value<'v>>,
    ) -> anyhow::Result<NoneType> {
        let s = this
            .downcast_ref::<GrpcStream>()
            .ok_or_else(|| anyhow::anyhow!("complete called on non-stream value"))?;
        let tonic_status = match status {
            None => None,
            Some(v) => Some(into_tonic_status(v)?),
        };
        s.sender.close(tonic_status);
        Ok(NoneType)
    }

    /// Returns `True` if the client has cancelled the originating call.
    /// Same signal as `rpc.cancelled()` from the call this stream came from.
    fn cancelled<'v>(this: Value<'v>) -> anyhow::Result<bool> {
        let s = this
            .downcast_ref::<GrpcStream>()
            .ok_or_else(|| anyhow::anyhow!("cancelled called on non-stream value"))?;
        Ok(s.rpc.is_cancelled())
    }
}
