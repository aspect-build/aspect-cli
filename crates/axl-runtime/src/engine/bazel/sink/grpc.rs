use std::{
    collections::HashMap,
    thread::{self, JoinHandle},
};

use axl_proto::{
    build_event_stream::BuildEvent,
    google::devtools::build::v1::{
        BuildStatus, PublishBuildToolEventStreamRequest, PublishLifecycleEventRequest,
    },
};
use build_event_stream::{
    build_tool,
    client::{Client, ClientError},
    lifecycle,
};

use tokio_stream::{StreamExt, wrappers::ReceiverStream};

use crate::engine::r#async::rt::AsyncRuntime;

use super::super::stream::Subscriber;
use super::retry::{
    BufferOverflow, ErrorStrategy, RetryBuffer, RetryConfig, SinkError, SinkOutcome, backoff,
    is_retryable,
};

#[derive(Debug)]
pub struct Grpc {}

/// Why `drive_stream` returned. Drives the outer state machine's reconnect
/// or terminal-exit decision.
enum DriveOutcome {
    /// `last_message` was sent and the response stream closed cleanly.
    Done,
    /// Stream broke or returned a retryable error mid-flight; reconnect if
    /// budget allows.
    Transient(ClientError),
    /// Server returned a non-retryable status; terminal regardless of budget.
    Fatal(ClientError),
    /// Buffer overflowed while we held unacked events; terminal.
    BufferFull(BufferOverflow),
    /// Upstream broadcaster closed (subscriber disconnected) without ever
    /// emitting `last_message`. Treat as clean shutdown — no more events to
    /// send, just wait for outstanding acks then exit.
    UpstreamClosed,
}

impl Grpc {
    /// Spawn a gRPC BES forwarding thread.
    ///
    /// The caller supplies `invocation_id` so that when multiple gRPC sinks
    /// are configured for the same invocation (e.g. an Aspect backend plus
    /// an internal mirror), every backend indexes this build under the same
    /// UUID. That id is reported back to the AXL layer via
    /// `Build.sink_invocation_id` — downstream consumers can build a single
    /// "View invocation" URL that resolves on any backend configured for
    /// this build.
    ///
    /// The returned thread never panics on transport or BES errors. Failures
    /// are encoded in the `SinkOutcome` per the sink's `error_strategy`, so
    /// `aspect`'s build is never killed by a flaky observability backend.
    pub fn spawn(
        rt: AsyncRuntime,
        recv: Subscriber<BuildEvent>,
        endpoint: String,
        headers: HashMap<String, String>,
        invocation_id: String,
        retry: RetryConfig,
    ) -> JoinHandle<SinkOutcome> {
        thread::spawn(move || {
            // We block_on here so the worker can drive both async tonic calls
            // and the synchronous broadcaster `recv`. block_on cannot itself
            // fail the way the previous panic-on-error path implied — if the
            // tokio handle is gone, that is a runtime-shutdown bug, not a
            // sink failure, so we still let it surface as a panic.
            rt.block_on(work(recv, endpoint, headers, invocation_id, retry))
        })
    }
}

async fn work(
    recv: Subscriber<BuildEvent>,
    endpoint: String,
    headers: HashMap<String, String>,
    invocation_id: String,
    retry: RetryConfig,
) -> SinkOutcome {
    let strategy = retry.error_strategy;
    let timeout = retry.timeout;
    let inner = work_inner(recv, endpoint.clone(), headers, invocation_id, retry);

    // Honor the configured upload deadline. Without this wrapper the BES
    // sink can stall indefinitely when the backend is slow to respond,
    // even with retry budgets exhausted: lifecycle calls, the bidi
    // stream, and `tokio::time::sleep` between retries are each bounded
    // only by their own internal logic. Wrapping the whole upload in
    // `tokio::time::timeout` mirrors Bazel's `--bes_timeout` and gives
    // the user-set knob a real effect.
    match timeout {
        Some(d) => match tokio::time::timeout(d, inner).await {
            Ok(r) => r,
            Err(_) => Err(finalize(
                strategy,
                &endpoint,
                format!("BES upload timed out after {d:?}"),
            )),
        },
        None => inner.await,
    }
}

