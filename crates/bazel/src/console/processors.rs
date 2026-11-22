//! The stages that observe or answer lines, and the repeat collapser:
//!
//! * [`LineMatcher`], which observes every record as it is shown (after the
//!   rewrites), and feeds [`Observations`]: fatal-pattern latching and match
//!   recording.
//! * [`Aggregator`], which feeds every record's message text to one of the
//!   [`Observations`]' [`Tally`]s. It and [`LineMatcher`] (the `observe`
//!   and `fatal` stages) never see a row of Bazel's progress UI
//!   (`is_progress_row`), which curses redraws and which says nothing a
//!   message does not.
//! * [`Interceptor`], which holds a matching record until a consumer
//!   answers with a [`Verdict`], so it can be rewritten or dropped before it
//!   is shown.
//! * [`CollapseRepeats`], which folds runs of identical lines in what the
//!   stages before it produced.
//!
//! Patterns match a record's message text ([`line::message_of`]): escape
//! sequences and a `--show_timestamps` prefix stripped, so `^WARNING:` matches
//! under a PTY and with timestamps too. The order stages run in is
//! [`super::Phase`]'s.

use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use regex::Regex;

use super::aggregate::{Spec, Tally};
use super::ansi;
use super::forward::LineProcessor;
use super::line;
use super::report::cut;

/// Hits kept by [`Observations`]; later hits still latch `fatal`.
pub const MAX_RECORDED_MATCHES: usize = 1024;

/// The longest copy of a line [`Observations`] keeps.
pub const MAX_RECORDED_LINE_BYTES: usize = 2048;

/// Fold each run of identical records into its first occurrence, then a
/// `(last line repeated N more times)` line before whatever follows the run
/// (or at end of stream). The first occurrence is forwarded at once. Blank
/// records are never folded; any other is, an action's output included.
#[derive(Default)]
pub struct CollapseRepeats {
    prev: Option<Vec<u8>>,
    repeats: u64,
}

impl CollapseRepeats {
    fn annotation(&mut self) -> Vec<u8> {
        let s = if self.repeats == 1 { "" } else { "s" };
        let line = format!("(last line repeated {} more time{s})\n", self.repeats);
        self.repeats = 0;
        line.into_bytes()
    }
}

impl LineProcessor for CollapseRepeats {
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        if !record.is_empty() && self.prev.as_deref() == Some(record) {
            self.repeats += 1;
            return None;
        }
        let mut out = if self.repeats > 0 {
            self.annotation()
        } else {
            Vec::new()
        };
        out.extend_from_slice(record);
        self.prev = Some(record.to_vec());
        Some(out)
    }

    fn finish(&mut self) -> Option<Vec<u8>> {
        (self.repeats > 0).then(|| self.annotation())
    }
}

/// Called by [`LineMatcher`] with `(id, line)` for each pattern a record
/// matches.
pub type OnMatch = Box<dyn FnMut(&str, &str) + Send>;

/// Calls `on_match` for every `(id, regex)` a record matches and forwards the
/// record unchanged.
pub struct LineMatcher {
    patterns: Vec<(String, Regex)>,
    on_match: OnMatch,
    given: Option<line::Line>,
}

impl LineMatcher {
    pub fn new(patterns: Vec<(String, Regex)>, on_match: OnMatch) -> Self {
        Self {
            patterns,
            on_match,
            given: None,
        }
    }
}

/// Whether `line` is a row of Bazel's progress UI, which no matching stage
/// sees: its first line or a row under it, a row curses drew to redraw
/// (which would match once per redraw), or the status line plain lines let
/// through. An action's output shaped like one is not.
fn is_progress_row(line: Option<&line::Line>) -> bool {
    line.is_some_and(|l| l.drawn || l.kind.is_progress())
}

impl LineProcessor for LineMatcher {
    fn see(&mut self, line: &line::Line) {
        self.given = Some(line.clone());
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        if is_progress_row(self.given.take().as_ref()) {
            return Some(record.to_vec());
        }
        let line = line::message_of(record);
        for (id, regex) in &self.patterns {
            if regex.is_match(&line) {
                (self.on_match)(id, &line);
            }
        }
        Some(record.to_vec())
    }
}

/// A consumer's answer to a [`HeldLine`].
#[derive(Debug)]
pub enum Verdict {
    Keep,
    /// Forward these bytes in place of the record's text: after the escape
    /// sequences it starts with, before the rest of its escape sequences
    /// (a colour reset, say) and its boundary.
    Replace(Vec<u8>),
    /// Suppress the record and its boundary, leaving only its escape
    /// sequences.
    Drop,
}

