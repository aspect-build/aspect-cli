//! The Build Event Protocol.

use std::sync::Arc;

use axl_proto::build_event_stream::BuildEvent;
use prost::bytes::Bytes;

pub mod binary_file;
pub mod client;
pub mod feed;
pub mod kind;
pub mod outputs;
pub mod redact;
pub mod sink;
pub mod warning;

/// One build event as a feed hands it to subscribers: decoded, and with it
/// the exact bytes it was decoded from (re-encoded when redaction changed it).
/// A subscriber that only forwards events — the gRPC sink — takes the bytes
/// and never touches the proto, so a backlog behind a slow backend costs what
/// bazel wrote and no more; subscribers that read fields share the one decode
/// behind the `Arc`.
#[derive(Debug)]
pub struct BuildEventEnvelope {
    pub event: BuildEvent,
    /// The serialized `BuildEvent` message, without its length prefix.
    pub encoded: Bytes,
}

impl BuildEventEnvelope {
    /// The decoded event, moved out when this was the last reference and
    /// cloned otherwise.
    pub fn into_event(this: Arc<Self>) -> BuildEvent {
        match Arc::try_unwrap(this) {
            Ok(envelope) => envelope.event,
            Err(shared) => shared.event.clone(),
        }
    }
}
