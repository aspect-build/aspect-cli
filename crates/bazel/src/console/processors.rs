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
use super::report::Style;
use super::report::cut;

/// Hits kept by [`Observations`]; later hits still latch `fatal`.
pub const MAX_RECORDED_MATCHES: usize = 1024;

/// The longest copy of a line [`Observations`] keeps.
pub const MAX_RECORDED_LINE_BYTES: usize = 2048;

/// Fold each run of identical lines into its first occurrence, then a
/// `(last line repeated N more times)` line (or one at end of stream). The
/// first occurrence is forwarded at once. Lines are compared by their message
/// text ([`line::message_of`]), so a colour or a `--show_timestamps` prefix
/// does not tell two apart. Any line folds, an action's output included,
/// except:
///
/// * a blank line;
/// * a row of Bazel's progress area or its status line, which ends a run:
///   during a stall the same status line coming through again is what shows
///   the build is still alive;
/// * a record that starts by moving the cursor up (a tool redrawing its own
///   line), which overwrites the line before rather than repeating it;
/// * under a PTY, a record that ends in a bare `\r` (a tool's spinner), which
///   the next record writes over, and which, unless blank, ends a run: a bare
///   `\r` with nothing on it, as a tool writes before moving up, leaves the
///   run pending.
///
/// The count goes just before the record that ends the run, which under a
/// PTY ([`CollapseRepeats::under_pty`]) is normally the first row of the
/// progress area curses draws after each burst of output, without moving
/// the cursor first: curses later moves up over the rows it drew, counting
/// from below them, so a line written before the area leaves that count
/// right. A tool redrawing its own lines counts rows up from where it is,
/// the folded copies among them, so before one the copies are written out
/// as they came instead of a count, and once one has redrawn, nothing folds
/// until Bazel's next progress row. A tool whose first redraw reaches back
/// past a run already counted, with other lines between, is the case left:
/// its redraw lands `N` rows higher. The count is styled as the console's
/// detail lines ([`Style`]).
#[derive(Default)]
pub struct CollapseRepeats {
    pty: bool,
    style: Style,
    given: line::Given,
    prev: Option<String>,
    repeats: u64,
    /// The last record folded, as it came, to write out again in place of
    /// the count before a tool's redraw.
    folded: Vec<u8>,
    /// The cursor moved up in this burst of output other than to erase
    /// Bazel's progress area (a tool redrawing its own lines, or curses
    /// erasing an area of a blank row), so nothing folds until Bazel's next
    /// progress row: a missed fold, never a wrong screen.
    redrawn: bool,
    /// The last record with text was a row of Bazel's progress area, so a
    /// record starting with cursor movement is curses erasing it.
    after_area: bool,
}

impl CollapseRepeats {
    /// For a PTY capture, where curses draws its progress area among the
    /// records.
    pub fn under_pty(mut self, pty: bool) -> Self {
        self.pty = pty;
        self
    }

