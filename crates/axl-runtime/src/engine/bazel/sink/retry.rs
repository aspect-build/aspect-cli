//! Retry / backoff machinery for the gRPC BES sink.
//!
//! Mirrors Bazel's `BuildEventServiceUploader`: bounded retry budget with
//! full-jitter exponential backoff and an in-flight buffer for replay across
//! reconnects. Ack progress during an attempt resets the budget, so only
//! consecutive no-progress attempts count against it and a server that
//! periodically ends streams with a retryable status (e.g. UNAVAILABLE)
//! is rejoined for as long as events keep landing. The replay buffer is
//! bounded by bytes rather than event count and sheds its oldest entries when
//! full, so a slow-acking backend costs replay coverage but never ends the
//! upload — see [`RetryBuffer`].
//!
//! Terminal failures are surfaced via the sink's outcome — the caller decides
//! what to do (warn, fail the task, etc.); the runtime never tries to
//! second-guess the policy.

use std::collections::VecDeque;
use std::time::Duration;

use axl_proto::google::devtools::build::v1::PublishBuildToolEventStreamRequest;
use build_event_stream::client::ClientError;
use prost::Message;
use rand::Rng;

/// Built-in byte budget for the unacked replay buffer (256 MiB). Sized to hold
/// a long stream of ordinary events (hundreds of thousands, at a few hundred
/// bytes each) plus a few of the multi-megabyte outliers that carry action
/// stdout, so eviction stays rare in practice.
///
/// Overridable per-runner via [`RETRY_MAX_BUFFER_BYTES_ENV`]; prefer
/// [`default_retry_max_buffer_bytes`] over reading this directly.
pub const DEFAULT_RETRY_MAX_BUFFER_BYTES: usize = 256 * 1024 * 1024;

/// Environment override for the replay buffer's byte budget, letting a runner
/// size it to its own memory without threading a flag through every task.
/// Accepts a plain byte count or a size suffix — see [`parse_byte_size`].
///
/// The `CLI` in the name distinguishes this from Bazel's own BES uploader
/// (`--bes_backend`), whose buffering it does not affect.
pub const RETRY_MAX_BUFFER_BYTES_ENV: &str = "ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES";

/// The replay buffer's default byte budget, honoring
/// [`RETRY_MAX_BUFFER_BYTES_ENV`].
///
/// Read once per process and cached, so two sinks in one build cannot disagree
/// if the environment changes mid-run.
pub fn default_retry_max_buffer_bytes() -> usize {
    static RESOLVED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *RESOLVED.get_or_init(|| {
        let (bytes, warning) = resolve_buffer_bytes_override(
            std::env::var(RETRY_MAX_BUFFER_BYTES_ENV).ok().as_deref(),
        );
        if let Some(w) = warning {
            crate::diag::warn(&w);
        }
        bytes
    })
}

