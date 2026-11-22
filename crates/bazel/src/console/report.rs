//! What the console says once it ends: the stages that only report
//! ([`RecapErrors`]) and the [`Summary`] of what the others hid, with the
//! raw log and the files of each collapsed kind.
//!
//! Each writes its lines from [`LineProcessor::report`], after every stage has
//! flushed what it held, so the report follows bazel's last line. A report
//! line is styled with [`Style`], dim on a terminal or a recognized CI log, so
//! it reads as aspect's rather than bazel's.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;

use super::ansi;
use super::filters::Collapsed;
use super::forward::LineProcessor;
use super::line::{self, Given, Kind, Line};
use super::log::{self, Cut, RawLog};

/// The longest line a report quotes.
pub const MAX_QUOTED_BYTES: usize = 300;

/// Collapsed kinds a [`Summary`] lists, most hidden first.
pub const MAX_LISTED_KINDS: usize = 10;

/// What the hiding stages did, for the [`Summary`].
///
/// With [`Stats::keep_groups`], every message of a collapsed kind, bodies
/// included, also goes to a file of its own beside the raw log, as plain
/// text. A kind's first message waits in memory until a second like it is
/// hidden, so a kind that hid nothing leaves no file; from then on its lines
/// are appended to `<log>.collapsed-s<slot>.log`, which
/// [`Stats::finish_groups`] renames to `<log>.collapsed-<n>.log`, numbered
/// most hidden first, once the console ends.
#[derive(Debug, Default)]
pub struct Stats {
    hidden: AtomicU64,
    templates: Mutex<Vec<Collapsed>>,
    /// The raw log the group files go beside, when there is one.
    groups: Mutex<Option<PathBuf>>,
    /// Per kind, as `templates`: its lines not yet written (the first
    /// message's, until the kind hides one), and whether writing failed.
    pending: Mutex<Vec<(Vec<u8>, bool)>>,
}

impl Stats {
    /// Count `n` records hidden.
    pub fn hid(&self, n: u64) {
        self.hidden.fetch_add(n, Ordering::Relaxed);
    }

    /// Keep each collapsed kind's messages in a file beside `log`.
    pub fn keep_groups(&self, log: &Path) {
        *self.groups.lock().unwrap() = Some(log.to_path_buf());
    }

    /// Start a collapsed kind first shown as `first`; its slot.
    pub fn template(&self, first: &str) -> usize {
        let mut templates = self.templates.lock().unwrap();
        templates.push(Collapsed {
            first: quote(first),
            hidden: 0,
            file: None,
            incomplete: false,
        });
        self.pending.lock().unwrap().push((vec![], false));
        templates.len() - 1
    }

    /// Count one record of the kind in `slot` hidden.
    pub fn collapse(&self, slot: usize) {
        self.hid(1);
        self.templates.lock().unwrap()[slot].hidden += 1;
    }

    /// Add `record`, a line of a message of the kind in `slot`, to that
    /// kind's file. Best effort: a write that fails marks the file
    /// incomplete, which the summary says.
    pub fn group_line(&self, slot: usize, record: &[u8]) {
        let Some(log) = self.groups.lock().unwrap().clone() else {
            return;
        };
        let hid = self.templates.lock().unwrap()[slot].hidden > 0;
        let mut pending = self.pending.lock().unwrap();
        let (held, failed) = &mut pending[slot];
        held.extend_from_slice(ansi::text(record).as_bytes());
        held.push(b'\n');
        if !hid {
            return;
        }
        let path = staging(&log, slot);
        if let Err(e) = log::append_private(&path).and_then(|mut f| f.write_all(held)) {
            tracing::debug!("collapsed console group {} failed: {e}", path.display());
            *failed = true;
        }
        held.clear();
    }

    /// Records hidden, bodies included.
    pub fn hidden(&self) -> u64 {
        self.hidden.load(Ordering::Relaxed)
    }

