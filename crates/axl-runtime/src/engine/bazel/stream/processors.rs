//! Built-in [`LineProcessor`] stages for the captured-output pipeline.
//!
//! Four capabilities, chained by `Build::spawn` as: observers
//! ([`LineMatcher`]) first so they see every record, then the responder
//! ([`MatchResponder`]) which may rewrite records, then [`CollapseRepeats`]
//! which dedups whatever the earlier stages produced:
//!
//! - **Noise reduction** — [`CollapseRepeats`] folds runs of identical lines
//!   into the first occurrence plus a `(last line repeated N more times)`
//!   annotation. Intended for the pipe (non-TTY) capture mode where Bazel
//!   emits clean line-oriented output.
//! - **Pattern-driven hooks** — [`LineMatcher`] scans each record against
//!   configured regexes and invokes a callback per hit, forwarding the
//!   record unchanged. What the callback does is the caller's business.
//! - **Health signals** — [`OutputSignals`] is the shared state a
//!   [`LineMatcher`] callback typically feeds: a fatal flag + first fatal
//!   line, and a bounded list of match hits. `Build` exposes it to Starlark
//!   (`output_fatal`, `output_fatal_line`, `output_matches()`) so tasks can
//!   react — e.g. cancel a wedged invocation or mark a runner unhealthy.
//! - **Interactive respond/replace** — [`MatchResponder`] holds a matching
//!   record, ships it to a consumer (AXL) as a [`PendingMatch`], and blocks
//!   forwarding until a [`Verdict`] arrives or the timeout fires. This is
//!   what lets AXL rewrite or suppress a line *before* the user sees it;
//!   see the type docs for the ordering and fail-open contract.
//!
//! All stages run on the reader thread; `LineMatcher`/`CollapseRepeats` work
//! is bounded and non-blocking (the `OutputSignals` mutexes are only ever
//! held briefly), while `MatchResponder` blocks by design — bounded by its
//! timeout — only on records that match.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use regex::Regex;

use super::output::LineProcessor;

/// Cap on recorded match hits; further hits still fire callbacks/flags but are
/// not stored, so a pathological pattern can't grow memory unboundedly.
const MAX_RECORDED_MATCHES: usize = 1024;

/// Cap on the stored copy of a matched/fatal line.
const MAX_RECORDED_LINE_BYTES: usize = 2048;

/// Collapse runs of consecutive identical records into the first occurrence
/// plus an annotation carrying the repeat count.
///
/// The first occurrence is forwarded immediately (streaming — never held
/// back). Each repeat is dropped and counted; when a different record arrives
/// the annotation is prepended to it, and a run still open at end-of-stream is
/// flushed via `finish`.
#[derive(Default)]
pub struct CollapseRepeats {
    prev: Option<Vec<u8>>,
    repeats: u64,
}

impl CollapseRepeats {
    pub fn new() -> Self {
        Self::default()
    }

    fn annotation(&self) -> Vec<u8> {
        let s = if self.repeats == 1 { "" } else { "s" };
        format!("(last line repeated {} more time{s})", self.repeats).into_bytes()
    }
}

impl LineProcessor for CollapseRepeats {
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        if self.prev.as_deref() == Some(record) {
            self.repeats += 1;
            return None;
        }
        let mut out = Vec::with_capacity(record.len());
        if self.repeats > 0 {
            out.extend_from_slice(&self.annotation());
            out.push(b'\n');
            self.repeats = 0;
        }
        self.prev = Some(record.to_vec());
        out.extend_from_slice(record);
        Some(out)
    }

    fn finish(&mut self) -> Option<Vec<u8>> {
        if self.repeats == 0 {
            return None;
        }
        let mut out = self.annotation();
        out.push(b'\n');
        Some(out)
    }
}

/// Callback fired by [`LineMatcher`] per pattern hit, with `(id, line)`.
pub type MatchCallback = Box<dyn FnMut(&str, &str) + Send>;

