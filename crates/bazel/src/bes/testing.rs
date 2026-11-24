//! Build events for tests, and their framing on the wire.

use std::sync::Arc;

use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_event_id::{ActionCompletedId, Id};
use axl_proto::build_event_stream::build_finished::ExitCode;
use axl_proto::build_event_stream::{
    ActionExecuted, BuildEvent, BuildEventId, BuildFinished, BuildStarted,
};
use prost::Message;
use prost::bytes::Bytes;

use super::BuildEventEnvelope;
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

/// The id of the action `label` owns.
pub(crate) fn action_id(label: &str) -> BuildEventId {
    BuildEventId {
        id: Some(Id::ActionCompleted(ActionCompletedId {
            primary_output: format!("bazel-out/bin/{label}.out"),
            label: label.into(),
            configuration: None,
        })),
    }
}

/// The `ActionExecuted` of the action `label` owns.
pub(crate) fn action(label: &str, success: bool) -> BuildEvent {
    BuildEvent {
        id: Some(action_id(label)),
        payload: Some(Payload::Action(ActionExecuted {
            success,
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// `event`, announcing the actions `labels` own.
pub(crate) fn announcing(event: BuildEvent, labels: &[&str]) -> BuildEvent {
    let mut event = event;
    event.children.extend(labels.iter().map(|l| action_id(l)));
    event
}

/// `event` as a feed hands it to subscribers.
pub(crate) fn envelope(event: BuildEvent) -> Arc<BuildEventEnvelope> {
    let encoded = Bytes::from(event.encode_to_vec());
    Arc::new(BuildEventEnvelope { event, encoded })
}