/// Resolve the override, returning the budget to use and any warning to emit.
/// Separate from [`default_retry_max_buffer_bytes`] so the rules are testable
/// without touching process-wide state.
///
/// Unset or empty is silent — the common case. A malformed or zero value warns
/// and falls back to the built-in default: BES upload is best-effort, so a
/// typo'd tuning knob should not cost a CI run.
fn resolve_buffer_bytes_override(raw: Option<&str>) -> (usize, Option<String>) {
    let Some(raw) = raw else {
        return (DEFAULT_RETRY_MAX_BUFFER_BYTES, None);
    };
    if raw.trim().is_empty() {
        return (DEFAULT_RETRY_MAX_BUFFER_BYTES, None);
    }
    match parse_byte_size(raw) {
        Ok(0) => (
            DEFAULT_RETRY_MAX_BUFFER_BYTES,
            Some(format!(
                "{RETRY_MAX_BUFFER_BYTES_ENV}={raw} must be > 0; \
                 using the default of {DEFAULT_RETRY_MAX_BUFFER_BYTES} bytes"
            )),
        ),
        Ok(n) => (n, None),
        Err(e) => (
            DEFAULT_RETRY_MAX_BUFFER_BYTES,
            Some(format!(
                "{RETRY_MAX_BUFFER_BYTES_ENV}: {e}; \
                 using the default of {DEFAULT_RETRY_MAX_BUFFER_BYTES} bytes"
            )),
        ),
    }
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Consecutive reconnect attempts without ack progress before the sink
    /// gives up. An attempt during which the server acked anything resets
    /// the count.
    pub max_retries: u32,
    pub retry_min_delay: Duration,
    /// Byte budget for the unacked replay buffer. Exceeding it evicts the
    /// oldest retained events rather than failing the stream — see
    /// [`RetryBuffer`]. Defaults via [`default_retry_max_buffer_bytes`].
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
            retry_max_buffer_bytes: default_retry_max_buffer_bytes(),
            timeout: None,
            send_stall_timeout: Duration::from_secs(60),
            ack_progress_timeout: Duration::from_secs(120),
            half_close_timeout: Duration::from_secs(30),
        }
    }
}

/// Split a suffixed scalar (`"512MB"`, `"250ms"`) into its numeric prefix and
/// lower-cased unit. Surrounding and internal whitespace is ignored; a missing
/// unit yields `""`.
///
/// `kind` names the value in error messages ("size", "duration").
fn split_scalar_unit<'a>(s: &'a str, kind: &str) -> Result<(&'a str, String), String> {
    let s = s.trim();
    if s.is_empty() {
        return Err(format!("empty {kind} string"));
    }
    let digits_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if digits_end == 0 {
        return Err(format!("invalid {kind} '{s}': expected a leading number"));
    }
    let (num, unit) = s.split_at(digits_end);
    Ok((num, unit.trim().to_ascii_lowercase()))
}

/// Parse a byte size: a plain count (`1048576`) or a suffixed value (`512MB`,
/// `1GiB`). Suffixes are binary multiples, so `KB` and `KiB` are both 1024.
/// Case-insensitive; whitespace between number and suffix is allowed.
pub fn parse_byte_size(s: &str) -> Result<usize, String> {
    let (num, unit) = split_scalar_unit(s, "size")?;
    let multiplier: usize = match unit.as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        other => {
            return Err(format!(
                "invalid size '{s}': unknown unit '{other}' (expected one of B, KB, MB, GB)"
            ));
        }
    };
    let n: usize = num
        .parse()
        .map_err(|e| format!("invalid size '{s}': {e}"))?;
    n.checked_mul(multiplier)
        .ok_or_else(|| format!("invalid size '{s}': value overflows"))
}

/// Parse a duration string like `"1s"`, `"500ms"`, `"2m"`, `"1h"`, `"1d"`,
/// `"0s"`.
///
/// Accepted suffixes mirror Bazel's `--bes_timeout`: `ms`, `s`, `m`, `h`, `d`.
///
/// `"0s"` (or any zero value) is the documented sentinel for "no deadline"
/// when used as a timeout; the caller decides what zero means.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let (num, unit) = split_scalar_unit(s, "duration")?;
    let n: u64 = num
        .parse()
        .map_err(|e| format!("invalid duration '{s}': {e}"))?;
    Ok(match unit.as_str() {
        "ms" => Duration::from_millis(n),
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        "h" => Duration::from_secs(n * 3600),
        "d" => Duration::from_secs(n * 86_400),
        _ => {
            return Err(format!(
                "invalid duration '{s}': expected suffix one of 'ms', 's', 'm', 'h', 'd'"
            ));
        }
    })
}