/// Scan each record against configured `(id, regex)` patterns and fire
/// `on_match(id, line)` per hit, forwarding the record unchanged.
///
/// Patterns are pre-compiled [`Regex`]es, so per-record cost is linear scans
/// that can't stall forwarding. The callback runs on the reader thread and
/// must not block.
pub struct LineMatcher {
    patterns: Vec<(String, Regex)>,
    on_match: MatchCallback,
}

impl LineMatcher {
    pub fn new(patterns: Vec<(String, Regex)>, on_match: MatchCallback) -> Self {
        Self { patterns, on_match }
    }
}

impl LineProcessor for LineMatcher {
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        if !self.patterns.is_empty() {
            let line = String::from_utf8_lossy(record);
            for (id, regex) in &self.patterns {
                if regex.is_match(&line) {
                    (self.on_match)(id, &line);
                }
            }
        }
        Some(record.to_vec())
    }
}

/// A consumer's decision on a [`PendingMatch`].
#[derive(Debug)]
pub enum Verdict {
    /// Forward the record unchanged.
    Keep,
    /// Forward these bytes instead (the original boundary is preserved).
    Replace(Vec<u8>),
    /// Suppress the record and its boundary.
    Drop,
}

/// A matched record awaiting a consumer's [`Verdict`]. Sending on `reply`
/// releases the held record; dropping the reply sender (or never answering)
/// fails open — the original record forwards once the responder times out.
pub struct PendingMatch {
    pub id: String,
    pub line: String,
    pub reply: mpsc::SyncSender<Verdict>,
}

/// Hold each record that matches an `(id, regex)` pattern until a consumer
/// (AXL, draining `build.output_events()`) delivers a [`Verdict`], then
/// forward accordingly. Non-matching records flow through untouched.
///
/// **Ordering:** while a verdict is pending, the reader thread is blocked, so
/// every subsequent record waits behind the held one — output order is always
/// preserved, and Bazel is back-pressured by the kernel pipe/PTY buffer. This
/// hold is what makes replace-before-display possible.
///
/// **Fail-open:** if no verdict arrives within `timeout` (consumer busy), or
/// the event is dropped unanswered, or nothing ever drains the events channel
/// (receiver gone), the original record forwards unchanged. A misbehaving
/// consumer can slow matched lines down, never wedge the build.
pub struct MatchResponder {
    patterns: Vec<(String, Regex)>,
    events: mpsc::Sender<PendingMatch>,
    timeout: Duration,
}

impl MatchResponder {
    /// `events` is the channel the consumer drains; one `PendingMatch` is in
    /// flight at a time (the reader blocks on its reply before matching the
    /// next record).
    pub fn new(
        patterns: Vec<(String, Regex)>,
        events: mpsc::Sender<PendingMatch>,
        timeout: Duration,
    ) -> Self {
        Self {
            patterns,
            events,
            timeout,
        }
    }
}

impl LineProcessor for MatchResponder {
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        let line = String::from_utf8_lossy(record);
        let id = match self.patterns.iter().find(|(_, r)| r.is_match(&line)) {
            Some((id, _)) => id.clone(),
            None => return Some(record.to_vec()),
        };
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let pending = PendingMatch {
            id,
            line: line.into_owned(),
            reply: reply_tx,
        };
        if self.events.send(pending).is_err() {
            // No consumer (receiver dropped) — fail open immediately.
            return Some(record.to_vec());
        }
        match reply_rx.recv_timeout(self.timeout) {
            Ok(Verdict::Keep) => Some(record.to_vec()),
            Ok(Verdict::Replace(bytes)) => Some(bytes),
            Ok(Verdict::Drop) => None,
            // Timeout, or the event was dropped unanswered — fail open.
            Err(_) => Some(record.to_vec()),
        }
    }
}