/// A held record, waiting for a [`Verdict`] on `reply`. Dropping `reply`
/// unanswered releases the record unchanged. `line` is its message text.
pub struct HeldLine {
    pub id: String,
    pub line: String,
    pub reply: mpsc::SyncSender<Verdict>,
}

/// Holds each record matching one of `patterns` (the first match names it)
/// until the consumer of `events` answers, then forwards per the [`Verdict`].
///
/// The reader thread waits while a record is held, so everything after it
/// waits too: output order is preserved and bazel is back-pressured. The wait
/// fails open: no answer within `timeout`, an event dropped unanswered, or no
/// consumer at all forwards the original record. A slow consumer can delay
/// matched lines; it cannot wedge the build.
pub struct Interceptor {
    patterns: Vec<(String, Regex)>,
    events: mpsc::Sender<HeldLine>,
    timeout: Duration,
    /// See [`Interceptor::under_pty`].
    pty: bool,
    /// What the chain said the record is ([`LineProcessor::see`]).
    given: Option<line::Line>,
}

impl Interceptor {
    pub fn new(
        patterns: Vec<(String, Regex)>,
        events: mpsc::Sender<HeldLine>,
        timeout: Duration,
    ) -> Self {
        Self {
            patterns,
            events,
            timeout,
            pty: false,
            given: None,
        }
    }

    /// For a PTY capture: lines of bazel's progress area, and blank lines,
    /// are never held. Curses redraws that area by moving the cursor up over
    /// as many lines as it wrote, so a dropped or multi-line answer there
    /// would make its next redraw erase output above it.
    pub fn under_pty(mut self, pty: bool) -> Self {
        self.pty = pty;
        self
    }
}

impl LineProcessor for Interceptor {
    fn see(&mut self, line: &line::Line) {
        self.given = Some(line.clone());
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        let given = self.given.take();
        if self.pty && given.is_some_and(|l| l.drawn || l.kind == line::Kind::Blank) {
            return Some(record.to_vec());
        }
        let line = line::message_of(record);
        let Some((id, _)) = self.patterns.iter().find(|(_, r)| r.is_match(&line)) else {
            return Some(record.to_vec());
        };
        let (reply, verdict) = mpsc::sync_channel(1);
        let pending = HeldLine {
            id: id.clone(),
            line,
            reply,
        };
        if self.events.send(pending).is_err() {
            return Some(record.to_vec());
        }
        match verdict.recv_timeout(self.timeout) {
            Ok(Verdict::Keep) | Err(_) => Some(record.to_vec()),
            Ok(Verdict::Replace(bytes)) => {
                let lead = ansi::leading(record);
                let rest = ansi::controls(&record[lead.len()..]);
                Some([lead, &bytes, &rest].concat())
            }
            Ok(Verdict::Drop) => None,
        }
    }
}

/// What the [`LineMatcher`]s saw, shared with whoever holds the build: whether
/// a fatal pattern matched and on which line first, and up to
/// [`MAX_RECORDED_MATCHES`] `(id, line)` hits of the observer patterns.
#[derive(Debug, Default)]
pub struct Observations {
    fatal_line: Mutex<Option<String>>,
    matches: Mutex<Vec<(String, String)>>,
    /// The ids `matches` has a hit of, for its cap ([`Observations::record_match`]).
    seen: Mutex<std::collections::HashSet<String>>,
    /// One per aggregate stage, in the order the stages were made.
    tallies: Mutex<Vec<Tally>>,
}

impl Observations {
    /// Latch `fatal`; the first line wins.
    pub fn set_fatal(&self, line: &str) {
        self.fatal_line
            .lock()
            .unwrap()
            .get_or_insert_with(|| truncate(line));
    }

    pub fn fatal(&self) -> bool {
        self.fatal_line.lock().unwrap().is_some()
    }

    pub fn fatal_line(&self) -> Option<String> {
        self.fatal_line.lock().unwrap().clone()
    }

    /// Record a hit, up to [`MAX_RECORDED_MATCHES`] in all; past that, an
    /// id's first hit is still recorded, so a busy observer cannot hide the
    /// one line another one waits for.
    pub fn record_match(&self, id: &str, line: &str) {
        let mut matches = self.matches.lock().unwrap();
        let first = self.seen.lock().unwrap().insert(id.to_string());
        if matches.len() < MAX_RECORDED_MATCHES || first {
            matches.push((id.to_string(), truncate(line)));
        }
    }

    /// The hits recorded so far.
    pub fn matches(&self) -> Vec<(String, String)> {
        self.matches.lock().unwrap().clone()
    }

    /// Keep a [`Tally`] for `spec`; its slot, for [`Observations::tally`].
    pub fn add_tally(&self, spec: Spec) -> usize {
        let mut tallies = self.tallies.lock().unwrap();
        tallies.push(Tally::new(spec));
        tallies.len() - 1
    }