/// Byte-bounded ring of unacked stream events keyed by their original sequence
/// number. On reconnect the entire buffer is replayed before fresh events
/// resume — the BES protocol's per-stream sequence-number dedup makes this
/// safe even if the server already saw some of the replayed events.
///
/// The bound is a byte budget, not an event count: BEP event sizes span orders
/// of magnitude (a few hundred bytes for most, tens of megabytes for one
/// carrying an action's stdout), so a count bounds neither memory nor stream
/// length.
///
/// Overflow is not an error. Retention exists only to enable replay, so when
/// the budget is exceeded the oldest entries are evicted. The cost is that a
/// later reconnect cannot replay the evicted range — the server tolerates the
/// resulting sequence gap, acking by position and paging forward past missing
/// seqs — while delivery of the live stream is unaffected.
pub struct RetryBuffer {
    /// Byte budget for retained (unacked) events.
    cap_bytes: usize,
    /// Sum of `encoded_len()` over `items`, maintained incrementally.
    bytes: usize,
    /// Events evicted since the last [`Self::take_evicted`].
    evicted: u64,
    /// `(sequence number, encoded size, request)`, oldest first. The size is
    /// cached so eviction and pruning adjust `bytes` without re-encoding.
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

        // `size <= cap_bytes` bounds the loop: an empty buffer has `bytes == 0`,
        // so the condition is false by the time everything has been evicted.
        while !self.items.is_empty() && self.bytes + size > self.cap_bytes {
            self.evict_oldest();
        }

        if size > self.cap_bytes {
            self.evicted += 1;
            return;
        }