/// Shared state fed by [`LineMatcher`] callbacks and read by the `Build`
/// handle (and through it, Starlark) while the invocation runs or after it
/// finishes.
///
/// `fatal` latches on the first fatal-pattern hit and keeps that line;
/// `matches` accumulates `(id, line)` hits up to [`MAX_RECORDED_MATCHES`].
/// All access is lock-brief and thread-safe.
#[derive(Debug, Default)]
pub struct OutputSignals {
    fatal: AtomicBool,
    fatal_line: Mutex<Option<String>>,
    matches: Mutex<Vec<(String, String)>>,
}

impl OutputSignals {
    /// Latch the fatal flag; the first fatal line wins.
    pub fn set_fatal(&self, line: &str) {
        if !self.fatal.swap(true, Ordering::SeqCst) {
            *self.fatal_line.lock().unwrap() = Some(truncate(line));
        }
    }

    pub fn fatal(&self) -> bool {
        self.fatal.load(Ordering::SeqCst)
    }

    pub fn fatal_line(&self) -> Option<String> {
        self.fatal_line.lock().unwrap().clone()
    }

    /// Record a match hit; hits past [`MAX_RECORDED_MATCHES`] are dropped.
    pub fn record_match(&self, id: &str, line: &str) {
        let mut matches = self.matches.lock().unwrap();
        if matches.len() < MAX_RECORDED_MATCHES {
            matches.push((id.to_string(), truncate(line)));
        }
    }

    /// Snapshot of the recorded `(id, line)` hits so far.
    pub fn matches(&self) -> Vec<(String, String)> {
        self.matches.lock().unwrap().clone()
    }
}