    /// Give each kind that hid something its final file name, most hidden
    /// first. Once, at the end of the console.
    fn finish_groups(&self) {
        let Some(log) = self.groups.lock().unwrap().clone() else {
            return;
        };
        let mut templates = self.templates.lock().unwrap();
        let mut pending = self.pending.lock().unwrap();
        let mut order: Vec<usize> = (0..templates.len())
            .filter(|&s| templates[s].hidden > 0)
            .collect();
        order.sort_by_key(|&s| std::cmp::Reverse(templates[s].hidden));
        for (n, slot) in order.into_iter().enumerate() {
            let from = staging(&log, slot);
            let (held, failed) = &mut pending[slot];
            if !held.is_empty()
                && log::append_private(&from)
                    .and_then(|mut f| f.write_all(held))
                    .is_err()
            {
                *failed = true;
            }
            held.clear();
            let to = RawLog::sibling(&log, &format!("collapsed-{}", n + 1));
            if std::fs::rename(&from, &to).is_ok() {
                templates[slot].file = Some(to);
                templates[slot].incomplete = pending[slot].1;
            }
        }
    }

    /// The kinds that hid something, most hidden first.
    pub fn collapsed(&self) -> Vec<Collapsed> {
        let templates = self.templates.lock().unwrap();
        let mut kinds: Vec<Collapsed> =
            templates.iter().filter(|t| t.hidden > 0).cloned().collect();
        kinds.sort_by_key(|k| std::cmp::Reverse(k.hidden));
        kinds
    }
}

/// `path` as an OSC 8 hyperlink to it, its text the path itself.
pub fn file_link(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let shown = path.display().to_string();
    let mut url = String::from("file://");
    url.push_str(&hostname());
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    format!("\x1b]8;;{url}\x1b\\{shown}\x1b]8;;\x1b\\")
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is valid for its length; gethostname NUL-terminates what
    // fits, and a failure leaves the name empty, a link to the local host.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
    if ok {
        String::from_utf8_lossy(&buf[..end]).into_owned()
    } else {
        String::new()
    }
}

/// How report lines are set apart from bazel's: wrapped in `open`/`close`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Style {
    pub open: String,
    pub close: String,
}

impl Style {
    /// Dim grey, which every terminal and CI log viewer we ship to renders.
    pub fn dim() -> Style {
        Style {
            open: "\x1b[38;5;244m".into(),
            close: "\x1b[0m".into(),
        }
    }

    fn line(&self, out: &mut Vec<u8>, text: &str) {
        out.extend_from_slice(self.open.as_bytes());
        out.extend_from_slice(text.as_bytes());
        out.extend_from_slice(self.close.as_bytes());
        out.push(b'\n');
    }
}

/// `text` cut to at most `max` bytes, on a char boundary.
pub fn cut(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// `text` cut to [`MAX_QUOTED_BYTES`], marked when cut.
pub fn quote(text: &str) -> String {
    let text = text.trim_end();
    let quoted = cut(text, MAX_QUOTED_BYTES);
    if quoted.len() == text.len() {
        text.to_string()
    } else {
        format!("{quoted}…")
    }
}

/// Bazel's own closing lines, which a recap would only repeat.
static BOILERPLATE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"^ERROR: (?:Build did NOT complete successfully|Build failed\. Not running tests)")
        .unwrap()
});

/// [`RecapErrors`]' `after` by default: lines after the first error for a
/// recap to be worth printing, below which the error is likely still on
/// screen.
pub const RECAP_AFTER_RECORDS: usize = 40;

/// [`RecapErrors`]' `max` by default.
pub const RECAP_MAX: usize = 10;

/// How many lines of an error's body (a failed action's output under it) a
/// recap repeats with it.
pub const RECAP_BODY_LINES: usize = 5;

/// A failed action's error header up to its exit code: what follows (the
/// tool that failed, its whole command line) the recap leaves out.
static EXIT_TAIL: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^(.*? failed: \(Exit [^)]*\)): .+$").unwrap());

/// A line that starts a message of bazel's own, which ends the body before
/// it even where the classifier takes it for output (the first message after
/// an action's, in plain lines, where no redraw is left to end the output).
static SEVERITY: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^(?:ERROR|WARNING|INFO|DEBUG|FATAL): ").unwrap());

