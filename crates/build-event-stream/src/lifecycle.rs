use std::time::SystemTime;

use axl_proto::google::devtools::build::v1::{
    BuildEvent, BuildStatus, OrderedBuildEvent, PublishLifecycleEventRequest,
    build_event::{
        BuildEnqueued, BuildFinished, Event, InvocationAttemptFinished, InvocationAttemptStarted,
    },
    publish_lifecycle_event_request::ServiceLevel,
};
use prost_types::Timestamp;

use super::stream_id::stream_id;

pub fn lifecycle_request(
    build_id: String,
    invocation_id: String,
    sequence_number: i64,
    ev: Event,
) -> PublishLifecycleEventRequest {
    let now = SystemTime::now();
    let stream_id = stream_id(build_id.clone(), invocation_id.clone(), &ev);
    PublishLifecycleEventRequest {
        service_level: ServiceLevel::Interactive as i32,
        build_event: Some(OrderedBuildEvent {
            sequence_number,
            stream_id: Some(stream_id),
            event: Some(BuildEvent {
                event: Some(ev),
                event_time: Some(Timestamp::from(now)),
            }),
        }),
        stream_timeout: None,
        notification_keywords: vec![],
        project_id: String::new(),
        check_preceding_lifecycle_events_present: false,
    }
}

// https://github.com/bazelbuild/bazel/blob/198c4c8aae1b5ef3d202f602932a99ce19707fc4/src/main/java/com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java#L73
pub fn build_enqueued(build_id: String, invocation_id: String) -> PublishLifecycleEventRequest {
    lifecycle_request(
        build_id,
        invocation_id,
        1,
        Event::BuildEnqueued(BuildEnqueued::default()),
    )
}

// https://github.com/bazelbuild/bazel/blob/198c4c8aae1b5ef3d202f602932a99ce19707fc4/src/main/java/com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java#L84
pub fn build_finished(build_id: String, invocation_id: String) -> PublishLifecycleEventRequest {
    lifecycle_request(
        build_id,
        invocation_id,
        2,
        Event::BuildFinished(BuildFinished::default()),
    )
}

//https://github.com/bazelbuild/bazel/blob/198c4c8aae1b5ef3d202f602932a99ce19707fc4/src/main/java/com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java#L95
pub fn invocation_started(build_id: String, invocation_id: String) -> PublishLifecycleEventRequest {
    lifecycle_request(
        build_id,
        invocation_id,
        2,
        Event::InvocationAttemptStarted(InvocationAttemptStarted::default()),
    )
}

//https://github.com/bazelbuild/bazel/blob/198c4c8aae1b5ef3d202f602932a99ce19707fc4/src/main/java/com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java#L108
pub fn invocation_finished(
    build_id: String,
    invocation_id: String,
    build_status: BuildStatus,
) -> PublishLifecycleEventRequest {
    lifecycle_request(
        build_id,
        invocation_id,
        2,
        Event::InvocationAttemptFinished(InvocationAttemptFinished {
            details: None,
            invocation_status: Some(build_status),
        }),
    )
}