        self.bytes += size;
        self.items.push_back((seq, size, req));
    }

    fn evict_oldest(&mut self) {
        if let Some((_, size, _)) = self.items.pop_front() {
            self.bytes -= size;
            self.evicted += 1;
        }
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

    /// Number of retained events.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Bytes currently retained, against the budget passed to [`Self::new`].
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Events dropped to stay under budget since the last call, resetting the
    /// counter.
    ///
    /// Callers report this once per reconnect, and the count is cumulative
    /// across the whole build; draining it keeps a second reconnect from
    /// re-reporting losses the first one already announced.
    pub fn take_evicted(&mut self) -> u64 {
        std::mem::take(&mut self.evicted)
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
        // `Cancelled` and `Unknown` are how a GOAWAY reaches us: hyper cancels
        // requests queued on a connection the server is retiring, and tonic maps
        // an h2 protocol error carrying no gRPC status to `Unknown`. Neither
        // request reached the application, so replaying it is safe. A local
        // abort never lands here — it closes the upstream instead, which
        // `drive_stream` reports as `UpstreamClosed`.
        ClientError::Status(status) => matches!(
            status.code(),
            Code::Unavailable
                | Code::DeadlineExceeded
                | Code::ResourceExhausted
                | Code::Aborted
                | Code::Internal
                | Code::Cancelled
                | Code::Unknown
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

    /// Many small events fit comfortably: event count alone never triggers
    /// eviction.
    #[test]
    fn buffer_retains_many_small_events() {
        let mut b = RetryBuffer::new(1024 * 1024);
        for i in 1..=20_000 {
            b.push(i, req());
        }
        assert_eq!(b.len(), 20_000);
        assert_eq!(b.take_evicted(), 0);
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
        assert_eq!(b.take_evicted(), 0);

        // Fourth event exceeds the budget: seq 1 is evicted to make room.
        b.push(4, req_sized(100));
        assert_eq!(seqs(&b), vec![2, 3, 4]);
        assert_eq!(b.take_evicted(), 1);
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
        assert!(
            b.len() < 10,
            "large event should displace several small ones"
        );
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
        assert_eq!(
            b.take_evicted(),
            2,
            "the evicted small event and the oversized one"
        );

        // The buffer stays usable afterwards.
        b.push(3, req_sized(10));
        assert_eq!(seqs(&b), vec![3]);
    }

    /// `take_evicted` drains, so consecutive reconnects each report only what
    /// was evicted since the last one rather than re-announcing old losses.
    #[test]
    fn buffer_take_evicted_drains() {
        let one = req_sized(100).encoded_len();
        let mut b = RetryBuffer::new(one * 2);
        assert_eq!(b.take_evicted(), 0, "nothing evicted yet");

        for i in 1..=4 {
            b.push(i, req_sized(100));
        }
        assert_eq!(b.take_evicted(), 2);
        assert_eq!(b.take_evicted(), 0, "second read must not repeat the count");

        b.push(5, req_sized(100));
        assert_eq!(b.take_evicted(), 1, "only the newly evicted event");
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
        let one = req_sized(100).encoded_len();
        let mut b = RetryBuffer::new(1024 * 1024);
        for i in 1..=10 {
            b.push(i, req_sized(100));
        }
        assert_eq!(b.bytes(), one * 10);

        b.prune_until(4);
        assert_eq!(b.bytes(), one * 6, "partial prune reclaims proportionally");

        b.prune_until(10);
        assert!(b.is_empty());
        assert_eq!(b.bytes(), 0);
    }

    /// Reclaimed budget is reusable: a stream that acks as it goes keeps
    /// pushing indefinitely without ever evicting.
    #[test]
    fn buffer_acked_stream_never_evicts() {
        let one = req_sized(100).encoded_len();
        let mut b = RetryBuffer::new(one * 4);
        for i in 1..=100 {
            b.push(i, req_sized(100));
            b.prune_until(i);
        }
        assert!(b.is_empty());
        assert_eq!(b.take_evicted(), 0, "acked events free their budget");
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
    fn parse_byte_size_plain_and_suffixed() {
        assert_eq!(parse_byte_size("1024").unwrap(), 1024);
        assert_eq!(parse_byte_size("0").unwrap(), 0);
        assert_eq!(parse_byte_size("512B").unwrap(), 512);
        assert_eq!(parse_byte_size("2KB").unwrap(), 2048);
        assert_eq!(parse_byte_size("256MB").unwrap(), 256 * 1024 * 1024);
        assert_eq!(parse_byte_size("1GB").unwrap(), 1024 * 1024 * 1024);

        // The `iB` spellings and bare unit letters are the same binary multiple.
        assert_eq!(parse_byte_size("256MiB").unwrap(), 256 * 1024 * 1024);
        assert_eq!(parse_byte_size("256M").unwrap(), 256 * 1024 * 1024);

        // Case and internal whitespace are tolerated.
        assert_eq!(parse_byte_size("256mb").unwrap(), 256 * 1024 * 1024);
        assert_eq!(parse_byte_size(" 256 MB ").unwrap(), 256 * 1024 * 1024);
    }

    #[test]
    fn parse_byte_size_rejects_malformed() {
        assert!(parse_byte_size("").is_err());
        assert!(parse_byte_size("   ").is_err());
        assert!(parse_byte_size("MB").is_err(), "no leading number");
        assert!(parse_byte_size("12TB").is_err(), "unsupported unit");
        assert!(parse_byte_size("1.5MB").is_err(), "fractional unsupported");
        assert!(parse_byte_size("-1").is_err(), "negative unsupported");
        assert!(
            parse_byte_size("99999999999999999999GB").is_err(),
            "overflow must be an error, not a wrap"
        );
    }

    /// End-to-end check that the resolver reads the real environment variable
    /// under its documented name — the unit tests below exercise the fallback
    /// rules but would keep passing if `RETRY_MAX_BUFFER_BYTES_ENV` were
    /// misspelled or `default_retry_max_buffer_bytes` stopped consulting it.
    ///
    /// Cannot call `default_retry_max_buffer_bytes` itself: it caches in a
    /// process-wide `OnceLock`, so the value it returns depends on whichever
    /// test touched it first. This drives the same path with an explicit read.
    #[test]
    fn env_var_name_is_read_from_the_environment() {
        // SAFETY: single-threaded within this test, and the value is removed
        // before returning so no other test observes it.
        unsafe { std::env::set_var(RETRY_MAX_BUFFER_BYTES_ENV, "7MB") };
        let raw = std::env::var(RETRY_MAX_BUFFER_BYTES_ENV).ok();
        let (bytes, warning) = resolve_buffer_bytes_override(raw.as_deref());
        unsafe { std::env::remove_var(RETRY_MAX_BUFFER_BYTES_ENV) };

        assert_eq!(bytes, 7 * 1024 * 1024);
        assert!(warning.is_none());
        assert_eq!(
            RETRY_MAX_BUFFER_BYTES_ENV, "ASPECT_CLI_BES_RETRY_MAX_BUFFER_BYTES",
            "the documented variable name is part of the public interface"
        );
    }

    /// A usable override is taken verbatim, with no warning.
    #[test]
    fn env_override_applies_when_valid() {
        let (bytes, warning) = resolve_buffer_bytes_override(Some("512MB"));
        assert_eq!(bytes, 512 * 1024 * 1024);
        assert!(warning.is_none(), "a valid override should be silent");

        let (bytes, warning) = resolve_buffer_bytes_override(Some("1048576"));
        assert_eq!(bytes, 1024 * 1024);
        assert!(warning.is_none());
    }

    /// Unset or empty is the common case and must not warn.
    #[test]
    fn env_override_absent_is_silent_default() {
        for raw in [None, Some(""), Some("   ")] {
            let (bytes, warning) = resolve_buffer_bytes_override(raw);
            assert_eq!(bytes, DEFAULT_RETRY_MAX_BUFFER_BYTES, "{raw:?}");
            assert!(warning.is_none(), "{raw:?} should not warn");
        }
    }

    /// A malformed or zero override falls back to the default and warns — a
    /// typo'd tuning knob must never break a build or silently disable the
    /// buffer.
    #[test]
    fn env_override_invalid_warns_and_falls_back() {
        for bad in ["garbage", "12TB", "1.5MB", "-1", "0", "0MB"] {
            let (bytes, warning) = resolve_buffer_bytes_override(Some(bad));
            assert_eq!(
                bytes, DEFAULT_RETRY_MAX_BUFFER_BYTES,
                "{bad:?} should fall back to the default"
            );
            let warning = warning.unwrap_or_else(|| panic!("{bad:?} should warn"));
            assert!(
                warning.contains(RETRY_MAX_BUFFER_BYTES_ENV),
                "warning should name the env var: {warning}"
            );
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
        assert!(parse_duration("10").is_err(), "a bare number has no unit");
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("5x").is_err(), "unknown unit");

        // Shared with `parse_byte_size` via `split_scalar_unit`: case and
        // internal whitespace are tolerated.
        assert_eq!(parse_duration("250MS").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_duration(" 2 m ").unwrap(), Duration::from_secs(120));
    }

    #[test]
    fn classifier_status_codes() {
        // `Cancelled` and `Unknown` are the two a GOAWAY produces: a request
        // hyper cancelled on a retiring connection, and an h2 protocol error
        // tonic could not map to a gRPC status.
        for code in [
            tonic::Code::Unavailable,
            tonic::Code::DeadlineExceeded,
            tonic::Code::ResourceExhausted,
            tonic::Code::Aborted,
            tonic::Code::Internal,
            tonic::Code::Cancelled,
            tonic::Code::Unknown,
        ] {
            let err = ClientError::Status(tonic::Status::new(code, "x"));
            assert!(is_retryable(&err), "{code:?} should be retryable");
        }
        for code in [
            tonic::Code::Unauthenticated,
            tonic::Code::PermissionDenied,
            tonic::Code::InvalidArgument,
        ] {
            let err = ClientError::Status(tonic::Status::new(code, "x"));
            assert!(!is_retryable(&err), "{code:?} should be terminal");
        }
        let bad_uri = "not a uri".parse::<hyper::Uri>().unwrap_err();
        assert!(!is_retryable(&ClientError::InvalidEndpoint(bad_uri)));
    }
}