/// Repeats the first `max` distinct `ERROR:` messages of bazel's, at the end,
/// once `after` lines followed the first (blank lines and the rows curses
/// draws do not count, so curses and plain lines count alike), enough to
/// have scrolled it away. Its header says whether there were more than it
/// repeats, never how many: in a pipe some are not recognized. Each comes
/// with its body (what says what went wrong, for a failed action), up to
/// [`RECAP_BODY_LINES`] lines: the first ones, and the last (a Starlark
/// traceback's error comes after its frames), the rest counted between; its
/// header is cut after the exit code.
pub struct RecapErrors {
    max: usize,
    after: usize,
    style: Style,
    given: Given,
    errors: Vec<Recapped>,
    /// The error whose body the lines now coming are, while they are.
    body: Option<usize>,
    distinct: HashSet<String>,
    after_first: usize,
}

/// One error a recap repeats: its header, and what it keeps of its body.
struct Recapped {
    header: String,
    head: Vec<String>,
    last: Option<String>,
    skipped: usize,
}

impl Recapped {
    fn body_line(&mut self, text: String) {
        if self.head.len() < RECAP_BODY_LINES - 1 {
            self.head.push(text);
        } else if self.last.replace(text).is_some() {
            self.skipped += 1;
        }
    }
}

impl RecapErrors {
    pub fn new(max: usize, after: usize, style: Style) -> Self {
        Self {
            max,
            after,
            style,
            given: Given::default(),
            errors: vec![],
            body: None,
            distinct: HashSet::new(),
            after_first: 0,
        }
    }
}

impl LineProcessor for RecapErrors {
    fn see(&mut self, line: &Line) {
        self.given.see(line);
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        let line = self.given.take(record);
        // The text as the stages before left it (rewritten, redacted,
        // relativized), not as bazel wrote it.
        let text = line::message_of(record);
        let text = text.trim_end();
        // A header only: a tool's coloured `ERROR:` is not bazel's. In a pipe
        // an error right after a failed action's output is not one until
        // bazel's next progress line, so a recap may miss it, never invent one.
        let body = self.body.take();
        if line.kind == Kind::Header && text.starts_with("ERROR: ") && !BOILERPLATE.is_match(text) {
            let short = EXIT_TAIL
                .captures(text)
                .map_or(text, |c| c.get(1).unwrap().as_str());
            let quoted = quote(short);
            if self.distinct.insert(quoted.clone()) && self.errors.len() < self.max {
                self.errors.push(Recapped {
                    header: quoted,
                    head: vec![],
                    last: None,
                    skipped: 0,
                });
                self.body = Some(self.errors.len() - 1);
            }
        } else if let Some(i) = body
            && matches!(line.kind, Kind::Other | Kind::Blank)
            && !line.drawn
            && !SEVERITY.is_match(text)
        {
            if line.kind == Kind::Other {
                self.errors[i].body_line(quote(text));
            }
            self.body = Some(i);
            self.after_first += usize::from(line.kind == Kind::Other);
        } else if !self.distinct.is_empty()
            && matches!(line.kind, Kind::Header | Kind::Other)
            && !line.drawn
        {
            // Rows curses draws are not counted: they never push anything
            // off screen, and are gone from plain lines, which must count
            // the same.
            self.after_first += 1;
        }
        Some(record.to_vec())
    }

    fn report(&mut self) -> Option<Vec<u8>> {
        if self.errors.is_empty() || self.after_first < self.after {
            return None;
        }
        let mut out = Vec::new();
        // No total: in a pipe an error right after a failed action's output
        // is not recognized as one, so the count could only understate it.
        let header = if self.distinct.len() > self.errors.len() {
            match self.errors.len() {
                1 => "INFO: The first error Bazel reported:".to_string(),
                n => format!("INFO: The first {n} errors Bazel reported:"),
            }
        } else {
            "INFO: Errors Bazel reported:".to_string()
        };
        self.style.line(&mut out, &header);
        for e in &self.errors {
            out.extend_from_slice(format!("  {}\n", e.header).as_bytes());
            for b in &e.head {
                out.extend_from_slice(format!("    {b}\n").as_bytes());
            }
            if e.skipped > 0 {
                let s = if e.skipped == 1 { "" } else { "s" };
                out.extend_from_slice(format!("    … {} more line{s}\n", e.skipped).as_bytes());
            }
            if let Some(last) = &e.last {
                out.extend_from_slice(format!("    {last}\n").as_bytes());
            }
        }
        Some(out)
    }
}