    /// The count styled as `style`'s detail lines.
    pub fn styled(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    fn annotation(&mut self) -> Vec<u8> {
        let s = if self.repeats == 1 { "" } else { "s" };
        let line = format!(
            "{}(last line repeated {} more time{s}){}\n",
            self.style.open, self.repeats, self.style.close
        );
        self.repeats = 0;
        line.into_bytes()
    }

    /// The folded copies, each with the boundary it came with, in place of
    /// the count.
    fn unfolded(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..self.repeats {
            out.extend_from_slice(&self.folded);
            out.push(b'\n');
        }
        self.repeats = 0;
        out
    }

    /// `record` with a pending count written before it.
    fn with_count(&mut self, record: &[u8]) -> Vec<u8> {
        if self.repeats == 0 {
            return record.to_vec();
        }
        let mut out = self.annotation();
        out.extend_from_slice(record);
        out
    }
}

impl LineProcessor for CollapseRepeats {
    fn see(&mut self, line: &line::Line) {
        self.given.see(line);
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        let given = self.given.take(record);
        let text = line::message_of(record);
        let blank = text.trim().is_empty();
        let progress = is_progress_row(Some(&given));
        let tools_redraw = self.pty && given.fresh && !self.after_area;
        if progress {
            self.redrawn = false;
        } else if tools_redraw {
            self.redrawn = true;
        }
        let foldable = !blank && !progress && !given.fresh && !given.returns && !self.redrawn;
        if foldable && self.prev.as_ref() == Some(&text) {
            self.repeats += 1;
            self.folded = record.to_vec();
            return None;
        }
        if self.pty && blank && given.returns && !given.fresh && self.repeats > 0 {
            // The `\r` a tool writes before moving up: the run goes on.
            return Some(record.to_vec());
        }
        if tools_redraw && self.repeats > 0 {
            // A tool's redraw: its cursor-up counts the copies.
            let mut out = self.unfolded();
            out.extend_from_slice(record);
            self.prev = None;
            return Some(out);
        }
        // A run ends on a foldable record, so curses' erase (a fresh record
        // after its area) never meets a pending count.
        let out = self.with_count(record);
        if !blank {
            self.after_area = progress;
        }
        self.prev = (!blank && !progress && !given.returns).then_some(text);
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
    /// Bazel's errors, for the task's failure summary, once the console has
    /// ended ([`crate::console::report::RecapErrors::keeping`]).
    errors: Mutex<Option<KeptErrors>>,
}

/// Bazel's distinct `ERROR:` messages, as the recap would have repeated
/// them: the first `max`, and how many there were in all.
#[derive(Debug, Clone, Default)]
pub struct KeptErrors {
    pub errors: Vec<KeptError>,
    pub distinct: usize,
}

/// One error: its header as Bazel wrote it (cut after a failed action's exit
/// code), what the recap picked of its body as quoted (no style, no links),
/// each count of the rest a line (`… 3 more lines`), where the history kept
/// the action's output, and the label of that kept action, when one was
/// matched.
#[derive(Debug, Clone, Default)]
pub struct KeptError {
    pub header: String,
    pub body: Vec<String>,
    pub log: Option<String>,
    pub label: Option<String>,
}

impl Observations {
    /// Keep the console's errors for the failure summary.
    pub fn set_errors(&self, errors: KeptErrors) {
        *self.errors.lock().unwrap() = Some(errors);
    }

    /// The console's errors, once it has ended, when a stage kept them.
    pub fn errors(&self) -> Option<KeptErrors> {
        self.errors.lock().unwrap().clone()
    }

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
    fn collapse_repeats_never_folds_bazels_status_line() {
        let status: &[u8] = b"[3 / 9] 2 actions running";
        let mut p = CollapseRepeats::default();
        assert_eq!(
            feed(&mut p, &[b"a", b"a", status, status, b"a", b"a"]),
            [
                &b"a\n(last line repeated 1 more time)\n"[..],
                status,
                b"\n",
                status,
                b"\na\n(last line repeated 1 more time)\n",
            ]
            .concat()
        );
    }

    fn pty_repeats() -> CollapseRepeats {
        CollapseRepeats::default().under_pty(true)
    }

    const AREA: &[u8] = b"\x1b[32m[1 / 4]\x1b[0m 3 actions running";
    const ERASE: &[u8] = b"\x1b[1A\x1b[K";

    /// Under a PTY the count goes before the progress area curses draws after
    /// the run, at once, rather than waiting for the next output.
    #[test]
    fn under_a_pty_the_count_goes_before_the_progress_area() {
        let out = feed(&mut pty_repeats(), &[b"a", b"a", b"a", AREA]);
        assert_eq!(
            out,
            [&b"a\n(last line repeated 2 more times)\n"[..], AREA, b"\n"].concat()
        );
    }

    /// A count goes before the area curses draws after the run, so curses'
    /// erase and redraw of it later come through as they were.
    #[test]
    fn under_a_pty_curses_erase_comes_through_whole() {
        let area_in_place: &[u8] = &[ERASE, AREA].concat();
        let next: &[u8] = &[ERASE, b"\x1b[32mINFO: \x1b[0mdone"].concat();
        let mut p = pty_repeats();
        let mut out = feed(&mut p, &[b"a", b"a", AREA]);
        out.extend(feed(&mut p, &[area_in_place, b"\r", ERASE, next]));
        assert_eq!(
            out,
            [
                &b"a\n(last line repeated 1 more time)\n"[..],
                AREA,
                b"\n",
                area_in_place,
                b"\n\r\n",
                ERASE,
                b"\n",
                next,
                b"\n",
            ]
            .concat()
        );
    }