/// Bound the stored copy of a line, respecting char boundaries.
fn truncate(line: &str) -> String {
    if line.len() <= MAX_RECORDED_LINE_BYTES {
        return line.to_string();
    }
    let mut end = MAX_RECORDED_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn feed(p: &mut dyn LineProcessor, records: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for r in records {
            if let Some(bytes) = p.process(r) {
                out.extend_from_slice(&bytes);
                out.push(b'\n');
            }
        }
        if let Some(bytes) = p.finish() {
            out.extend_from_slice(&bytes);
        }
        out
    }

    #[test]
    fn collapse_repeats_basic() {
        let mut p = CollapseRepeats::new();
        let out = feed(&mut p, &[b"a", b"a", b"a", b"b"]);
        assert_eq!(out, b"a\n(last line repeated 2 more times)\nb\n");
    }

    #[test]
    fn collapse_repeats_no_repeats_is_passthrough() {
        let mut p = CollapseRepeats::new();
        assert_eq!(feed(&mut p, &[b"a", b"b", b"c"]), b"a\nb\nc\n");
    }

    #[test]
    fn collapse_repeats_flushes_trailing_run_on_finish() {
        let mut p = CollapseRepeats::new();
        let out = feed(&mut p, &[b"x", b"x", b"x"]);
        assert_eq!(out, b"x\n(last line repeated 2 more times)\n");
    }

    #[test]
    fn collapse_repeats_multiple_runs() {
        let mut p = CollapseRepeats::new();
        let out = feed(&mut p, &[b"a", b"a", b"b", b"b", b"b", b"a"]);
        assert_eq!(
            out,
            b"a\n(last line repeated 1 more time)\nb\n(last line repeated 2 more times)\na\n"
                .to_vec()
        );
    }

    fn rx(pattern: &str) -> Regex {
        Regex::new(pattern).unwrap()
    }

    #[test]
    fn line_matcher_fires_per_hit_and_forwards_unchanged() {
        let hits = Arc::new(Mutex::new(Vec::new()));
        let sink = hits.clone();
        let mut p = LineMatcher::new(
            vec![
                ("oom".to_string(), rx(r"OutOfMemory")),
                ("warn".to_string(), rx(r"^WARNING:")),
            ],
            Box::new(move |id, line| sink.lock().unwrap().push((id.to_string(), line.to_string()))),
        );
        let out = feed(
            &mut p,
            &[
                b"INFO: ok",
                b"java.lang.OutOfMemoryError: heap",
                b"WARNING: deprecated",
            ],
        );
        assert_eq!(
            out,
            b"INFO: ok\njava.lang.OutOfMemoryError: heap\nWARNING: deprecated\n"
        );
        let hits = hits.lock().unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "oom");
        assert_eq!(hits[1].0, "warn");
    }

    #[test]
    fn responder_verdicts_apply() {
        let (events_tx, events_rx) = mpsc::channel();
        // Consumer thread: keep INFO lines, rewrite WARNINGs, drop DEBUGs.
        let consumer = std::thread::spawn(move || {
            for ev in events_rx {
                let PendingMatch { id, reply, .. } = ev;
                let verdict = match id.as_str() {
                    "warn" => Verdict::Replace(b"warning elided".to_vec()),
                    "debug" => Verdict::Drop,
                    _ => Verdict::Keep,
                };
                let _ = reply.send(verdict);
            }
        });

        let mut p = MatchResponder::new(
            vec![
                ("info".to_string(), rx(r"^INFO:")),
                ("warn".to_string(), rx(r"^WARNING:")),
                ("debug".to_string(), rx(r"^DEBUG:")),
            ],
            events_tx,
            Duration::from_secs(5),
        );
        let out = feed(
            &mut p,
            &[b"INFO: ok", b"WARNING: noisy", b"DEBUG: spam", b"plain"],
        );
        assert_eq!(out, b"INFO: ok\nwarning elided\nplain\n");
        drop(p); // disconnects the events channel so the consumer loop exits
        consumer.join().unwrap();
    }

    #[test]
    fn responder_fails_open_on_timeout() {
        let (events_tx, events_rx) = mpsc::channel();
        // Consumer receives but never answers.
        let _keep_alive = events_rx;
        let mut p = MatchResponder::new(
            vec![("m".to_string(), rx("match"))],
            events_tx,
            Duration::from_millis(50),
        );
        assert_eq!(feed(&mut p, &[b"match me"]), b"match me\n");
    }

    #[test]
    fn responder_fails_open_with_no_consumer() {
        let (events_tx, events_rx) = mpsc::channel::<PendingMatch>();
        drop(events_rx);
        let mut p = MatchResponder::new(
            vec![("m".to_string(), rx("match"))],
            events_tx,
            Duration::from_secs(30), // must not wait this long
        );
        let start = std::time::Instant::now();
        assert_eq!(feed(&mut p, &[b"match me"]), b"match me\n");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn responder_fails_open_when_event_dropped_unanswered() {
        let (events_tx, events_rx) = mpsc::channel();
        let consumer = std::thread::spawn(move || {
            for ev in events_rx {
                drop(ev); // reply sender dropped without a verdict
            }
        });
        let mut p = MatchResponder::new(
            vec![("m".to_string(), rx("match"))],
            events_tx,
            Duration::from_secs(30), // disconnect must release before this
        );
        let start = std::time::Instant::now();
        assert_eq!(feed(&mut p, &[b"match me"]), b"match me\n");
        assert!(start.elapsed() < Duration::from_secs(5));
        drop(p);
        consumer.join().unwrap();
    }

    #[test]
    fn signals_fatal_latches_first_line() {
        let s = OutputSignals::default();
        assert!(!s.fatal());
        s.set_fatal("first failure");
        s.set_fatal("second failure");
        assert!(s.fatal());
        assert_eq!(s.fatal_line().as_deref(), Some("first failure"));
    }

    #[test]
    fn signals_matches_capped() {
        let s = OutputSignals::default();
        for i in 0..(MAX_RECORDED_MATCHES + 10) {
            s.record_match("id", &format!("line {i}"));
        }
        assert_eq!(s.matches().len(), MAX_RECORDED_MATCHES);
    }

    #[test]
    fn truncate_respects_char_boundary() {
        let long = "é".repeat(MAX_RECORDED_LINE_BYTES); // 2 bytes per char
        let t = truncate(&long);
        assert!(t.len() <= MAX_RECORDED_LINE_BYTES);
        assert!(t.chars().all(|c| c == 'é'));
    }
}
