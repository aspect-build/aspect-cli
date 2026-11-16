//! Build events for tests, and their framing on the wire.

use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_finished::ExitCode;
use axl_proto::build_event_stream::{BuildEvent, BuildFinished, BuildStarted};
use prost::Message;

use super::binary_file::REMOTE_CACHE_EVICTED;

pub(crate) fn started() -> BuildEvent {
    BuildEvent {
        payload: Some(Payload::Started(BuildStarted::default())),
        ..Default::default()
    }
}

pub(crate) fn build_finished(code: i32, last_message: bool) -> BuildEvent {
    BuildEvent {
        last_message,
        payload: Some(Payload::Finished(BuildFinished {
            exit_code: Some(ExitCode {
                code,
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// The `BuildFinished` that ends an attempt bazel is about to retry.
pub(crate) fn retry_marker(last_message: bool) -> BuildEvent {
    build_finished(REMOTE_CACHE_EVICTED, last_message)
}

/// An event with no payload.
pub(crate) fn plain(last_message: bool) -> BuildEvent {
    BuildEvent {
        last_message,
        ..Default::default()
    }
}

/// `event` as bazel writes it: length-delimited.
pub(crate) fn encoded(event: &BuildEvent) -> Vec<u8> {
    event.encode_length_delimited_to_vec()
}

/// `events` as bazel writes them, one after the other.
pub(crate) fn encoded_all(events: &[BuildEvent]) -> Vec<u8> {
    events.iter().flat_map(encoded).collect()
}