/// Where a collapsed kind's messages are staged, by its slot in the stats,
/// until the summary renames the file by how it ranks (`collapsed-<n>`).
fn staging(log: &Path, slot: usize) -> PathBuf {
    RawLog::sibling(log, &format!("collapsed-s{slot}"))
}

/// What the summary adds to a file a write to it failed for.
const INCOMPLETE: &str = " (incomplete: writing it failed)";

/// Where the console as bazel wrote it was kept, for the [`Summary`].
#[derive(Debug, Clone)]
pub enum RawLogState {
    /// None was asked for.
    Off,
    /// One was, but it could not be created.
    Failed,
    /// The log, and whether writing it failed part-way ([`Cut`]).
    Kept(PathBuf, std::sync::Arc<std::sync::atomic::AtomicU8>),
}

/// The last stage of a pipeline that keeps a raw log or can hide lines:
/// says how many were hidden, lists the collapsed kinds with the file each
/// is in, and names the raw log whenever one was kept. Silent with no log
/// and nothing hidden; a progress line hidden is not counted. `links`
/// writes each path as an OSC 8 hyperlink, for a terminal.
pub struct Summary {
    stats: std::sync::Arc<Stats>,
    log: RawLogState,
    style: Style,
    links: bool,
}

impl Summary {
    pub fn new(stats: std::sync::Arc<Stats>, log: RawLogState, style: Style, links: bool) -> Self {
        Self {
            stats,
            log,
            style,
            links,
        }
    }

    fn path(&self, path: &Path) -> String {
        if self.links {
            file_link(path)
        } else {
            path.display().to_string()
        }
    }
}

