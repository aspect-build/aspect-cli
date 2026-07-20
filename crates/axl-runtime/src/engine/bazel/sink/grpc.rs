use std::{
    collections::HashMap,
    sync::OnceLock,
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

use crate::diag;
use crate::engine::r#async::rt::AsyncRuntime;

use super::super::stream::Subscriber;
use super::retry::{
    BufferOverflow, RetryBuffer, RetryConfig, SinkError, SinkOutcome, SinkStats, backoff,
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

impl DriveOutcome {
    /// Short label for `ASPECT_DEBUG` logging.
    fn label(&self) -> String {
        match self {
            DriveOutcome::Done => "Done".to_string(),
            DriveOutcome::UpstreamClosed => "UpstreamClosed".to_string(),
            DriveOutcome::Transient(e) => format!("Transient({e})"),
            DriveOutcome::Fatal(e) => format!("Fatal({e})"),
            DriveOutcome::BufferFull(e) => format!("BufferFull({e})"),
        }
    }
}

impl Grpc {
    /// Spawn a gRPC BES forwarding thread. All sinks for a single build
    /// share `invocation_id` so every backend indexes it under one UUID.
    /// Transport / BES errors never panic; they surface as `SinkOutcome`
    /// for the caller's `sink.wait()` to inspect.
    pub fn spawn(
        rt: AsyncRuntime,
        recv: Subscriber<BuildEvent>,
        endpoint: String,
        headers: HashMap<String, String>,
        invocation_id: String,
        retry: RetryConfig,
    ) -> JoinHandle<(SinkStats, SinkOutcome)> {
        thread::spawn(move || rt.block_on(work(recv, endpoint, headers, invocation_id, retry)))
    }
}

/// Whether `ASPECT_DEBUG` was set when the process started. Cached
/// once so sink hot paths don't repeat the env lookup on every call.
fn debug_enabled() -> bool {
    static D: OnceLock<bool> = OnceLock::new();
    *D.get_or_init(|| {
        std::env::var_os("ASPECT_DEBUG")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    })
}

/// Emit a sink-lifecycle log line on stderr when `ASPECT_DEBUG` is set.
/// Prefixed `BES sink <endpoint> [<id8>]:` so a single `grep BES\ sink`
/// pulls every event for one sink across a run, and multi-sink
/// configurations (e.g. an Aspect backend plus an internal mirror)
/// stay distinguishable via the endpoint segment.
fn dbg(endpoint: &str, invocation_id: &str, msg: &str) {
    if !debug_enabled() {
        return;
    }
    let short = invocation_id.get(..8).unwrap_or(invocation_id);
    eprintln!("BES sink {endpoint} [{short}]: {msg}");
}

/// Emit a user-facing `WARNING:` for this sink, prefixed `BES sink <endpoint>:`
/// to distinguish it in multi-sink configurations. Unlike [`dbg`], it is not
/// `ASPECT_DEBUG`-gated: BES upload is best-effort and never fails the build,
/// so this is the only signal that events were delayed or lost.
fn warn(endpoint: &str, msg: &str) {
    diag::warn(&format!("BES sink {endpoint}: {msg}"));
}

/// Send one of the unary lifecycle events (`build_enqueued`,
/// `invocation_started`, `invocation_finished`, `build_finished`),
/// timing the round trip and logging the result. Errors are wrapped
/// via `finalize` so a failure exits the sink with a `SinkError`
/// rather than propagating as a raw `ClientError`.
async fn send_lifecycle_logged(
    name: &str,
    retry: &RetryConfig,
    client: &mut Client,
    request: PublishLifecycleEventRequest,
    endpoint: &str,
    invocation_id: &str,
) -> Result<(), SinkError> {
    dbg(
        endpoint,
        invocation_id,
        &format!("sending lifecycle: {name}"),
    );
    let t = std::time::Instant::now();
    retry_lifecycle(retry, client, request).await.map_err(|e| {
        dbg(
            endpoint,
            invocation_id,
            &format!("{name} FAILED after {:?}: {e}", t.elapsed()),
        );
        finalize(endpoint, format!("{name}: {e}"))
    })?;
    dbg(
        endpoint,
        invocation_id,
        &format!("{name} ack'd in {:?}", t.elapsed()),
    );
    Ok(())
}

/// Per-sink stream state that survives across `drive_stream` reconnect
/// attempts: the unacked-event replay buffer plus the sequence/ack counters
/// every attempt shares.
struct StreamState {
    buffer: RetryBuffer,
    /// Next unused sequence number (BES streams are 1-based).
    next_seq: i64,
    /// Highest sequence number the server has acked.
    max_acked: i64,
    /// Whether the event flagged `last_message` has been written to a stream.
    last_message_sent: bool,
}

async fn work(
    recv: Subscriber<BuildEvent>,
    endpoint: String,
    headers: HashMap<String, String>,
    invocation_id: String,
    retry: RetryConfig,
) -> (SinkStats, SinkOutcome) {
    dbg(
        &endpoint,
        &invocation_id,
        &format!(
            "starting work (headers={}, max_retries={}, timeout={:?})",
            headers.len(),
            retry.max_retries,
            retry.timeout,
        ),
    );

    // Forward the synchronous broadcaster `recv` into a tokio mpsc so
    // `drive_stream` can pull from `event_rx` inside `tokio::select!`.
    // The forwarder runs once for the whole sink lifetime — events
    // queue up on `event_rx` even when no bidi stream is open, ready
    // for the next `drive_stream` iteration to drain.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BuildEvent>();
    let _forwarder = tokio::task::spawn_blocking(move || {
        while let Ok(ev) = recv.recv() {
            if event_tx.send(ev).is_err() {
                break;
            }
        }
    });

    // `Client::new` only builds a lazy channel (endpoint URI + TLS config); it
    // does not dial, so a failure here is a client misconfiguration, not an
    // unreachable backend — the real connect/auth errors surface on the first
    // lifecycle RPC below. The timeout guards against a pathological stall in
    // channel construction.
    const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    let connect_started = std::time::Instant::now();
    let mut client =
        match tokio::time::timeout(CONNECT_TIMEOUT, Client::new(endpoint.clone(), headers)).await {
            Ok(Ok(client)) => client,
            Ok(Err(e)) => {
                return (
                    SinkStats::default(),
                    Err(fail(
                        &endpoint,
                        "invalid backend configuration, build events will not be delivered",
                        format!("client setup failed: {e}"),
                    )),
                );
            }
            Err(_) => {
                let secs = CONNECT_TIMEOUT.as_secs();
                return (
                    SinkStats::default(),
                    Err(fail(
                        &endpoint,
                        &format!(
                            "client setup stalled for {secs}s, build events will not be delivered"
                        ),
                        format!("client setup timed out after {secs}s"),
                    )),
                );
            }
        };
    dbg(
        &endpoint,
        &invocation_id,
        &format!(
            "Client::new returned in {:?} (lazy channel — no TCP/TLS yet)",
            connect_started.elapsed()
        ),
    );

    let build_id = invocation_id.clone();

    if let Err(e) = send_lifecycle_logged(
        "build_enqueued",
        &retry,
        &mut client,
        lifecycle::build_enqueued(build_id.clone(), invocation_id.clone()),
        &endpoint,
        &invocation_id,
    )
    .await
    {
        return (SinkStats::default(), Err(e));
    }
    if let Err(e) = send_lifecycle_logged(
        "invocation_started",
        &retry,
        &mut client,
        lifecycle::invocation_started(build_id.clone(), invocation_id.clone()),
        &endpoint,
        &invocation_id,
    )
    .await
    {
        return (SinkStats::default(), Err(e));
    }

    let mut state = StreamState {
        buffer: RetryBuffer::new(retry.retry_max_buffer_size),
        next_seq: 1,
        max_acked: 0,
        last_message_sent: false,
    };
    let mut attempt: u32 = 0;
    // Warn at most once per sink when the stream first drops into retry, so a
    // flapping or slow backend produces an early user-facing signal without a
    // line per retry attempt.
    let mut warned_transient = false;

    'reconnect: loop {
        dbg(
            &endpoint,
            &invocation_id,
            &format!(
                "entering drive_stream (attempt={}, next_seq={}, buffered={})",
                attempt,
                state.next_seq,
                state.buffer.len()
            ),
        );
        let drive_started = std::time::Instant::now();
        let outcome = drive_stream(
            &mut client,
            &build_id,
            &invocation_id,
            &mut event_rx,
            &mut state,
            &endpoint,
            &retry,
        )
        .await;
        dbg(
            &endpoint,
            &invocation_id,
            &format!(
                "drive_stream returned after {:?}: outcome={}",
                drive_started.elapsed(),
                outcome.label(),
            ),
        );

        match outcome {
            DriveOutcome::Done | DriveOutcome::UpstreamClosed => break 'reconnect,
            DriveOutcome::Transient(err) => {
                if attempt >= retry.max_retries {
                    let stats = SinkStats::from_counters(state.next_seq, state.max_acked);
                    return (
                        stats,
                        Err(finalize(
                            &endpoint,
                            format!("giving up after {attempt} attempts: {err}"),
                        )),
                    );
                }
                if !warned_transient {
                    warn(
                        &endpoint,
                        &format!(
                            "backend unreachable, retrying (up to {} times): {err}",
                            retry.max_retries
                        ),
                    );
                    warned_transient = true;
                }
                let delay = backoff(retry.retry_min_delay, attempt);
                dbg(
                    &endpoint,
                    &invocation_id,
                    &format!("backoff {:?} before retry {}", delay, attempt + 1),
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue 'reconnect;
            }
            DriveOutcome::Fatal(err) => {
                let stats = SinkStats::from_counters(state.next_seq, state.max_acked);
                return (
                    stats,
                    Err(finalize(&endpoint, format!("non-retryable: {err}"))),
                );
            }
            DriveOutcome::BufferFull(err) => {
                let stats = SinkStats::from_counters(state.next_seq, state.max_acked);
                return (stats, Err(finalize(&endpoint, err.to_string())));
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
    // Bazel's exit code lives on the parent process and isn't reachable
    // from here, so we submit a successful BuildStatus unconditionally.
    let post_stream = async {
        send_lifecycle_logged(
            "invocation_finished",
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
            &endpoint,
            &invocation_id,
        )
        .await?;
        send_lifecycle_logged(
            "build_finished",
            &retry,
            &mut client,
            lifecycle::build_finished(build_id.clone(), invocation_id.clone()),
            &endpoint,
            &invocation_id,
        )
        .await?;
        Ok::<(), SinkError>(())
    };
    let outcome = match retry.timeout {
        Some(d) => match tokio::time::timeout(d, post_stream).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(finalize(
                &endpoint,
                format!("post-stream lifecycle exceeded {d:?} budget"),
            )),
        },
        None => post_stream.await,
    };
    let stats = SinkStats::from_counters(state.next_seq, state.max_acked);
    (stats, outcome)
}

/// Bound for the `event_rx.recv()` wait inside `preload_first_event`.
/// Bazel emits `build_started` very early after JVM startup; if nothing
/// arrives within this window we proceed to open the bidi anyway and
/// let the stream-open timeout bound the worst case.
const FIRST_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Push one message into the request-side mpsc channel before
/// `drive_stream` awaits `publish_build_tool_event_stream`.
///
/// The Aspect Workflows BES backend (and several other BES backends)
/// defers sending response headers until it sees the first client
/// message on the bidi stream. Without a message in the channel, tonic's
/// `await` blocks indefinitely on the response-header read and never
/// returns. Pushing a message into the channel here is sufficient —
/// tonic spawns a request-pump task at the start of the bidi call that
/// pulls from the channel and writes to the wire concurrently with the
/// response-header await, so as soon as the HTTP/2 stream is open our
/// event flows out and the server responds.
///
/// Source of the first message:
///   - **Reconnect** (`state.buffer` non-empty): replay its first entry.
///   - **Fresh attempt** (buffer empty): wait up to `FIRST_EVENT_TIMEOUT`
///     for the first event on `event_rx`. If none arrives, return `None`
///     and let the caller open the bidi without pre-load (the stream-open
///     timeout in `drive_stream` still bounds the worst case).
///
/// Returns the sequence number of the pre-loaded entry so `drive_stream`'s
/// post-open replay loop can skip it (avoiding a wasted re-send; the server
/// would dedup anyway).
async fn preload_first_event(
    sender: &tokio::sync::mpsc::Sender<PublishBuildToolEventStreamRequest>,
    build_id: &str,
    invocation_id: &str,
    endpoint: &str,
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<BuildEvent>,
    state: &mut StreamState,
) -> Result<Option<i64>, DriveOutcome> {
    if let Some((seq, req)) = state.buffer.iter().next() {
        let seq = *seq;
        let req = req.clone();
        if sender.send(req).await.is_err() {
            return Err(DriveOutcome::Transient(ClientError::Status(
                tonic::Status::unavailable("request stream closed before bidi open"),
            )));
        }
        dbg(
            endpoint,
            invocation_id,
            &format!("pre-loaded first event from buffer (seq={seq})"),
        );
        return Ok(Some(seq));
    }

    if state.last_message_sent {
        return Ok(None);
    }

    let started = std::time::Instant::now();
    match tokio::time::timeout(FIRST_EVENT_TIMEOUT, event_rx.recv()).await {
        Ok(Some(event)) => {
            let seq = state.next_seq;
            state.next_seq += 1;
            let last = event.last_message;
            let req = build_tool::bazel_event(
                build_id.to_string(),
                invocation_id.to_string(),
                seq,
                &event,
            );
            if let Err(overflow) = state.buffer.push(seq, req.clone()) {
                return Err(DriveOutcome::BufferFull(overflow));
            }
            if sender.send(req).await.is_err() {
                return Err(DriveOutcome::Transient(ClientError::Status(
                    tonic::Status::unavailable("request stream closed before bidi open"),
                )));
            }
            if last {
                state.last_message_sent = true;
            }
            dbg(
                endpoint,
                invocation_id,
                &format!(
                    "pre-loaded first event from event_rx (seq={seq} last_message={last}) in {:?}",
                    started.elapsed()
                ),
            );
            Ok(Some(seq))
        }
        Ok(None) => {
            dbg(
                endpoint,
                invocation_id,
                "event_rx closed before producing first event — opening bidi without pre-load",
            );
            Ok(None)
        }
        Err(_) => {
            dbg(
                endpoint,
                invocation_id,
                &format!(
                    "no event from event_rx within {FIRST_EVENT_TIMEOUT:?} — opening bidi without pre-load"
                ),
            );
            Ok(None)
        }
    }
}

/// Sleep until an optional deadline; pends forever when `None`. For
/// `tokio::select!` arms guarded on the deadline being armed.
async fn sleep_opt(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending::<()>().await,
    }
}

/// Send one request into the bidi request channel, bounding the wait.
///
/// The channel only backs up when tonic's request pump stops pulling —
/// i.e. the server stopped reading and the HTTP/2 flow-control windows
/// are full (hung backend, or a dead connection the keepalive hasn't
/// killed yet). An unbounded `send().await` there would suspend
/// `drive_stream`'s select loop forever; instead surface a Transient
/// outcome so the outer retry loop tears the stream down and replays.
async fn send_bounded(
    sender: &tokio::sync::mpsc::Sender<PublishBuildToolEventStreamRequest>,
    req: PublishBuildToolEventStreamRequest,
    stall_timeout: std::time::Duration,
    context: &str,
) -> Result<(), DriveOutcome> {
    match tokio::time::timeout(stall_timeout, sender.send(req)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(DriveOutcome::Transient(ClientError::Status(
            tonic::Status::unavailable(format!("request stream closed {context}")),
        ))),
        Err(_) => Err(DriveOutcome::Transient(ClientError::Status(
            tonic::Status::deadline_exceeded(format!(
                "request stream stalled for {stall_timeout:?} {context} (server stopped reading)"
            )),
        ))),
    }
}

/// Drive a single bidi stream until it ends (cleanly or with error).
///
/// Owns the request channel for this connection. Pulls events from
/// `event_rx`, assigns sequence numbers, buffers them in `state.buffer`,
/// and writes them down the wire. In parallel reads responses and prunes
/// the buffer up to each ack'd `sequence_number`. Replays any leftover
/// buffered events under their original seqs at the start of the
/// connection (skipping the one already pre-loaded by
/// `preload_first_event`).
///
/// Every wait is bounded so a hung or silently-dropped connection can
/// never wedge the sink: stream open (`OPEN_TIMEOUT`), each write into
/// the request channel (`retry.send_stall_timeout`), ack progress while
/// unacked events are outstanding (`retry.ack_progress_timeout`), and
/// the post-half-close drain (`retry.half_close_timeout`).
async fn drive_stream(
    client: &mut Client,
    build_id: &str,
    invocation_id: &str,
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<BuildEvent>,
    state: &mut StreamState,
    endpoint: &str,
    retry: &RetryConfig,
) -> DriveOutcome {
    let (sender, receiver) = tokio::sync::mpsc::channel::<PublishBuildToolEventStreamRequest>(64);
    let request_stream = ReceiverStream::new(receiver);

    let preloaded_seq = match preload_first_event(
        &sender,
        build_id,
        invocation_id,
        endpoint,
        event_rx,
        state,
    )
    .await
    {
        Ok(seq) => seq,
        Err(outcome) => return outcome,
    };

    // Bound the bidi stream open: tonic does not add a deadline and the
    // server may accept the TCP/TLS handshake without ever responding. With
    // the pre-load above the open succeeds promptly in practice; the timeout
    // bounds the worst case.
    const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    dbg(
        endpoint,
        invocation_id,
        "opening publish_build_tool_event_stream bidi (10s timeout)",
    );
    let open_started = std::time::Instant::now();
    let mut response_stream = match tokio::time::timeout(
        OPEN_TIMEOUT,
        client.publish_build_tool_event_stream(request_stream),
    )
    .await
    {
        Ok(Ok(s)) => {
            dbg(
                endpoint,
                invocation_id,
                &format!(
                    "publish_build_tool_event_stream returned response stream in {:?}",
                    open_started.elapsed()
                ),
            );
            s.into_inner()
        }
        Ok(Err(e)) => {
            dbg(
                endpoint,
                invocation_id,
                &format!(
                    "publish_build_tool_event_stream returned error in {:?}: {e}",
                    open_started.elapsed()
                ),
            );
            return if is_retryable(&e) {
                DriveOutcome::Transient(e)
            } else {
                DriveOutcome::Fatal(e)
            };
        }
        Err(_) => {
            dbg(
                endpoint,
                invocation_id,
                &format!(
                    "publish_build_tool_event_stream stream-open TIMED OUT after {:?}",
                    open_started.elapsed()
                ),
            );
            return DriveOutcome::Transient(ClientError::Status(tonic::Status::deadline_exceeded(
                "stream open timed out",
            )));
        }
    };
    dbg(
        endpoint,
        invocation_id,
        &format!(
            "replaying {} buffered events (skipping pre-loaded seq={:?})",
            state.buffer.len(),
            preloaded_seq
        ),
    );

    // Replay any buffered (unacked) events under their original sequence
    // numbers. The server dedups via OrderedBuildEvent.sequence_number.
    // Skip the entry `preload_first_event` already sent — server would
    // dedup anyway, but resending wastes bytes.
    for (seq, req) in state.buffer.iter() {
        if Some(*seq) == preloaded_seq {
            continue;
        }
        if let Err(outcome) = send_bounded(
            &sender,
            req.clone(),
            retry.send_stall_timeout,
            "during replay",
        )
        .await
        {
            return outcome;
        }
    }

    let mut sender_opt = Some(sender);

    // Ack-progress watchdog: armed while unacked events are outstanding and
    // the request side is still open, reset on every ack. Catches a backend
    // that keeps the connection alive but silently stops acking mid-stream —
    // otherwise that case is only detected at buffer overflow, or never.
    // Once half-closed, `half_close_deadline` governs instead.
    let mut ack_deadline: Option<tokio::time::Instant> = (!state.buffer.is_empty())
        .then(|| tokio::time::Instant::now() + retry.ack_progress_timeout);

    // Hard deadline for waiting after the request side is half-closed
    // (`sender_opt` dropped: last_message sent or upstream broadcaster
    // closed), when the server is supposed to ack any pending events and
    // close the response stream. See `RetryConfig::half_close_timeout`.
    let mut half_close_deadline: Option<tokio::time::Instant> = None;

    // If a previous attempt already sent last_message, the pre-load
    // and replay above resent it on this fresh stream. The
    // event-receive arm below stays disabled (last_message_sent is
    // set), so the `if last { ... }` branch that normally drops the
    // sender will never fire on this attempt. Close the request side
    // now and arm the half-close deadline ourselves — otherwise
    // drive_stream parks in response_stream.next() forever waiting
    // for an ack or close that a flaky backend may never send, and
    // Build.wait() hangs.
    if state.last_message_sent {
        sender_opt = None;
        half_close_deadline = Some(tokio::time::Instant::now() + retry.half_close_timeout);
    }

    loop {
        tokio::select! {
            // Pulling new events from the upstream broadcaster.
            // Disabled once last_message has been sent.
            ev = event_rx.recv(), if !state.last_message_sent && sender_opt.is_some() => {
                let Some(event) = ev else {
                    // Upstream closed without a final last_message. Drop the
                    // request side so the server flushes any pending acks,
                    // and wait for the response stream to wind down.
                    sender_opt = None;
                    if state.buffer.is_empty() {
                        return DriveOutcome::UpstreamClosed;
                    }
                    half_close_deadline.get_or_insert_with(|| {
                        tokio::time::Instant::now() + retry.half_close_timeout
                    });
                    continue;
                };

                let seq = state.next_seq;
                state.next_seq += 1;
                let last = event.last_message;
                let req = build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    seq,
                    &event,
                );

                if let Err(overflow) = state.buffer.push(seq, req.clone()) {
                    return DriveOutcome::BufferFull(overflow);
                }
                ack_deadline.get_or_insert_with(|| {
                    tokio::time::Instant::now() + retry.ack_progress_timeout
                });

                let s = sender_opt.as_ref().unwrap();
                if let Err(outcome) =
                    send_bounded(s, req, retry.send_stall_timeout, "mid-send").await
                {
                    return outcome;
                }

                if last {
                    state.last_message_sent = true;
                    dbg(endpoint, invocation_id, "last_message sent — half-closing");
                    // Drop the request side to signal half-close, but stay
                    // in the loop so we keep draining acks until the server
                    // closes the response side.
                    sender_opt = None;
                    half_close_deadline.get_or_insert_with(|| {
                        tokio::time::Instant::now() + retry.half_close_timeout
                    });
                }
            }
            // Reading server acks from the bidi response stream.
            resp = response_stream.next() => {
                match resp {
                    Some(Ok(r)) => {
                        state.buffer.prune_until(r.sequence_number);
                        state.max_acked = state.max_acked.max(r.sequence_number);
                        ack_deadline = (!state.buffer.is_empty())
                            .then(|| tokio::time::Instant::now() + retry.ack_progress_timeout);
                        // Once we've sent last_message AND every event we
                        // sent has been ack'd, we're done — exit without
                        // waiting for the server to close the response
                        // stream. Some BES backends keep the response
                        // stream open after the final ack, which previously
                        // caused this loop to hang indefinitely on every
                        // successful upload.
                        if state.last_message_sent && state.buffer.is_empty() {
                            return DriveOutcome::Done;
                        }
                        if sender_opt.is_none() && state.buffer.is_empty() {
                            return DriveOutcome::UpstreamClosed;
                        }
                    }
                    Some(Err(status)) => {
                        dbg(
                            endpoint,
                            invocation_id,
                            &format!("server returned error status: {status}"),
                        );
                        let err = ClientError::Status(status);
                        return if is_retryable(&err) {
                            DriveOutcome::Transient(err)
                        } else {
                            DriveOutcome::Fatal(err)
                        };
                    }
                    None => {
                        dbg(
                            endpoint,
                            invocation_id,
                            &format!(
                                "response stream closed (last_message_sent={}, buffered={})",
                                state.last_message_sent,
                                state.buffer.len()
                            ),
                        );
                        // Server closed the response stream.
                        if state.last_message_sent && state.buffer.is_empty() {
                            return DriveOutcome::Done;
                        }
                        if sender_opt.is_none() && state.buffer.is_empty() {
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
            //     outer loop converts to a terminal SinkError that the
            //     caller's `sink.wait()` surfaces. Returning Done here
            //     would silently drop unacked events.
            _ = sleep_opt(half_close_deadline), if half_close_deadline.is_some() => {
                if !state.buffer.is_empty() {
                    return DriveOutcome::Transient(ClientError::Status(
                        tonic::Status::deadline_exceeded(format!(
                            "half-close deadline ({:?}) elapsed with {} events unacked",
                            retry.half_close_timeout,
                            state.buffer.len()
                        )),
                    ));
                }
                return if state.last_message_sent {
                    DriveOutcome::Done
                } else {
                    DriveOutcome::UpstreamClosed
                };
            }
            // Ack-progress watchdog arm (see `ack_deadline`). Disabled once
            // the request side is half-closed — `half_close_deadline` bounds
            // that phase with a tighter deadline.
            _ = sleep_opt(ack_deadline), if ack_deadline.is_some() && half_close_deadline.is_none() => {
                dbg(
                    endpoint,
                    invocation_id,
                    &format!(
                        "no ack progress within {:?} ({} events unacked) — reconnecting",
                        retry.ack_progress_timeout,
                        state.buffer.len()
                    ),
                );
                return DriveOutcome::Transient(ClientError::Status(
                    tonic::Status::deadline_exceeded(format!(
                        "no BES ack progress within {:?} ({} events unacked)",
                        retry.ack_progress_timeout,
                        state.buffer.len()
                    )),
                ));
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

/// Terminate the sink: warn the user (with `user_msg`) that events won't be
/// delivered, and return the `SinkError` carrying the machine-readable
/// `last_error` cause. The two strings differ when the user-facing phrasing
/// should read better than the raw cause (e.g. client-setup failures).
fn fail(endpoint: &str, user_msg: &str, last_error: String) -> SinkError {
    warn(endpoint, user_msg);
    SinkError { last_error }
}

/// [`fail`] for the give-up cases, where the raw cause is also fit to show the
/// user directly.
fn finalize(endpoint: &str, last_error: String) -> SinkError {
    fail(
        endpoint,
        &format!("giving up, build events were not delivered: {last_error}"),
        last_error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::pin::Pin;
    use std::time::Duration;

    use axl_proto::build_event_stream::{Progress, build_event::Payload};
    use axl_proto::google::devtools::build::v1::{
        PublishBuildToolEventStreamResponse,
        publish_build_event_server::{PublishBuildEvent, PublishBuildEventServer},
    };
    use futures::Stream;
    use tonic::{Request, Response, Status, Streaming};

    /// How the in-process BES server treats the bidi stream.
    #[derive(Clone, Copy)]
    enum ServerMode {
        /// Hold the request stream open without ever polling it: HTTP/2
        /// flow-control credit is never returned and the client's writes
        /// stall (a hung backend, or a peer that died without closing the
        /// connection). Never acks, never closes the response stream.
        Stall,
        /// Read and discard every request; never ack, never close the
        /// response stream.
        DrainNoAck,
        /// Well-behaved: ack every request's sequence number.
        Ack,
    }

    struct TestServer {
        mode: ServerMode,
    }

    #[tonic::async_trait]
    impl PublishBuildEvent for TestServer {
        async fn publish_lifecycle_event(
            &self,
            _request: Request<PublishLifecycleEventRequest>,
        ) -> Result<Response<()>, Status> {
            Ok(Response::new(()))
        }

        type PublishBuildToolEventStreamStream =
            Pin<Box<dyn Stream<Item = Result<PublishBuildToolEventStreamResponse, Status>> + Send>>;

        async fn publish_build_tool_event_stream(
            &self,
            request: Request<Streaming<PublishBuildToolEventStreamRequest>>,
        ) -> Result<Response<Self::PublishBuildToolEventStreamStream>, Status> {
            let mut inbound = request.into_inner();
            match self.mode {
                ServerMode::Stall | ServerMode::DrainNoAck => {
                    let drain = matches!(self.mode, ServerMode::DrainNoAck);
                    tokio::spawn(async move {
                        if drain {
                            while inbound.next().await.is_some() {}
                        }
                        // Park forever still owning `inbound`: the request
                        // stream must stay open (and unread in Stall mode),
                        // never reset from the server side.
                        let _hold = inbound;
                        std::future::pending::<()>().await;
                    });
                    Ok(Response::new(Box::pin(futures::stream::pending())))
                }
                ServerMode::Ack => {
                    let (ack_tx, ack_rx) = tokio::sync::mpsc::channel(16);
                    tokio::spawn(async move {
                        while let Some(Ok(req)) = inbound.next().await {
                            let seq = req.ordered_build_event.map_or(0, |e| e.sequence_number);
                            let ack = PublishBuildToolEventStreamResponse {
                                stream_id: None,
                                sequence_number: seq,
                            };
                            if ack_tx.send(Ok(ack)).await.is_err() {
                                break;
                            }
                        }
                    });
                    Ok(Response::new(Box::pin(ReceiverStream::new(ack_rx))))
                }
            }
        }
    }

    async fn start_server(mode: ServerMode) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(PublishBuildEventServer::new(TestServer { mode }))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
        );
        format!("http://{addr}")
    }

    fn progress_event(stderr_bytes: usize) -> BuildEvent {
        BuildEvent {
            payload: Some(Payload::Progress(Progress {
                stderr: "x".repeat(stderr_bytes),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    /// Run `drive_stream` against a [`TestServer`] and return its outcome,
    /// panicking if it fails to complete within the test bound (i.e. a stall
    /// went undetected).
    async fn drive_against(
        endpoint: String,
        events: Vec<BuildEvent>,
        retry: RetryConfig,
    ) -> DriveOutcome {
        let mut client = Client::new(endpoint, HashMap::new()).await.unwrap();
        let (tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BuildEvent>();
        for ev in events {
            tx.send(ev).unwrap();
        }
        // `tx` stays alive across the call: upstream must look open, since a
        // closed upstream arms the (already bounded) half-close path instead.
        let mut state = StreamState {
            buffer: RetryBuffer::new(retry.retry_max_buffer_size),
            next_seq: 1,
            max_acked: 0,
            last_message_sent: false,
        };
        let outcome = tokio::time::timeout(
            Duration::from_secs(30),
            drive_stream(
                &mut client,
                "test-build",
                "test-invocation",
                &mut event_rx,
                &mut state,
                "test-endpoint",
                &retry,
            ),
        )
        .await
        .expect("drive_stream did not complete within the test bound");
        drop(tx);
        outcome
    }

    /// Server accepts the stream then never reads: flow-control windows and
    /// the request channel fill, and the bounded send must surface a
    /// Transient outcome instead of suspending the loop forever.
    #[tokio::test(flavor = "multi_thread")]
    async fn send_stall_surfaces_transient() {
        let endpoint = start_server(ServerMode::Stall).await;
        let retry = RetryConfig {
            send_stall_timeout: Duration::from_millis(300),
            // Keep the ack watchdog out of the way so the send path is what
            // trips first.
            ack_progress_timeout: Duration::from_secs(60),
            ..Default::default()
        };
        // 128 KiB events overrun the HTTP/2 window quickly; 100 of them is
        // far more than window + request-channel capacity (64).
        let events = (0..100).map(|_| progress_event(128 * 1024)).collect();
        match drive_against(endpoint, events, retry).await {
            DriveOutcome::Transient(ClientError::Status(s)) => {
                assert!(s.message().contains("stalled"), "unexpected status: {s}");
            }
            other => panic!("expected Transient stall, got {}", other.label()),
        }
    }

    /// Server reads every event but never acks: the ack-progress watchdog
    /// must surface a Transient outcome rather than waiting for buffer
    /// overflow (or forever).
    #[tokio::test(flavor = "multi_thread")]
    async fn ack_silence_surfaces_transient() {
        let endpoint = start_server(ServerMode::DrainNoAck).await;
        let retry = RetryConfig {
            send_stall_timeout: Duration::from_secs(60),
            ack_progress_timeout: Duration::from_millis(300),
            ..Default::default()
        };
        let events = (0..5).map(|_| progress_event(64)).collect();
        match drive_against(endpoint, events, retry).await {
            DriveOutcome::Transient(ClientError::Status(s)) => {
                assert!(
                    s.message().contains("no BES ack progress"),
                    "unexpected status: {s}"
                );
            }
            other => panic!("expected Transient ack timeout, got {}", other.label()),
        }
    }

    /// Server ends up holding unacked events when the build finishes
    /// (`last_message` sent, request side half-closed): the half-close
    /// deadline must bound the wait for outstanding acks.
    #[tokio::test(flavor = "multi_thread")]
    async fn half_close_with_unacked_events_surfaces_transient() {
        let endpoint = start_server(ServerMode::DrainNoAck).await;
        let retry = RetryConfig {
            half_close_timeout: Duration::from_millis(300),
            ..Default::default()
        };
        let mut events: Vec<BuildEvent> = (0..5).map(|_| progress_event(64)).collect();
        events.last_mut().unwrap().last_message = true;
        match drive_against(endpoint, events, retry).await {
            DriveOutcome::Transient(ClientError::Status(s)) => {
                assert!(
                    s.message().contains("half-close deadline"),
                    "unexpected status: {s}"
                );
            }
            other => panic!(
                "expected Transient half-close timeout, got {}",
                other.label()
            ),
        }
    }

    /// Well-behaved server acking every event: the stream completes `Done`
    /// after `last_message` with no deadline firing spuriously.
    #[tokio::test(flavor = "multi_thread")]
    async fn acked_stream_completes_done() {
        let endpoint = start_server(ServerMode::Ack).await;
        let mut events: Vec<BuildEvent> = (0..5).map(|_| progress_event(64)).collect();
        events.last_mut().unwrap().last_message = true;
        match drive_against(endpoint, events, RetryConfig::default()).await {
            DriveOutcome::Done => {}
            other => panic!("expected Done, got {}", other.label()),
        }
    }

    #[test]
    fn fail_preserves_machine_cause_independent_of_user_message() {
        // The user-facing phrasing and the machine-readable cause are allowed to
        // differ; `SinkError` must carry the latter verbatim for callers that
        // inspect `sink.error`.
        let err = fail(
            "grpcs://bes.example.com",
            "could not connect, build events will not be delivered",
            "connect failed: transport error".to_string(),
        );
        assert_eq!(err.last_error, "connect failed: transport error");
    }

    #[test]
    fn finalize_carries_cause_in_sink_error() {
        let err = finalize("grpcs://bes.example.com", "deadline exceeded".to_string());
        assert_eq!(err.last_error, "deadline exceeded");
    }
}
