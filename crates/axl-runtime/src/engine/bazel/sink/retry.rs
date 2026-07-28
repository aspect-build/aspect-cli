//! Retry / backoff machinery for the gRPC BES sink.
//!
//! Mirrors Bazel's `BuildEventServiceUploader`: bounded retry budget with
//! full-jitter exponential backoff and an in-flight buffer for replay across
//! reconnects. Ack progress during an attempt resets the budget, so only
//! consecutive no-progress attempts count against it and a server that
//! periodically ends streams with a retryable status (e.g. UNAVAILABLE)
//! is rejoined for as long as events keep landing.
//! Terminal failures are surfaced via the sink's outcome — the
//! caller decides what to do (warn, fail the task, etc.); the runtime never
//! tries to second-guess the policy.
//!
//! The replay buffer is bounded by bytes and evicts its oldest entries when
//! full; a slow-acking backend degrades replay coverage but never terminates
//! the upload. See [`RetryBuffer`].

use std::collections::VecDeque;
use std::time::Duration;

use axl_proto::google::devtools::build::v1::PublishBuildToolEventStreamRequest;
use build_event_stream::client::ClientError;
use prost::Message;
use rand::Rng;

/// Default byte budget for the unacked replay buffer (256 MiB). Sized to hold
/// a long stream of ordinary events (hundreds of thousands, at a few hundred
/// bytes each) plus a few of the multi-megabyte outliers that carry action
/// stdout, so eviction stays rare in practice.
pub const DEFAULT_RETRY_MAX_BUFFER_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Consecutive reconnect attempts without ack progress before the sink
    /// gives up. An attempt during which the server acked anything resets
    /// the count.
    pub max_retries: u32,
    pub retry_min_delay: Duration,
    /// Byte budget for the unacked replay buffer. Exceeding it evicts the
    /// oldest retained events rather than failing the stream — see
    /// [`RetryBuffer`].
    pub retry_max_buffer_bytes: usize,
    pub timeout: Option<Duration>,
    /// How long a single write into the bidi request stream may block before
    /// the connection is declared stalled (server stopped reading; HTTP/2
    /// flow-control windows and the request channel are full) and the stream
    /// is torn down for a retry. Not exposed to Starlark; overridable in
    /// tests.
    pub send_stall_timeout: Duration,
    /// How long the server may go without acking while unacked events are
    /// outstanding (pre-half-close) before the stream is torn down for a
    /// retry. Not exposed to Starlark; overridable in tests.
    pub ack_progress_timeout: Duration,
    /// How long to wait, after the request side half-closes, for the server
    /// to ack outstanding events and close the response stream. Some
    /// backends sit on a half-closed stream without acking or closing;
    /// without this bound the sink thread — and the build's `sink.wait()` —
    /// would hang. Not exposed to Starlark; overridable in tests.
    pub half_close_timeout: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 4,
            retry_min_delay: Duration::from_secs(1),
            retry_max_buffer_bytes: DEFAULT_RETRY_MAX_BUFFER_BYTES,
            timeout: None,
            send_stall_timeout: Duration::from_secs(60),
            ack_progress_timeout: Duration::from_secs(120),
            half_close_timeout: Duration::from_secs(30),
        }
    }
}

/// Parse a duration string like `"1s"`, `"500ms"`, `"2m"`, `"1h"`, `"1d"`,
/// `"0s"`.
///
/// Accepted suffixes mirror Bazel's `--bes_timeout`: `ms`, `s`, `m`, `h`, `d`.
///
/// `"0s"` (or any zero value) is the documented sentinel for "no deadline"
/// when used as a timeout; the caller decides what zero means.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".into());
    }
    let (num_str, unit) = if let Some(rest) = s.strip_suffix("ms") {
        (rest, "ms")
    } else if let Some(rest) = s.strip_suffix('s') {
        (rest, "s")
    } else if let Some(rest) = s.strip_suffix('m') {
        (rest, "m")
    } else if let Some(rest) = s.strip_suffix('h') {
        (rest, "h")
    } else if let Some(rest) = s.strip_suffix('d') {
        (rest, "d")
    } else {
        return Err(format!(
            "invalid duration '{s}': expected suffix one of 'ms', 's', 'm', 'h', 'd'"
        ));
    };
    let n: u64 = num_str
        .trim()
        .parse()
        .map_err(|e| format!("invalid duration '{s}': {e}"))?;
    Ok(match unit {
        "ms" => Duration::from_millis(n),
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        "h" => Duration::from_secs(n * 3600),
        "d" => Duration::from_secs(n * 86_400),
        _ => unreachable!(),
    })
}

