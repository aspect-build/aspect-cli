use axl_proto::google::devtools::build::v1::{
    StreamId, build_event::Event, stream_id::BuildComponent,
};

pub fn stream_id(build_id: String, invocation_id: String, ev: &Event) -> StreamId {
    StreamId {
        build_id,
        invocation_id,
        component: match ev {
            Event::InvocationAttemptStarted(_) | Event::InvocationAttemptFinished(_) => {
                BuildComponent::Controller
            }
            Event::BuildEnqueued(_) | Event::BuildFinished(_) => BuildComponent::Controller,
            Event::ComponentStreamFinished(_) | Event::BazelEvent(_) => BuildComponent::Tool,
            // These are not handled by Bazel.
            Event::ConsoleOutput(_) => todo!(),
            Event::BuildExecutionEvent(_) => todo!(),
            Event::SourceFetchEvent(_) => todo!(),
        } as i32,
    }
}
