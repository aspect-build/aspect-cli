use std::{
    collections::HashMap,
    sync::mpsc::RecvError,
    thread::{self, JoinHandle},
};

use axl_proto::{
    build_event_stream::BuildEvent,
    google::devtools::build::v1::{BuildStatus, PublishBuildToolEventStreamRequest},
};
use build_event_stream::{
    build_tool,
    client::{Client, ClientError},
    lifecycle,
};

use thiserror::Error;
use tokio::{sync::mpsc::error::SendError, task};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

use super::super::r#async::rt::AsyncRuntime;
use super::stream::Subscriber;

#[derive(Error, Debug)]
pub enum SinkError {
    #[error("channel disconnected")]
    RecvError(#[from] RecvError),
    #[error(transparent)]
    ClientError(#[from] ClientError),
    #[error(transparent)]
    SendError(#[from] SendError<PublishBuildToolEventStreamRequest>),
}

#[derive(Debug)]
pub struct GrpcEventStreamSink {}

impl GrpcEventStreamSink {
    pub fn spawn(
        rt: AsyncRuntime,
        recv: Subscriber<BuildEvent>,
        endpoint: String,
        headers: HashMap<String, String>,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            rt.block_on(async {
                GrpcEventStreamSink::task_spawn(recv, endpoint, headers)
                    .await
                    .await
            })
            .expect("failed to join")
            .expect("failed to wait")
        })
    }

    pub async fn task_spawn(
        recv: Subscriber<BuildEvent>,
        endpoint: String,
        headers: HashMap<String, String>,
    ) -> task::JoinHandle<Result<(), SinkError>> {
        tokio::task::spawn(GrpcEventStreamSink::work(recv, endpoint, headers))
    }

    async fn work(
        recv: Subscriber<BuildEvent>,
        endpoint: String,
        headers: HashMap<String, String>,
    ) -> Result<(), SinkError> {
        let mut client = Client::new(endpoint, headers).await?;

        let uuid = uuid::Uuid::new_v4().to_string();
        let build_id = uuid.to_string();
        let invocation_id = uuid.to_string();

        client
            .publish_lifecycle_event(lifecycle::build_enqueued(
                build_id.to_string(),
                invocation_id.to_string(),
            ))
            .await?;

        client
            .publish_lifecycle_event(lifecycle::invocation_started(
                build_id.to_string(),
                invocation_id.to_string(),
            ))
            .await?;

        let seq = 0;

        let (sender, receiver) =
            tokio::sync::mpsc::channel::<PublishBuildToolEventStreamRequest>(1);

        let rstream = ReceiverStream::new(receiver);
        let stream = client.publish_build_tool_event_stream(rstream);

        // Clone for use in async block
        let build_id_for_events = build_id.clone();
        let invocation_id_for_events = invocation_id.clone();

        let (a, b): (Result<(), SinkError>, Result<(), SinkError>) = tokio::join!(
            async move {
                let mut stream = stream.await?.into_inner();
                while let Some(event) = stream.next().await {
                    match event {
                        // Succesfully received BES event ack
                        // TODO: Use this information to control how many inflight BES events we should be
                        // sending.
                        Ok(_ev) => {}
                        Err(err) => eprintln!("{}", err),
                    }
                }
                Ok(())
            },
            async move {
                let mut seq = seq;
                loop {
                    seq += 1;
                    let event = recv.recv();
                    if event.is_err() {
                        break;
                    }
                    let event = event.unwrap();

                    sender
                        .send(build_tool::bazel_event(
                            build_id_for_events.to_string(),
                            invocation_id_for_events.to_string(),
                            seq,
                            &event,
                        ))
                        .await?;

                    if event.last_message {
                        drop(sender);
                        break;
                    }
                }
                Ok(())
            }
        );

        a?;
        b?;

        client
            .publish_lifecycle_event(lifecycle::invocation_finished(
                build_id.to_string(),
                invocation_id.to_string(),
                BuildStatus {
                    result: 0,
                    final_invocation_id: build_id.to_string(),
                    build_tool_exit_code: Some(0),
                    error_message: String::new(),
                    details: None,
                },
            ))
            .await?;

        client
            .publish_lifecycle_event(lifecycle::build_finished(
                build_id.to_string(),
                invocation_id.to_string(),
            ))
            .await?;

        Ok(())
    }
}