    /// Once a tool has redrawn its own lines, nothing folds until Bazel's next
    /// progress row: the tool's later redraws count the rows it wrote.
    #[test]
    fn under_a_pty_nothing_folds_after_a_tools_redraw_until_the_area() {
        let redraw: &[u8] = &[ERASE, b"  step 2"].concat();
        let out = feed(
            &mut pty_repeats(),
            &[redraw, b"  waiting", b"  waiting", AREA, b"x", b"x"],
        );
        assert_eq!(
            out,
            [
                redraw,
                b"\n  waiting\n  waiting\n",
                AREA,
                b"\nx\n(last line repeated 1 more time)\n",
            ]
            .concat()
        );
    }

    /// A redraw whose cursor-up comes as a record of its own, before a bare
    /// `\r` (indicatif's frames), still stops folding until the area.
    #[test]
    fn under_a_pty_a_bare_cursor_up_stops_folding_too() {
        let mut p = pty_repeats();
        let mut out = Vec::new();
        for (record, returns) in [
            (&b"\x1b[4A"[..], true),
            (b"\x1b[2Ka", false),
            (b"\x1b[2Ka", false),
        ] {
            let mut line = line::Classifier::default().classify(record);
            line.returns = returns;
            p.see(&line);
            if let Some(bytes) = p.process(record) {
                out.extend(bytes);
                out.push(b'\n');
            }
        }
        assert_eq!(
            out, b"\x1b[4A\n\x1b[2Ka\n\x1b[2Ka\n",
            "nothing folds after the redraw"
        );
    }

    /// The `\r` a tool writes before moving up over a run does not end it, so
    /// the copies are written back before the redraw that counts them.
    #[test]
    fn under_a_pty_a_tools_carriage_return_keeps_the_run_pending() {
        let mut p = pty_repeats();
        let mut out = Vec::new();
        for (record, returns) in [
            (&b"wait"[..], false),
            (b"wait", false),
            (b"wait", false),
            (b"", true),
            (b"\x1b[1A\x1b[Kdone", false),
        ] {
            let mut line = line::Classifier::default().classify(record);
            line.returns = returns;
            p.see(&line);
            if let Some(bytes) = p.process(record) {
                out.extend(bytes);
                out.push(b'\n');
            }
        }
        assert_eq!(out, b"wait\n\nwait\nwait\n\x1b[1A\x1b[Kdone\n");
    }

    /// Before a tool redrawing its own line, whose cursor-up counts the
    /// copies, they are written out as they came rather than counted.
    #[test]
    fn under_a_pty_a_tools_redraw_gets_the_copies_back() {
        let redraw: &[u8] = &[ERASE, b"step 2"].concat();
        let out = feed(
            &mut pty_repeats(),
            &[b"a", b"\x1b[1ma\x1b[0m", redraw, b"b"],
        );
        assert_eq!(
            out,
            [&b"a\n\x1b[1ma\x1b[0m\n"[..], redraw, b"\nb\n"].concat()
        );
    }

    /// The first line curses writes after erasing its area starts a run, its
    /// cursor movement notwithstanding.
    #[test]
    fn under_a_pty_a_line_after_curses_erase_starts_a_run() {
        let first: &[u8] = &[ERASE, b"a"].concat();
        let out = feed(&mut pty_repeats(), &[AREA, first, b"a", b"a"]);
        assert_eq!(
            out,
            [AREA, b"\n", first, b"\n(last line repeated 2 more times)\n"].concat()
        );
    }

    /// A colour or a `--show_timestamps` prefix does not tell two lines apart.
    #[test]
    fn collapse_repeats_compares_text() {
        let out = feed(
            &mut CollapseRepeats::default(),
            &[b"\x1b[31ma\x1b[0m", b"a", b"(09:00:01) a", b"b"],
        );
        assert_eq!(
            out,
            b"\x1b[31ma\x1b[0m\n(last line repeated 2 more times)\nb\n"
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