async fn work_inner(
    recv: Subscriber<BuildEvent>,
    endpoint: String,
    headers: HashMap<String, String>,
    invocation_id: String,
    retry: RetryConfig,
) -> SinkOutcome {
    let strategy = retry.error_strategy;
    let context = |stage: &str, err: &dyn std::fmt::Display| -> SinkError {
        finalize(strategy, &endpoint, format!("{stage}: {err}"))
    };

    // Forward the synchronous broadcaster `recv` into a tokio mpsc so the
    // state machine can `select!` over it alongside the bidi response
    // stream. The forwarder runs once for the whole sink lifetime — events
    // are queued on `event_rx` and pulled lazily as we (re)open streams.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BuildEvent>();
    let _forwarder = tokio::task::spawn_blocking(move || {
        while let Ok(ev) = recv.recv() {
            if event_tx.send(ev).is_err() {
                break;
            }
        }
    });

    let mut client = Client::new(endpoint.clone(), headers)
        .await
        .map_err(|e| context("connect failed", &e))?;

    let build_id = invocation_id.clone();

    retry_lifecycle(
        &retry,
        &mut client,
        lifecycle::build_enqueued(build_id.clone(), invocation_id.clone()),
    )
    .await
    .map_err(|e| context("build_enqueued", &e))?;

    retry_lifecycle(
        &retry,
        &mut client,
        lifecycle::invocation_started(build_id.clone(), invocation_id.clone()),
    )
    .await
    .map_err(|e| context("invocation_started", &e))?;

    let mut buffer = RetryBuffer::new(retry.retry_max_buffer_size);
    let mut next_seq: i64 = 1;
    let mut attempt: u32 = 0;
    let mut last_message_sent = false;

    'reconnect: loop {
        let outcome = drive_stream(
            &mut client,
            &build_id,
            &invocation_id,
            &mut event_rx,
            &mut buffer,
            &mut next_seq,
            &mut last_message_sent,
        )
        .await;

        match outcome {
            DriveOutcome::Done | DriveOutcome::UpstreamClosed => break 'reconnect,
            DriveOutcome::Transient(err) => {
                if attempt >= retry.max_retries {
                    return Err(finalize(
                        strategy,
                        &endpoint,
                        format!("giving up after {attempt} attempts: {err}"),
                    ));
                }
                let delay = backoff(retry.retry_min_delay, attempt);
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue 'reconnect;
            }
            DriveOutcome::Fatal(err) => {
                return Err(finalize(
                    strategy,
                    &endpoint,
                    format!("non-retryable: {err}"),
                ));
            }
            DriveOutcome::BufferFull(err) => {
                return Err(finalize(strategy, &endpoint, err.to_string()));
            }
        }
    }

    // We don't know the bazel exit code at this point — that lives on the
    // parent process. Submit a successful BuildStatus; a future revision
    // can plumb the real status through.
    retry_lifecycle(
        &retry,
        &mut client,
        lifecycle::invocation_finished(
            build_id.clone(),
            invocation_id.clone(),
            BuildStatus {
                result: 0,
                final_invocation_id: build_id.clone(),
                build_tool_exit_code: Some(0),
                error_message: String::new(),
                details: None,
            },
        ),
    )
    .await
    .map_err(|e| context("invocation_finished", &e))?;

    retry_lifecycle(
        &retry,
        &mut client,
        lifecycle::build_finished(build_id.clone(), invocation_id.clone()),
    )
    .await
    .map_err(|e| context("build_finished", &e))?;

    Ok(())
}

