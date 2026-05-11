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
    work_inner(recv, endpoint, headers, invocation_id, retry).await
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

    // Bound the initial connect: a TLS handshake against an unresponsive
    // endpoint can stall indefinitely without any of the retry machinery
    // ever running.
    const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    let mut client =
        match tokio::time::timeout(CONNECT_TIMEOUT, Client::new(endpoint.clone(), headers)).await {
            Ok(r) => r.map_err(|e| context("connect failed", &e))?,
            Err(_) => {
                return Err(finalize(
                    strategy,
                    &endpoint,
                    "connect timed out after 10s".to_string(),
                ));
            }
        };

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

    // Post-stream lifecycle. The user's `bes_timeout` knob lands here:
    // it mirrors Bazel's `--bes_timeout`, the deadline for BES upload
    // completion *after* the build and tests finish. When unset (or
    // explicitly `"0s"`, which the Starlark surface maps to None and
    // documents as "no deadline"), no deadline applies — matching the
    // documented behavior even at the cost of a possible hang against a
    // silent backend. The events themselves were already streamed, so
    // failing these is non-fatal under the `Warn` default.
    //
    // We don't know the bazel exit code at this point — that lives on
    // the parent process. Submit a successful BuildStatus; a future
    // revision can plumb the real status through.
    let post_stream = async {
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

        Ok::<(), SinkError>(())
    };
    match retry.timeout {
        Some(d) => match tokio::time::timeout(d, post_stream).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(finalize(
                strategy,
                &endpoint,
                format!("post-stream lifecycle exceeded {d:?} budget"),
            )),
        },
        None => post_stream.await,
    }
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

    // Bound the bidi stream open: tonic does not add a deadline and the
    // server may accept the TCP/TLS handshake without ever responding.
    const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    let response_stream = match tokio::time::timeout(
        OPEN_TIMEOUT,
        client.publish_build_tool_event_stream(request_stream),
    )
    .await
    {
        Ok(Ok(s)) => s.into_inner(),
        Ok(Err(e)) => {
            return if is_retryable(&e) {
                DriveOutcome::Transient(e)
            } else {
                DriveOutcome::Fatal(e)
            };
        }
        Err(_) => {
            return DriveOutcome::Transient(ClientError::Status(tonic::Status::deadline_exceeded(
                "stream open timed out",
            )));
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

    // Hard deadline for waiting after the request side is half-closed.
    // Once we drop `sender_opt` (last_message sent or upstream broadcaster
    // closed), the server is supposed to ack any pending events and then
    // close the response stream. In practice some BES backends sit on the
    // half-closed connection without acking or closing — without this
    // deadline `wait()` blocks indefinitely on the sink JoinHandle and the
    // entire build task hangs at end-of-build. 30s is generous enough for
    // a slow ack while still bounding the total stall.
    const HALF_CLOSE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
    let mut half_close_deadline: Option<tokio::time::Instant> = None;

    // If a previous attempt already sent last_message, the replay above
    // resent it on this fresh stream. The event-receive arm below stays
    // disabled (last_message_sent is set), so the `if last { ... }`
    // branch that normally drops the sender will never fire on this
    // attempt. Close the request side now and arm the half-close
    // deadline ourselves — otherwise drive_stream parks in
    // response_stream.next() forever waiting for an ack or close that a
    // flaky backend may never send, and Build.wait() hangs.
    if *last_message_sent {
        sender_opt = None;
        half_close_deadline = Some(tokio::time::Instant::now() + HALF_CLOSE_DEADLINE);
    }

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
                    half_close_deadline.get_or_insert_with(|| {
                        tokio::time::Instant::now() + HALF_CLOSE_DEADLINE
                    });
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
                    half_close_deadline.get_or_insert_with(|| {
                        tokio::time::Instant::now() + HALF_CLOSE_DEADLINE
                    });
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
            // Hard deadline arm: only enabled once the request side is
            // half-closed. Two outcomes:
            //   * Buffer empty — every event we sent was already acked, so
            //     the only thing we were waiting for is the server closing
            //     the response stream. Return Done/UpstreamClosed; this
            //     bypasses BES backends that keep the stream open forever
            //     after the final ack.
            //   * Buffer non-empty — the server stopped acking unacked
            //     events. Surface as Transient so the outer retry budget
            //     applies; if every replay attempt also times out, the
            //     outer loop converts to a terminal SinkError that
            //     `wait()` resolves per `error_strategy`. Returning Done
            //     here would silently drop unacked events on `fail_at_end`
            //     / `abort`.
            _ = async {
                match half_close_deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await,
                }
            }, if half_close_deadline.is_some() => {
                if !buffer.is_empty() {
                    return DriveOutcome::Transient(ClientError::Status(
                        tonic::Status::deadline_exceeded(
                            "BES half-close deadline elapsed with unacked events",
                        ),
                    ));
                }
                return if *last_message_sent {
                    DriveOutcome::Done
                } else {
                    DriveOutcome::UpstreamClosed
                };
            }
        }
    }
}

/// Retry an idempotent lifecycle call. Each attempt uses the same backoff
/// as stream reconnects. Per the design, every call gets `max_retries`
/// attempts and the counter is local to the call (a successful lifecycle
/// event resets to a clean state for the next one).
///
/// Each attempt is bounded by a per-RPC deadline. Without it, a server
/// that accepts the connection but never responds wedges the sink thread
/// forever — `tonic` does not add a default RPC deadline, and the outer
/// retry loop only spins on errors, never on hangs.
async fn retry_lifecycle(
    cfg: &RetryConfig,
    client: &mut Client,
    request: PublishLifecycleEventRequest,
) -> Result<(), ClientError> {
    const PER_ATTEMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    let mut attempt: u32 = 0;
    loop {
        let result = match tokio::time::timeout(
            PER_ATTEMPT_TIMEOUT,
            client.publish_lifecycle_event(request.clone()),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => Err(ClientError::Status(tonic::Status::deadline_exceeded(
                "lifecycle attempt timed out",
            ))),
        };
        match result {
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