impl LineProcessor for Summary {
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        Some(record.to_vec())
    }

    fn report(&mut self) -> Option<Vec<u8>> {
        self.stats.finish_groups();
        let hidden = self.stats.hidden();
        let log = match &self.log {
            RawLogState::Kept(path, cut) => {
                let cut = match Cut::from_code(cut.load(Ordering::Relaxed)) {
                    Cut::None => "",
                    Cut::Failed => INCOMPLETE,
                };
                Some(format!("the raw console{cut} is in {}", self.path(path)))
            }
            RawLogState::Failed => Some("the raw console could not be saved".to_string()),
            RawLogState::Off => None,
        };
        let mut out = Vec::new();
        match (hidden, log) {
            (0, None) => return None,
            (0, Some(log)) => self.style.line(&mut out, &format!("INFO: {log}")),
            (n, log) => {
                let s = if n == 1 { "" } else { "s" };
                let log = log.map(|l| format!("; {l}")).unwrap_or_default();
                self.style.line(
                    &mut out,
                    &format!("INFO: aspect hid {n} console line{s}{log}"),
                );
            }
        }
        let kinds = self.stats.collapsed();
        for k in kinds.iter().take(MAX_LISTED_KINDS) {
            self.style
                .line(&mut out, &format!("  {} more like: {}", k.hidden, k.first));
            if let Some(file) = &k.file {
                let incomplete = if k.incomplete { INCOMPLETE } else { "" };
                self.style.line(
                    &mut out,
                    &format!(
                        "    all {} in {}{incomplete}",
                        k.hidden + 1,
                        self.path(file)
                    ),
                );
            }
        }
        if kinds.len() > MAX_LISTED_KINDS {
            let more = kinds.len() - MAX_LISTED_KINDS;
            let filed = kinds[MAX_LISTED_KINDS..].iter().any(|k| k.file.is_some());
            let files = match (&self.log, filed) {
                (RawLogState::Kept(path, _), true) => {
                    format!(", in {}", RawLog::sibling(path, "collapsed-*").display())
                }
                _ => String::new(),
            };
            self.style
                .line(&mut out, &format!("  and {more} more kinds of line{files}"));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::console::line::tests::bazel;

    /// Run `records` through `p`, coloured as bazel colours its messages
    /// ([`bazel`]), and what it reports.
    fn feed(p: &mut dyn LineProcessor, records: &[&str]) -> String {
        for r in records {
            let r = bazel(r);
            assert_eq!(p.process(r.as_bytes()).as_deref(), Some(r.as_bytes()));
        }
        String::from_utf8(p.report().unwrap_or_default()).unwrap()
    }

    #[test]
    fn recap_repeats_errors_that_scrolled_away() {
        let mut p = RecapErrors::new(2, RECAP_AFTER_RECORDS, Style::default());
        let mut records = vec![
            // A failed action's error opens its output, which bazel's next
            // progress line ends.
            "\x1b[31mERROR: \x1b[0m/ws/BUILD:1:1: Compiling a.cc failed",
            "\x1b[32m[1 / 9]\x1b[0m running",
            "ERROR: /ws/BUILD:1:1: Compiling a.cc failed",
            "\x1b[32m[1 / 9]\x1b[0m running",
            "ERROR: /ws/BUILD:2:1: Compiling b.cc failed",
            "\x1b[32m[1 / 9]\x1b[0m running",
            "ERROR: /ws/BUILD:3:1: Compiling c.cc failed",
            "\x1b[32m[1 / 9]\x1b[0m running",
        ];
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        records.extend(std::iter::repeat_n(
            "\x1b[32mLoading:\x1b[0m 1 packages loaded",
            5,
        ));
        records.push("ERROR: Build did NOT complete successfully");
        assert_eq!(
            feed(&mut p, &records),
            "INFO: The first 2 errors Bazel reported:\n  \
             ERROR: /ws/BUILD:1:1: Compiling a.cc failed\n  \
             ERROR: /ws/BUILD:2:1: Compiling b.cc failed\n"
        );
    }

    #[test]
    fn recap_repeats_an_errors_body_and_cuts_its_command() {
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::default());
        let mut records = vec![
            "\x1b[31mERROR: \x1b[0m/ws/BUILD:1:1: Executing genrule //dep:bad failed: (Exit 3): bash failed: error executing Genrule command /bin/bash -c 'echo boom; exit 3'",
            "boom",
            "",
            "app/x.cc:3: error: what went wrong",
        ];
        records.extend(std::iter::repeat_n("more output", RECAP_BODY_LINES));
        records.push("\x1b[32m[1 / 9]\x1b[0m running");
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        assert_eq!(
            feed(&mut p, &records),
            "INFO: Errors Bazel reported:\n  \
             ERROR: /ws/BUILD:1:1: Executing genrule //dep:bad failed: (Exit 3)\n    \
             boom\n    app/x.cc:3: error: what went wrong\n    more output\n    \
             more output\n    … 2 more lines\n    more output\n"
        );
    }

    #[test]
    fn a_recapped_body_ends_at_bazels_next_message_and_keeps_its_last_line() {
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::default());
        let mut records = vec![
            "ERROR: Traceback (most recent call last):",
            "\tFile \"/ws/a.bzl\", line 1",
            "\tFile \"/ws/b.bzl\", line 2",
            "\tFile \"/ws/c.bzl\", line 3",
            "\tFile \"/ws/d.bzl\", line 4",
            "\tFile \"/ws/e.bzl\", line 5",
            "Error in fail: the real reason",
            // In plain lines nothing erased is left to end the body.
            "WARNING: Remote Cache: UNAVAILABLE",
        ];
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        assert_eq!(
            feed(&mut p, &records),
            "INFO: Errors Bazel reported:\n  \
             ERROR: Traceback (most recent call last):\n    \
             \tFile \"/ws/a.bzl\", line 1\n    \tFile \"/ws/b.bzl\", line 2\n    \
             \tFile \"/ws/c.bzl\", line 3\n    \tFile \"/ws/d.bzl\", line 4\n    \
             … 1 more line\n    Error in fail: the real reason\n"
        );
    }

    #[test]
    fn recap_stays_quiet_while_the_error_is_on_screen() {
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::default());
        let mut records = vec!["ERROR: one"];
        records.extend(std::iter::repeat_n(
            "\x1b[32m[1 / 2]\x1b[0m Compiling",
            RECAP_AFTER_RECORDS,
        ));
        records.extend(std::iter::repeat_n("  ", RECAP_AFTER_RECORDS));
        assert_eq!(
            feed(&mut p, &records),
            "",
            "progress and blank lines do not count"
        );
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::default());
        assert_eq!(
            feed(&mut p, &["ERROR: Build did NOT complete successfully"]),
            ""
        );
    }

    #[test]
    fn summary_lists_what_was_hidden_and_where_the_log_is() {
        let stats = Arc::new(Stats::default());
        let mut p = Summary::new(
            stats.clone(),
            RawLogState::Kept("/tmp/c.log".into(), Default::default()),
            Style::default(),
            false,
        );
        assert_eq!(
            feed(&mut p, &["a"]),
            "INFO: the raw console is in /tmp/c.log\n",
            "a kept log is named though nothing was hidden"
        );

        let few = stats.template("WARNING: few");
        let many = stats.template("WARNING: many");
        stats.template("WARNING: shown once");
        stats.collapse(few);
        for _ in 0..3 {
            stats.collapse(many);
        }
        stats.hid(2);
        assert_eq!(
            feed(&mut p, &[]),
            "INFO: aspect hid 6 console lines; the raw console is in /tmp/c.log\n  \
             3 more like: WARNING: many\n  1 more like: WARNING: few\n"
        );
    }

    #[test]
    fn recap_counts_each_distinct_error_once() {
        let mut p = RecapErrors::new(1, RECAP_AFTER_RECORDS, Style::default());
        let mut records = vec![];
        for e in ["ERROR: A", "ERROR: B", "ERROR: B", "ERROR: B"] {
            records.extend([e, "\x1b[32m[1 / 9]\x1b[0m running"]);
        }
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        assert_eq!(
            feed(&mut p, &records),
            "INFO: The first error Bazel reported:\n  ERROR: A\n"
        );
    }

    /// Curses redraws its progress area in place, so its lines do not push
    /// an error off screen.
    #[test]
    fn recap_ignores_the_progress_area_under_a_pty() {
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::default());
        let mut records = vec!["ERROR: one"];
        for _ in 0..RECAP_AFTER_RECORDS {
            records.extend([
                "\x1b[1A\x1b[K",
                "\x1b[1A\x1b[K",
                "\x1b[1A\x1b[K\x1b[32m[3 / 9]\x1b[0m 2 actions running",
                "    Compiling a.cc; 1s local",
                "    Compiling b.cc; 1s local",
            ]);
        }
        assert_eq!(feed(&mut p, &records), "");
    }

    #[test]
    fn summary_names_no_log_when_none_was_configured() {
        let stats = Arc::new(Stats::default());
        stats.hid(1);
        let mut p = Summary::new(stats.clone(), RawLogState::Off, Style::default(), false);
        assert_eq!(feed(&mut p, &[]), "INFO: aspect hid 1 console line\n");
        let mut p = Summary::new(
            Arc::new(Stats::default()),
            RawLogState::Off,
            Style::default(),
            false,
        );
        assert_eq!(
            feed(&mut p, &[]),
            "",
            "silent with no log and nothing hidden"
        );
    }

    /// A log that could not be made, or stopped short, is said so.
    #[test]
    fn summary_says_when_the_log_failed() {
        let p = |state| Summary::new(Arc::new(Stats::default()), state, Style::default(), false);
        assert_eq!(
            feed(&mut p(RawLogState::Failed), &[]),
            "INFO: the raw console could not be saved\n"
        );
        let cut = Arc::new(std::sync::atomic::AtomicU8::new(Cut::Failed as u8));
        assert_eq!(
            feed(&mut p(RawLogState::Kept("/tmp/c.log".into(), cut)), &[]),
            "INFO: the raw console (incomplete: writing it failed) is in /tmp/c.log\n"
        );
    }

    /// Past the kinds it lists, the summary counts the rest and points at
    /// their files, beside the log.
    #[test]
    fn summary_counts_the_kinds_it_does_not_list() {
        let stats = Arc::new(Stats::default());
        for i in 0..MAX_LISTED_KINDS + 2 {
            let slot = stats.template(&format!("WARNING: kind {i}"));
            stats.collapse(slot);
        }
        let mut p = Summary::new(
            stats,
            RawLogState::Kept("/tmp/c.log".into(), Default::default()),
            Style::default(),
            false,
        );
        let out = feed(&mut p, &[]);
        assert!(out.ends_with("  and 2 more kinds of line\n"), "{out}");
    }

    /// Every message of a collapsed kind, body included, lands in its file,
    /// numbered most hidden first; a kind that hid nothing leaves none.
    #[test]
    fn each_collapsed_kind_gets_a_file_beside_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("console-1-2-3.log");
        std::fs::write(&log, b"").unwrap();
        let stats = Arc::new(Stats::default());
        stats.keep_groups(&log);
        let few = stats.template("WARNING: few 1");
        let many = stats.template("WARNING: many 1");
        let once = stats.template("WARNING: once");
        stats.group_line(few, b"\x1b[35mWARNING: \x1b[0mfew 1");
        stats.group_line(once, b"WARNING: once");
        for i in 1..=3 {
            stats.group_line(many, format!("WARNING: many {i}").as_bytes());
            stats.group_line(many, b"  under it");
        }
        stats.collapse(few);
        stats.group_line(few, b"WARNING: few 2");
        for _ in 0..2 {
            stats.collapse(many);
        }
        let mut p = Summary::new(
            stats,
            RawLogState::Kept(log.clone(), Default::default()),
            Style::default(),
            false,
        );
        let out = feed(&mut p, &[]);
        let one = RawLog::sibling(&log, "collapsed-1");
        let two = RawLog::sibling(&log, "collapsed-2");
        assert_eq!(
            std::fs::read_to_string(&one).unwrap(),
            "WARNING: many 1\n  under it\nWARNING: many 2\n  under it\nWARNING: many 3\n  under it\n"
        );
        assert_eq!(
            std::fs::read_to_string(&two).unwrap(),
            "WARNING: few 1\nWARNING: few 2\n",
            "plain text"
        );
        assert!(!RawLog::sibling(&log, "collapsed-3").exists());
        assert!(
            !RawLog::sibling(&log, "collapsed-s2").exists(),
            "nothing hidden, no file"
        );
        assert_eq!(
            out,
            format!(
                "INFO: aspect hid 3 console lines; the raw console is in {}\n  \
                 2 more like: WARNING: many 1\n    all 3 in {}\n  \
                 1 more like: WARNING: few 1\n    all 2 in {}\n",
                log.display(),
                one.display(),
                two.display()
            )
        );
    }

    #[test]
    fn a_link_names_the_path_it_shows() {
        let link = file_link(Path::new("/tmp/a b.log"));
        assert!(link.starts_with("\x1b]8;;file://"), "{link:?}");
        assert!(
            link.contains("/tmp/a%20b.log\x1b\\/tmp/a b.log\x1b]8;;\x1b\\"),
            "{link:?}"
        );
    }

    #[test]
    fn quote_cuts_long_lines_on_a_char_boundary() {
        let long = "é".repeat(MAX_QUOTED_BYTES);
        let q = quote(&long);
        assert!(q.ends_with('…'));
        assert!(q.len() <= MAX_QUOTED_BYTES + '…'.len_utf8());
        assert_eq!(quote("short  "), "short");
    }
}