/// Byte-bounded ring of unacked stream events keyed by their original sequence
/// number. On reconnect the entire buffer is replayed before fresh events
/// resume — the BES protocol's per-stream sequence-number dedup makes this
/// safe even if the server already saw some of the replayed events.
///
/// The bound is a byte budget, not an event count, because BEP event sizes span
/// orders of magnitude: most events are a few hundred bytes, but an action's
/// captured stdout can reach tens of megabytes. A count-based cap therefore
/// bounds neither memory (a handful of large events blows past any reasonable
/// footprint) nor stream length (hundreds of thousands of small events are
/// cheap), so it fires on builds that are perfectly healthy while failing to
/// protect against the ones that actually consume memory.
///
/// Overflow is not an error. Retaining an event serves exactly one purpose —
/// replaying it if the stream reconnects — so when the budget is exceeded the
/// oldest entries are evicted to make room. The only consequence is that a
/// subsequent reconnect cannot replay the evicted range; the server tolerates
/// the resulting sequence gap (it acks by position and readers page forward
/// past missing seqs). Delivery of the current stream is unaffected, which is
/// what makes eviction strictly better than tearing the stream down.
pub struct RetryBuffer {
    /// Byte budget for retained (unacked) events.
    cap_bytes: usize,
    /// Sum of `encoded_len()` over `items`, maintained incrementally.
    bytes: usize,
    /// Events evicted to stay under budget — a nonzero count means a reconnect
    /// would replay an incomplete range.
    evicted: u64,
    items: VecDeque<(i64, usize, PublishBuildToolEventStreamRequest)>,
}

impl RetryBuffer {
    pub fn new(cap_bytes: usize) -> Self {
        Self {
            cap_bytes,
            bytes: 0,
            evicted: 0,
            items: VecDeque::new(),
        }
    }

    /// Retain an event for potential replay, evicting the oldest entries when
    /// it would exceed the byte budget. Infallible: the event is always sent
    /// regardless of whether it could be retained.
    ///
    /// An event larger than the whole budget is not retained at all — evicting
    /// everything else to hold one oversized event would forfeit more replay
    /// coverage than it buys.
    pub fn push(&mut self, seq: i64, req: PublishBuildToolEventStreamRequest) {
        let size = req.encoded_len();

        if size > self.cap_bytes {
            self.evict_all();
            self.evicted += 1;
            return;
        }

        while self.bytes + size > self.cap_bytes {
            match self.items.pop_front() {
                Some((_, evicted_size, _)) => {
                    self.bytes -= evicted_size;
                    self.evicted += 1;
                }
                // Unreachable: `size <= cap_bytes` and an empty buffer has
                // `bytes == 0`, so the loop condition is already false.
                None => break,
            }
        }

        self.bytes += size;
        self.items.push_back((seq, size, req));
    }

    fn evict_all(&mut self) {
        self.evicted += self.items.len() as u64;
        self.items.clear();
        self.bytes = 0;
    }

    /// Drop every entry with `seq <= ack_seq`. Called when the server acks a
    /// response on the bidi stream.
    pub fn prune_until(&mut self, ack_seq: i64) {
        while let Some((seq, size, _)) = self.items.front() {
            if *seq <= ack_seq {
                self.bytes -= *size;
                self.items.pop_front();
            } else {
                break;
            }
        }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Retained bytes. Exposed for tests and debug logging.
    #[allow(dead_code)]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// How many events were dropped to stay under budget. Nonzero means a
    /// reconnect replays an incomplete range.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&i64, &PublishBuildToolEventStreamRequest)> {
        self.items.iter().map(|(seq, _, req)| (seq, req))
    }
}

