use std::time::SystemTime;

use axl_proto::{
    build_event_stream::BuildEvent as BazelBuildEvent,
    google::devtools::build::v1::{
        BuildEvent, OrderedBuildEvent, PublishBuildToolEventStreamRequest, build_event::Event,
    },
};
use prost::{Message, Name, bytes::Bytes};
use prost_types::{Any, Timestamp};

use super::stream_id::stream_id;

pub fn stream_request(
    build_id: String,
    invocation_id: String,
    seq: i64,
    event: BuildEvent,
) -> PublishBuildToolEventStreamRequest {
    PublishBuildToolEventStreamRequest {
        check_preceding_lifecycle_events_present: false,
        notification_keywords: vec![],
        ordered_build_event: Some(OrderedBuildEvent {
            sequence_number: seq,
            stream_id: Some(stream_id(
                build_id,
                invocation_id,
                event.event.as_ref().unwrap(),
            )),
            event: Some(event),
        }),
        project_id: String::new(),
    }
}

pub fn bazel_event(
    build_id: String,
    invocation_id: String,
    seq: i64,
    event: &BazelBuildEvent,
) -> PublishBuildToolEventStreamRequest {
    bazel_event_encoded(
        build_id,
        invocation_id,
        seq,
        Bytes::from(event.encode_to_vec()),
        Timestamp::from(SystemTime::now()),
    )
}

/// [`bazel_event`] for an event already on the wire: `encoded` is the
/// serialized `BuildEvent` message exactly as bazel wrote it, so a caller
/// holding those bytes skips a decode/re-encode round trip.
pub fn bazel_event_encoded(
    build_id: String,
    invocation_id: String,
    seq: i64,
    encoded: Bytes,
    event_time: Timestamp,
) -> PublishBuildToolEventStreamRequest {
    let packed = Any {
        type_url: BazelBuildEvent::type_url(),
        value: encoded.into(),
    };
    stream_request(
        build_id,
        invocation_id,
        seq,
        BuildEvent {
            event_time: Some(event_time),
            event: Some(Event::BazelEvent(packed)),
        },
    )
}
