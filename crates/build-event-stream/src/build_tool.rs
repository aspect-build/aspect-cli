use std::time::SystemTime;

use axl_proto::{
    build_event_stream::BuildEvent as BazelBuildEvent,
    google::devtools::build::v1::{
        build_event::Event, BuildEvent, OrderedBuildEvent, PublishBuildToolEventStreamRequest,
    },
};
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
    let packed = Any::from_msg(event).expect("failed to encode bazel event");
    stream_request(
        build_id,
        invocation_id,
        seq,
        BuildEvent {
            event_time: Some(Timestamp::from(SystemTime::now())),
            event: Some(Event::BazelEvent(packed)),
        },
    )
}