/// Full-jitter exponential backoff. Mirrors Bazel:
///
/// ```text
/// delay = random(0, min(min_delay * 2^attempt, min_delay * 30))
/// ```
pub fn backoff(min_delay: Duration, attempt: u32) -> Duration {
    let cap_ns = (min_delay.as_nanos() as u64).saturating_mul(30);
    let exp = 1u64 << attempt.min(30);
    let upper_ns = (min_delay.as_nanos() as u64)
        .saturating_mul(exp)
        .min(cap_ns);
    if upper_ns == 0 {
        return Duration::from_nanos(0);
    }
    let jitter = rand::thread_rng().gen_range(0..=upper_ns);
    Duration::from_nanos(jitter)
}

/// Whether a `ClientError` should trigger a reconnect attempt (true) or be
/// treated as terminal immediately (false).
pub fn is_retryable(err: &ClientError) -> bool {
    use tonic::Code;
    match err {
        // Transport-level: TLS handshake, h2 protocol error, connection
        // reset — all assumed transient.
        ClientError::Transport(_) => true,
        ClientError::InvalidEndpoint(_) => false,
        ClientError::Status(status) => matches!(
            status.code(),
            Code::Unavailable
                | Code::DeadlineExceeded
                | Code::ResourceExhausted
                | Code::Aborted
                | Code::Internal
        ),
    }
}

/// Terminal failure of a sink. Carries the human-readable description of
/// the underlying error. Implements `Error` via `thiserror` so sink work
/// functions can use `?` and callers can chain it through `anyhow` without
/// ceremony. Surface policy lives in the caller, not on this struct.
#[derive(Debug, thiserror::Error)]
#[error("{last_error}")]
pub struct SinkError {
    pub last_error: String,
}

/// What a sink thread returns. `Ok(())` on clean exit; `Err(SinkError)` when
/// the sink gave up.
pub type SinkOutcome = Result<(), SinkError>;

/// How much a gRPC sink transferred, reported on both clean and failed exits so
/// the end-of-build summary can say how many build events reached the backend.
/// `sent` counts distinct events streamed (deduped across reconnect replays);
/// `acked` counts those the server confirmed (its sequence-number acks are the
/// only delivery signal), so `acked < sent` means events were streamed but not
/// confirmed landed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SinkStats {
    pub sent: u64,
    pub acked: u64,
}