/// Drive a single bidi stream until it ends (cleanly or with error).
///
/// Owns the request channel for this connection. Pulls events from
/// `event_rx`, assigns sequence numbers, buffers them in `buffer`, and pings
/// them down the wire. In parallel reads responses and prunes the buffer up
/// to each ack'd `sequence_number`. Replays any leftover buffered events
/// under their original seqs at the start of the connection.
async fn drive_stream(
    client: &mut Client,
    build_id: &str,
    invocation_id: &str,
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<BuildEvent>,
    buffer: &mut RetryBuffer,
    next_seq: &mut i64,
    last_message_sent: &mut bool,
) -> DriveOutcome {
    let (sender, receiver) = tokio::sync::mpsc::channel::<PublishBuildToolEventStreamRequest>(64);
    let request_stream = ReceiverStream::new(receiver);

    let response_stream = match client.publish_build_tool_event_stream(request_stream).await {
        Ok(s) => s.into_inner(),
        Err(e) => {
            return if is_retryable(&e) {
                DriveOutcome::Transient(e)
            } else {
                DriveOutcome::Fatal(e)
            };
        }
    };
    let mut response_stream = response_stream;

    // Replay any buffered (unacked) events under their original sequence
    // numbers. The server dedups via OrderedBuildEvent.sequence_number.
    for (_seq, req) in buffer.iter() {
        if sender.send(req.clone()).await.is_err() {
            // Server side closed before we could even replay; treat as
            // transient so the outer loop reconnects.
            return DriveOutcome::Transient(ClientError::Status(tonic::Status::unavailable(
                "request stream closed during replay",
            )));
        }
    }

    let mut sender_opt = Some(sender);

    loop {
        tokio::select! {
            // Pulling new events from the upstream broadcaster.
            // Disabled once last_message has been sent.
            ev = event_rx.recv(), if !*last_message_sent && sender_opt.is_some() => {
                let Some(event) = ev else {
                    // Upstream closed without a final last_message. Drop the
                    // request side so the server flushes any pending acks,
                    // and wait for the response stream to wind down.
                    sender_opt = None;
                    if buffer.is_empty() {
                        return DriveOutcome::UpstreamClosed;
                    }
                    continue;
                };

                let seq = *next_seq;
                *next_seq += 1;
                let last = event.last_message;
                let req = build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    seq,
                    &event,
                );

                if let Err(overflow) = buffer.push(seq, req.clone()) {
                    return DriveOutcome::BufferFull(overflow);
                }

                let s = sender_opt.as_ref().unwrap();
                if s.send(req).await.is_err() {
                    // Request channel rejected: the bidi stream broke.
                    // Hold onto the buffered event for replay on reconnect.
                    return DriveOutcome::Transient(ClientError::Status(
                        tonic::Status::unavailable("request stream closed mid-send"),
                    ));
                }

                if last {
                    *last_message_sent = true;
                    // Drop the request side to signal half-close, but stay
                    // in the loop so we keep draining acks until the server
                    // closes the response side.
                    sender_opt = None;
                }
            }
            // Reading server acks from the bidi response stream.
            resp = response_stream.next() => {
                match resp {
                    Some(Ok(r)) => {
                        buffer.prune_until(r.sequence_number);
                        // Once we've sent last_message AND every event we
                        // sent has been ack'd, we're done — exit without
                        // waiting for the server to close the response
                        // stream. Some BES backends keep the response
                        // stream open after the final ack, which previously
                        // caused this loop to hang indefinitely on every
                        // successful upload.
                        if *last_message_sent && buffer.is_empty() {
                            return DriveOutcome::Done;
                        }
                        if sender_opt.is_none() && buffer.is_empty() {
                            return DriveOutcome::UpstreamClosed;
                        }
                    }
                    Some(Err(status)) => {
                        let err = ClientError::Status(status);
                        return if is_retryable(&err) {
                            DriveOutcome::Transient(err)
                        } else {
                            DriveOutcome::Fatal(err)
                        };
                    }
                    None => {
                        // Server closed the response stream.
                        if *last_message_sent && buffer.is_empty() {
                            return DriveOutcome::Done;
                        }
                        if sender_opt.is_none() && buffer.is_empty() {
                            return DriveOutcome::UpstreamClosed;
                        }
                        // Server closed prematurely with unacked events —
                        // treat as transient so we reconnect and replay.
                        return DriveOutcome::Transient(ClientError::Status(
                            tonic::Status::unavailable("response stream closed prematurely"),
                        ));
                    }
                }
            }
        }
    }
}

/// Retry an idempotent lifecycle call. Each attempt uses the same backoff
/// as stream reconnects. Per the design, every call gets `max_retries`
/// attempts and the counter is local to the call (a successful lifecycle
/// event resets to a clean state for the next one).
async fn retry_lifecycle(
    cfg: &RetryConfig,
    client: &mut Client,
    request: PublishLifecycleEventRequest,
) -> Result<(), ClientError> {
    let mut attempt: u32 = 0;
    loop {
        match client.publish_lifecycle_event(request.clone()).await {
            Ok(_) => return Ok(()),
            Err(err) => {
                if !is_retryable(&err) || attempt >= cfg.max_retries {
                    return Err(err);
                }
                tokio::time::sleep(backoff(cfg.retry_min_delay, attempt)).await;
                attempt += 1;
            }
        }
    }
}

fn finalize(strategy: ErrorStrategy, endpoint: &str, last_error: String) -> SinkError {
    if matches!(strategy, ErrorStrategy::Warn) {
        eprintln!("WARN: BES sink {endpoint} giving up: {last_error}");
    }
    SinkError {
        strategy,
        last_error,
    }
}
