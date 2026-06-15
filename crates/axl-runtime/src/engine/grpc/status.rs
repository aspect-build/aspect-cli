use std::fmt;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, ValueLike, starlark_value};

/// gRPC `Status` value constructed via `grpc.Status(code = ..., message = ...)`.
///
/// `details` (proto Any payloads for typed error metadata) is deliberately
/// omitted in v1 — the slot stays open in the constructor signature and
/// gets wired through when needed.
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct GrpcStatus {
    pub code: i32,
    pub message: String,
}

impl fmt::Display for GrpcStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Status(code={}, message={:?})", self.code, self.message)
    }
}

starlark_simple_value!(GrpcStatus);

#[starlark_value(type = "grpc.Status")]
impl<'v> StarlarkValue<'v> for GrpcStatus {
    fn get_attr(&self, attr: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attr {
            "code" => Some(heap.alloc(self.code)),
            "message" => Some(heap.alloc(self.message.clone())),
            _ => None,
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["code".to_owned(), "message".to_owned()]
    }
}

/// Extract a tonic `Status` from a `grpc.Status` Starlark value. Used by
/// `rpc.abort(status=...)` and `stream.complete(status=...)`.
pub fn into_tonic_status(value: Value<'_>) -> anyhow::Result<tonic::Status> {
    let s = value
        .downcast_ref::<GrpcStatus>()
        .ok_or_else(|| anyhow::anyhow!("expected grpc.Status value, got {}", value.get_type()))?;
    Ok(tonic::Status::new(code_from_i32(s.code), s.message.clone()))
}

pub fn code_from_i32(c: i32) -> tonic::Code {
    match c {
        0 => tonic::Code::Ok,
        1 => tonic::Code::Cancelled,
        2 => tonic::Code::Unknown,
        3 => tonic::Code::InvalidArgument,
        4 => tonic::Code::DeadlineExceeded,
        5 => tonic::Code::NotFound,
        6 => tonic::Code::AlreadyExists,
        7 => tonic::Code::PermissionDenied,
        8 => tonic::Code::ResourceExhausted,
        9 => tonic::Code::FailedPrecondition,
        10 => tonic::Code::Aborted,
        11 => tonic::Code::OutOfRange,
        12 => tonic::Code::Unimplemented,
        13 => tonic::Code::Internal,
        14 => tonic::Code::Unavailable,
        15 => tonic::Code::DataLoss,
        16 => tonic::Code::Unauthenticated,
        _ => tonic::Code::Unknown,
    }
}

/// `grpc.Status(code = ..., message = ...)` constructor.
pub fn status_ctor(code: i32, message: String) -> GrpcStatus {
    GrpcStatus { code, message }
}