impl SinkStats {
    /// Derive the stats from a forwarder's live counters at an exit point.
    /// `next_seq` is the next unused sequence number (starts at 1, so distinct
    /// events sent is `next_seq - 1`); `max_acked` is the highest sequence the
    /// server confirmed. Both clamp at 0 so a pre-stream exit reports nothing.
    pub fn from_counters(next_seq: i64, max_acked: i64) -> Self {
        SinkStats {
            sent: (next_seq - 1).max(0) as u64,
            acked: max_acked.max(0) as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::google::devtools::build::v1::PublishBuildToolEventStreamRequest;

    fn req() -> PublishBuildToolEventStreamRequest {
        PublishBuildToolEventStreamRequest::default()
    }

    /// A request whose encoded size is at least `n` bytes, for byte-budget
    /// tests. Payload goes in `project_id`, a plain string field.
    fn req_sized(n: usize) -> PublishBuildToolEventStreamRequest {
        PublishBuildToolEventStreamRequest {
            project_id: "x".repeat(n),
            ..Default::default()
        }
    }

    fn seqs(b: &RetryBuffer) -> Vec<i64> {
        b.iter().map(|(s, _)| *s).collect()
    }

    #[test]
    fn sink_stats_from_counters() {
        // Fresh forwarder that never streamed an event (next_seq still 1).
        assert_eq!(
            SinkStats::from_counters(1, 0),
            SinkStats { sent: 0, acked: 0 }
        );
        // Streamed 1284, server acked 812.
        assert_eq!(
            SinkStats::from_counters(1285, 812),
            SinkStats {
                sent: 1284,
                acked: 812
            }
        );
        // Defensive clamp: negative counters (unreachable in practice) report 0.
        assert_eq!(
            SinkStats::from_counters(0, -1),
            SinkStats { sent: 0, acked: 0 }
        );
    }

    /// A high budget retains everything: many small events must not evict,
    /// which is the case a count-based cap used to fail.
    #[test]
    fn buffer_retains_many_small_events() {
        let mut b = RetryBuffer::new(1024 * 1024);
        for i in 1..=20_000 {
            b.push(i, req());
        }
        assert_eq!(b.len(), 20_000);
        assert_eq!(b.evicted(), 0);
    }

    /// Overflow evicts oldest-first and keeps the newest events, rather than
    /// failing.
    #[test]
    fn buffer_evicts_oldest_when_over_budget() {
        let one = req_sized(100).encoded_len();
        let mut b = RetryBuffer::new(one * 3);

        for i in 1..=3 {
            b.push(i, req_sized(100));
        }
        assert_eq!(seqs(&b), vec![1, 2, 3]);
        assert_eq!(b.evicted(), 0);

        // Fourth event exceeds the budget: seq 1 is evicted to make room.
        b.push(4, req_sized(100));
        assert_eq!(seqs(&b), vec![2, 3, 4]);
        assert_eq!(b.evicted(), 1);
        assert!(b.bytes() <= one * 3);
    }

    /// One large event can evict several small ones — eviction is driven by
    /// bytes, not entry count.
    #[test]
    fn buffer_large_event_evicts_multiple_small() {
        let small = req_sized(10).encoded_len();
        let mut b = RetryBuffer::new(small * 10);
        for i in 1..=10 {
            b.push(i, req_sized(10));
        }
        assert_eq!(b.len(), 10);

        b.push(11, req_sized(small * 5));
        assert_eq!(*seqs(&b).last().unwrap(), 11);
        assert!(b.len() < 10, "large event should displace several small ones");
        assert!(b.bytes() <= small * 10);
    }

    /// An event bigger than the entire budget is not retained, and does not
    /// take the rest of the buffer down with it beyond making room.
    #[test]
    fn buffer_oversized_event_is_not_retained() {
        let mut b = RetryBuffer::new(1024);
        b.push(1, req_sized(10));
        b.push(2, req_sized(4096));

        assert!(b.is_empty(), "oversized event must not be retained");
        assert_eq!(b.bytes(), 0);
        assert_eq!(b.evicted(), 2, "the evicted small event and the oversized one");

        // The buffer stays usable afterwards.
        b.push(3, req_sized(10));
        assert_eq!(seqs(&b), vec![3]);
    }

    #[test]
    fn buffer_prune_removes_only_le_ack() {
        let mut b = RetryBuffer::new(1024 * 1024);
        for i in 1..=5 {
            b.push(i, req());
        }
        b.prune_until(3);
        assert_eq!(seqs(&b), vec![4, 5]);
    }

    /// Pruning must return bytes to the budget, or the buffer would evict
    /// spuriously after a long healthy stream.
    #[test]
    fn buffer_prune_reclaims_bytes() {
        let mut b = RetryBuffer::new(1024 * 1024);
        for i in 1..=10 {
            b.push(i, req_sized(100));
        }
        let full = b.bytes();
        assert!(full > 0);

        b.prune_until(10);
        assert!(b.is_empty());
        assert_eq!(b.bytes(), 0, "pruning everything must zero the byte count");
    }

    #[test]
    fn backoff_in_envelope() {
        let min = Duration::from_millis(100);
        for attempt in 0..10 {
            let d = backoff(min, attempt);
            let cap = min * 30;
            assert!(d <= cap, "attempt {attempt}: {d:?} > {cap:?}");
        }
    }

    #[test]
    fn parse_duration_units() {
        assert_eq!(parse_duration("0s").unwrap(), Duration::from_secs(0));
        assert_eq!(parse_duration("250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_duration("3s").unwrap(), Duration::from_secs(3));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86_400));
        assert!(parse_duration("").is_err());
        assert!(parse_duration("10").is_err());
        assert!(parse_duration("abc").is_err());
    }

    #[test]
    fn classifier_status_codes() {
        let unavailable = ClientError::Status(tonic::Status::new(tonic::Code::Unavailable, "x"));
        let unauth = ClientError::Status(tonic::Status::new(tonic::Code::Unauthenticated, "x"));
        let internal = ClientError::Status(tonic::Status::new(tonic::Code::Internal, "x"));
        let perm = ClientError::Status(tonic::Status::new(tonic::Code::PermissionDenied, "x"));
        assert!(is_retryable(&unavailable));
        assert!(is_retryable(&internal));
        assert!(!is_retryable(&unauth));
        assert!(!is_retryable(&perm));
    }
}
