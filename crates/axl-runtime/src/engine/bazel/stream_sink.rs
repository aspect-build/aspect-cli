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
    /// Spawn a gRPC BES forwarding thread.
    ///
    /// The caller supplies `invocation_id` so that when multiple gRPC sinks
    /// are configured for the same invocation (e.g. an Aspect backend plus
    /// an internal mirror), every backend indexes this build under the same
    /// UUID. That id is reported back to the AXL layer via
    /// `Build.sink_invocation_id` — downstream consumers can build a single
    /// "View invocation" URL that resolves on any backend configured for
    /// this build.
    pub fn spawn(
        rt: AsyncRuntime,
        recv: Subscriber<BuildEvent>,
        endpoint: String,
        headers: HashMap<String, String>,
        invocation_id: String,
    ) -> JoinHandle<()> {
        thread::spawn(move || {
            // BES forwarding is a side-channel — its failure shouldn't take
            // the build down. The build's own success is determined by Bazel's
            // exit code; if a sink panics, the user has likely already seen
            // the work they care about complete (test output, lint findings,
            // delivery results) and dropping the rest of the BES forward is
            // recoverable. Log via stderr (also captured by axl.log under
            // ASPECT_DEBUG=1) and return cleanly so `Build::wait()` can join
            // the thread without surfacing it as a build failure.
            let inv_id = invocation_id.clone();
            let result = rt.block_on(async {
                GrpcEventStreamSink::task_spawn(recv, endpoint, headers, invocation_id)
                    .await
                    .await
            });
            match result {
                Ok(Ok(())) => {}
                Ok(Err(sink_err)) => {
                    eprintln!(
                        "warning: BES sink failed (non-fatal) sink_invocation_id={}: {}",
                        inv_id, sink_err
                    );
                    tracing::trace!(
                        target: "axl.log",
                        "debug: BES sink failed (non-fatal) sink_invocation_id={}: {}",
                        inv_id, sink_err
                    );
                }
                Err(join_err) => {
                    eprintln!(
                        "warning: BES sink task did not complete cleanly (non-fatal) sink_invocation_id={}: {}",
                        inv_id, join_err
                    );
                    tracing::trace!(
                        target: "axl.log",
                        "debug: BES sink task did not complete cleanly (non-fatal) sink_invocation_id={}: {}",
                        inv_id, join_err
                    );
                }
            }
        })
    }

    pub async fn task_spawn(
        recv: Subscriber<BuildEvent>,
        endpoint: String,
        headers: HashMap<String, String>,
        invocation_id: String,
    ) -> task::JoinHandle<Result<(), SinkError>> {
        tokio::task::spawn(GrpcEventStreamSink::work(
            recv,
            endpoint,
            headers,
            invocation_id,
        ))
    }

    async fn work(
        recv: Subscriber<BuildEvent>,
        endpoint: String,
        headers: HashMap<String, String>,
        invocation_id: String,
    ) -> Result<(), SinkError> {
        // All `tracing::trace!(target: "axl.log", ...)` lines below surface on
        // stderr under `ASPECT_DEBUG=1` via the same FileSinksLayer that
        // catches axl's `trace.log(...)` calls. They give a phase-by-phase
        // trail of the gRPC sink's lifecycle so when `sink_invocation_id` is
        // captured but the backend later returns 404, the log shows exactly
        // which lifecycle event was the last one published before the
        // process exited.
        tracing::trace!(
            target: "axl.log",
            "debug: GrpcEventStreamSink: connecting to backend endpoint={} sink_invocation_id={}",
            endpoint, invocation_id
        );
        let mut client = Client::new(endpoint, headers).await?;

        let build_id = invocation_id.clone();

        tracing::trace!(target: "axl.log", "debug: GrpcEventStreamSink: publishing build_enqueued sink_invocation_id={}", invocation_id);
        client
            .publish_lifecycle_event(lifecycle::build_enqueued(
                build_id.to_string(),
                invocation_id.to_string(),
            ))
            .await?;

        tracing::trace!(target: "axl.log", "debug: GrpcEventStreamSink: publishing invocation_started sink_invocation_id={}", invocation_id);
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
        tracing::trace!(target: "axl.log", "debug: GrpcEventStreamSink: event stream drained sink_invocation_id={}", invocation_id);

        tracing::trace!(target: "axl.log", "debug: GrpcEventStreamSink: publishing invocation_finished sink_invocation_id={}", invocation_id);
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

        tracing::trace!(target: "axl.log", "debug: GrpcEventStreamSink: publishing build_finished sink_invocation_id={}", invocation_id);
        client
            .publish_lifecycle_event(lifecycle::build_finished(
                build_id.to_string(),
                invocation_id.to_string(),
            ))
            .await?;

        tracing::trace!(target: "axl.log", "debug: GrpcEventStreamSink: all lifecycle events published; closing sink_invocation_id={}", invocation_id);
        Ok(())
    }
}