    /// Give `line`, a record's message text, to the tally in `slot`.
    pub fn tally(&self, slot: usize, line: &str) {
        if let Some(t) = self.tallies.lock().unwrap().get_mut(slot) {
            t.add(line);
        }
    }

    /// Every tally's [`Tally::report`] so far, in the order they were added.
    pub fn aggregates(&self) -> Vec<serde_json::Value> {
        self.tallies
            .lock()
            .unwrap()
            .iter()
            .map(Tally::report)
            .collect()
    }
}

/// Feeds each record's message text to one tally of [`Observations`]: not a
/// row of Bazel's progress UI ([`is_progress_row`]).
pub struct Aggregator {
    observations: std::sync::Arc<Observations>,
    slot: usize,
    given: Option<line::Line>,
}

impl Aggregator {
    pub fn new(observations: std::sync::Arc<Observations>, slot: usize) -> Self {
        Self {
            observations,
            slot,
            given: None,
        }
    }
}

impl LineProcessor for Aggregator {
    fn see(&mut self, line: &line::Line) {
        self.given = Some(line.clone());
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        if !is_progress_row(self.given.take().as_ref()) {
            self.observations
                .tally(self.slot, &line::message_of(record));
        }
        Some(record.to_vec())
    }
}

/// `line` cut to [`MAX_RECORDED_LINE_BYTES`] on a char boundary.
fn truncate(line: &str) -> String {
    cut(line, MAX_RECORDED_LINE_BYTES).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ids_first_hit_is_kept_past_the_cap() {
        let o = Observations::default();
        for _ in 0..MAX_RECORDED_MATCHES + 5 {
            o.record_match("busy", "x");
        }
        o.record_match("rare", "the one");
        o.record_match("rare", "again");
        let got = o.matches();
        assert_eq!(got.len(), MAX_RECORDED_MATCHES + 1);
        assert_eq!(
            got.last().unwrap(),
            &("rare".to_string(), "the one".to_string())
        );
    }
    use std::sync::Arc;
    use std::time::Instant;

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

    fn rx(pattern: &str) -> Regex {
        Regex::new(pattern).unwrap()
    }

    fn interceptor(timeout: Duration) -> (Interceptor, mpsc::Receiver<HeldLine>) {
        let (tx, rx_events) = mpsc::channel();
        (
            Interceptor::new(vec![("m".into(), rx("match"))], tx, timeout),
            rx_events,
        )
    }

    #[test]
    fn collapse_repeats_folds_each_run() {
        let mut p = CollapseRepeats::default();
        assert_eq!(
            feed(&mut p, &[b"a", b"a", b"a", b"b"]),
            b"a\n(last line repeated 2 more times)\nb\n"
        );
        let mut p = CollapseRepeats::default();
        assert_eq!(
            feed(&mut p, &[b"a", b"a", b"b", b"b", b"b", b"a"]),
            b"a\n(last line repeated 1 more time)\nb\n(last line repeated 2 more times)\na\n"
        );
    }

    #[test]
    fn collapse_repeats_passes_distinct_lines_through() {
        let mut p = CollapseRepeats::default();
        assert_eq!(feed(&mut p, &[b"a", b"b", b"c"]), b"a\nb\nc\n");
    }

    #[test]
    fn collapse_repeats_flushes_a_trailing_run() {
        let mut p = CollapseRepeats::default();
        assert_eq!(
            feed(&mut p, &[b"x", b"x", b"x"]),
            b"x\n(last line repeated 2 more times)\n"
        );
    }

    #[test]
    fn collapse_repeats_leaves_blank_lines_alone() {
        let mut p = CollapseRepeats::default();
        assert_eq!(feed(&mut p, &[b"a", b"", b"", b"b"]), b"a\n\n\nb\n");
    }

    #[test]
    fn line_matcher_reports_every_pattern_hit_and_forwards_unchanged() {
        let hits = Arc::new(Mutex::new(Vec::new()));
        let sink = hits.clone();
        let mut p = LineMatcher::new(
            vec![
                ("oom".into(), rx("OutOfMemory")),
                ("java".into(), rx(r"^java\.")),
                ("warn".into(), rx("^WARNING:")),
            ],
            Box::new(move |id, line| {
                sink.lock()
                    .unwrap()
                    .push((id.to_string(), line.to_string()))
            }),
        );
        let out = feed(&mut p, &[b"INFO: ok", b"java.lang.OutOfMemoryError: heap"]);
        assert_eq!(out, b"INFO: ok\njava.lang.OutOfMemoryError: heap\n");
        let ids: Vec<String> = hits
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(ids, ["oom", "java"]);
    }

    #[test]
    fn interceptor_applies_each_verdict() {
        let (tx, events) = mpsc::channel();
        let consumer = std::thread::spawn(move || {
            for HeldLine { id, reply, .. } in events {
                let _ = reply.send(match id.as_str() {
                    "warn" => Verdict::Replace(b"warning elided".to_vec()),
                    "debug" => Verdict::Drop,
                    _ => Verdict::Keep,
                });
            }
        });
        let mut p = Interceptor::new(
            vec![
                ("info".into(), rx("^INFO:")),
                ("warn".into(), rx("^WARNING:")),
                ("debug".into(), rx("^DEBUG:")),
            ],
            tx,
            Duration::from_secs(30),
        );
        let out = feed(
            &mut p,
            &[b"INFO: ok", b"WARNING: noisy", b"DEBUG: spam", b"plain"],
        );
        assert_eq!(out, b"INFO: ok\nwarning elided\nplain\n");
        drop(p);
        consumer.join().unwrap();
    }

    /// Under a PTY the held record starts with curses' cursor movement, which
    /// the replacement must keep; the consumer sees the text alone.
    #[test]
    fn interceptor_keeps_the_leading_escapes_of_a_replaced_record() {
        let (tx, events) = mpsc::channel();
        let consumer = std::thread::spawn(move || {
            for HeldLine { line, reply, .. } in events {
                assert_eq!(line, "WARNING: password=x");
                let _ = reply.send(Verdict::Replace(b"(elided)".to_vec()));
            }
        });
        let mut p = Interceptor::new(
            vec![("m".into(), rx("^WARNING:"))],
            tx,
            Duration::from_secs(30),
        );
        let out = p.process(b"\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mpassword=x");
        assert_eq!(
            out.as_deref(),
            Some(&b"\x1b[1A\x1b[K\x1b[35m(elided)\x1b[0m"[..])
        );
        drop(p);
        consumer.join().unwrap();
    }

    /// Under a PTY a line of the progress area is never held, so no answer
    /// can change how many lines curses wrote there.
    #[test]
    fn under_a_pty_the_progress_area_is_never_held() {
        let (tx, events) = mpsc::channel();
        let mut p = Interceptor::new(
            vec![("f".into(), rx("Fetching"))],
            tx,
            Duration::from_secs(30),
        )
        .under_pty(true);
        let mut lines = line::Classifier::default();
        for record in [
            &b"\x1b[32mLoading:\x1b[0m 1 package"[..],
            b"    Fetching https://u:t@x.dev/a",
        ] {
            p.see(&lines.classify(record));
            assert_eq!(p.process(record).as_deref(), Some(record));
        }
        assert!(events.try_recv().is_err(), "nothing was held");
    }

    #[test]
    fn interceptor_fails_open_on_timeout() {
        let (mut p, _unanswered) = interceptor(Duration::from_millis(50));
        assert_eq!(feed(&mut p, &[b"match me"]), b"match me\n");
    }

    #[test]
    fn interceptor_fails_open_at_once_without_a_consumer() {
        let (mut p, events) = interceptor(Duration::from_secs(30));
        drop(events);
        let start = Instant::now();
        assert_eq!(feed(&mut p, &[b"match me"]), b"match me\n");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn interceptor_fails_open_at_once_when_an_event_is_dropped() {
        let (mut p, events) = interceptor(Duration::from_secs(30));
        let consumer = std::thread::spawn(move || events.into_iter().for_each(drop));
        let start = Instant::now();
        assert_eq!(feed(&mut p, &[b"match me"]), b"match me\n");
        assert!(start.elapsed() < Duration::from_secs(5));
        drop(p);
        consumer.join().unwrap();
    }

    #[test]
    fn observations_latch_the_first_fatal_line() {
        let s = Observations::default();
        assert!(!s.fatal());
        assert_eq!(s.fatal_line(), None);
        s.set_fatal("first failure");
        s.set_fatal("second failure");
        assert!(s.fatal());
        assert_eq!(s.fatal_line().as_deref(), Some("first failure"));
    }

    #[test]
    fn observations_cap_recorded_matches() {
        let s = Observations::default();
        for i in 0..(MAX_RECORDED_MATCHES + 10) {
            s.record_match("id", &format!("line {i}"));
        }
        assert_eq!(s.matches().len(), MAX_RECORDED_MATCHES);
    }

    #[test]
    fn truncate_cuts_on_a_char_boundary() {
        let long = "é".repeat(MAX_RECORDED_LINE_BYTES);
        let t = truncate(&long);
        assert!(t.len() <= MAX_RECORDED_LINE_BYTES);
        assert!(t.chars().all(|c| c == 'é'));
        assert_eq!(truncate("short"), "short");
    }
}
