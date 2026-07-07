//! Retry / backoff machinery for the gRPC BES sink.
//!
//! Mirrors Bazel's `BuildEventServiceUploader`: bounded retry budget with
//! full-jitter exponential backoff and an in-flight buffer for replay across
//! reconnects. Terminal failures are surfaced via the sink's outcome — the
//! caller decides what to do (warn, fail the task, etc.); the runtime never
//! tries to second-guess the policy.

use std::collections::VecDeque;
use std::time::Duration;

use axl_proto::google::devtools::build::v1::PublishBuildToolEventStreamRequest;
use build_event_stream::client::ClientError;
use rand::Rng;

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub retry_min_delay: Duration,
    pub retry_max_buffer_size: usize,
    pub timeout: Option<Duration>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 4,
            retry_min_delay: Duration::from_secs(1),
            retry_max_buffer_size: 10_000,
            timeout: None,
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

/// Bounded ring of unacked stream events keyed by their original sequence
/// number. On reconnect the entire buffer is replayed before fresh events
/// resume — the BES protocol's per-stream sequence-number dedup makes this
/// safe even if the server already saw some of the replayed events.
pub struct RetryBuffer {
    cap: usize,
    items: VecDeque<(i64, PublishBuildToolEventStreamRequest)>,
}

impl RetryBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            items: VecDeque::new(),
        }
    }

    /// Push an event into the buffer. Returns `Err` if the buffer is full —
    /// the caller must transition to terminal at that point per the design.
    pub fn push(
        &mut self,
        seq: i64,
        req: PublishBuildToolEventStreamRequest,
    ) -> Result<(), BufferOverflow> {
        if self.items.len() >= self.cap {
            return Err(BufferOverflow { cap: self.cap, seq });
        }
        self.items.push_back((seq, req));
        Ok(())
    }

    /// Drop every entry with `seq <= ack_seq`. Called when the server acks a
    /// response on the bidi stream.
    pub fn prune_until(&mut self, ack_seq: i64) {
        while let Some((seq, _)) = self.items.front() {
            if *seq <= ack_seq {
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

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &(i64, PublishBuildToolEventStreamRequest)> {
        self.items.iter()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("retry buffer overflowed (cap={cap}) while attempting to buffer seq {seq}")]
pub struct BufferOverflow {
    pub cap: usize,
    pub seq: i64,
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

    #[test]
    fn buffer_push_until_cap_then_overflow() {
        let mut b = RetryBuffer::new(2);
        b.push(1, req()).unwrap();
        b.push(2, req()).unwrap();
        let err = b.push(3, req()).unwrap_err();
        assert_eq!(err.cap, 2);
        assert_eq!(err.seq, 3);
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn buffer_prune_removes_only_le_ack() {
        let mut b = RetryBuffer::new(8);
        for i in 1..=5 {
            b.push(i, req()).unwrap();
        }
        b.prune_until(3);
        let seqs: Vec<i64> = b.iter().map(|(s, _)| *s).collect();
        assert_eq!(seqs, vec![4, 5]);
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
