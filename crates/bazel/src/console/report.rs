//! What the console says once it ends: the stages that only report
//! ([`RecapErrors`]) and the [`Summary`] of what the others hid, with the
//! raw log and the files of each collapsed kind.
//!
//! Each writes its lines from [`LineProcessor::report`], after every stage has
//! flushed what it held, so the report follows bazel's last line. A report
//! line is styled with [`Style`] on a terminal or a recognized CI log, so it
//! reads as aspect's rather than bazel's: an `INFO:` line as the CLI's own
//! are (its label in the theme's `label.info`, the text plain), the summary's
//! detail lines under it `muted`, the recap under a `── Error recap (N) ──`
//! rule in `label.error`; the recap's quoted errors stay as Bazel wrote them
//! but for each header's `ERROR` label, coloured as the live line's is, with
//! what they name linked on a terminal ([`super::links`]). Where the task
//! prints a failure summary instead (`ErrorsForSummary`), there is no recap
//! and the summary leaves the raw log for that summary to name
//! ([`Summary::log_named_elsewhere`]).

use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;

use super::ansi;
use super::excerpt;
use super::filters::Collapsed;
use super::forward::LineProcessor;
use super::line::{self, Given, Kind, Line};
use super::links::{FileLinks, Known, Linker, file_link, links_in};
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
/// are appended to `<log>.collapsed-s<slot>.log`, kept open and buffered
/// (a console may hide hundreds of thousands of lines) while it is among
/// the [`max_open_groups`] most recently written, which
/// [`Stats::finish_groups`] renames to `<log>.collapsed-<n>.log`, numbered
/// most hidden first, once the console ends.
#[derive(Debug, Default)]
pub struct Stats {
    hidden: AtomicU64,
    templates: Mutex<Vec<Collapsed>>,
    /// The raw log the group files go beside, when there is one.
    groups: Mutex<Option<PathBuf>>,
    /// Per kind, as `templates`: its lines not yet written (the first
    /// message's, until the kind hides one), whether writing failed, and its
    /// file while open; and which kinds' files are open, least recently
    /// written first, at most [`max_open_groups`].
    pending: Mutex<(Vec<Pending>, std::collections::VecDeque<usize>)>,
}

/// The most collapsed kinds' files held open at once: a quarter of the
/// process's descriptor limit (macOS's default is 256, Linux's 1024),
/// between 16 and 512. Past it the one least recently written is flushed
/// and closed, and opened again, appending, should its kind hide more.
fn max_open_groups() -> usize {
    static MAX: std::sync::LazyLock<usize> = std::sync::LazyLock::new(|| {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit only writes the struct it is given.
        let soft = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0 {
            limit.rlim_cur as u64
        } else {
            256
        };
        (soft / 4).clamp(16, 512) as usize
    });
    *MAX
}

/// One collapsed kind's file as it is written ([`Stats::group_line`]).
#[derive(Debug, Default)]
struct Pending {
    held: Vec<u8>,
    failed: bool,
    file: Option<std::io::BufWriter<std::fs::File>>,
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
        self.pending.lock().unwrap().0.push(Pending::default());
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
        let mut guard = self.pending.lock().unwrap();
        let (kinds, open) = &mut *guard;
        if kinds[slot].failed {
            return;
        }
        let p = &mut kinds[slot];
        p.held.extend_from_slice(ansi::text(record).as_bytes());
        p.held.push(b'\n');
        if !hid {
            return;
        }
        if kinds[slot].file.is_some() {
            // Least recently written last out: a kind in use stays open.
            if open.back() != Some(&slot)
                && let Some(at) = open.iter().position(|&s| s == slot)
            {
                open.remove(at);
                open.push_back(slot);
            }
        } else {
            while open.len() >= max_open_groups() {
                let Some(oldest) = open.pop_front() else {
                    break;
                };
                if let Some(mut file) = kinds[oldest].file.take()
                    && file.flush().is_err()
                {
                    kinds[oldest].failed = true;
                }
            }
            let path = staging(&log, slot);
            match log::append_private(&path) {
                Ok(f) => {
                    kinds[slot].file = Some(std::io::BufWriter::new(f));
                    open.push_back(slot);
                }
                // Kept, to be written once it can be: on the kind's next
                // line, or at the end.
                Err(e) => {
                    tracing::debug!("collapsed console group {} failed: {e}", path.display());
                    return;
                }
            }
        }
        let p = &mut kinds[slot];
        let held = std::mem::take(&mut p.held);
        if let Some(file) = p.file.as_mut()
            && let Err(e) = file.write_all(&held)
        {
            tracing::debug!("collapsed console group {slot} failed: {e}");
            p.failed = true;
        }
    }

    /// Records hidden, bodies included.
    pub fn hidden(&self) -> u64 {
        self.hidden.load(Ordering::Relaxed)
    }

    /// Give each kind that hid something its final file name, most hidden
    /// first. Once, at the end of the console.
    pub(crate) fn finish_groups(&self) {
        let Some(log) = self.groups.lock().unwrap().clone() else {
            return;
        };
        let mut templates = self.templates.lock().unwrap();
        let mut guard = self.pending.lock().unwrap();
        let (pending, open) = &mut *guard;
        open.clear();
        let mut order: Vec<usize> = (0..templates.len())
            .filter(|&s| templates[s].hidden > 0)
            .collect();
        order.sort_by_key(|&s| std::cmp::Reverse(templates[s].hidden));
        for (n, slot) in order.into_iter().enumerate() {
            let from = staging(&log, slot);
            let p = &mut pending[slot];
            let held = std::mem::take(&mut p.held);
            let wrote = match p.file.take() {
                Some(mut file) => file.write_all(&held).and_then(|()| file.flush()),
                None if held.is_empty() => Ok(()),
                None => log::append_private(&from).and_then(|mut f| f.write_all(&held)),
            };
            if wrote.is_err() {
                p.failed = true;
            }
            let to = RawLog::sibling(&log, &format!("collapsed-{}", n + 1));
            if std::fs::rename(&from, &to).is_ok() {
                templates[slot].file = Some(to);
                templates[slot].incomplete = pending[slot].failed;
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

/// How report lines are set apart from bazel's: an `INFO:` line's label in
/// `info`, as the CLI's own `INFO:` lines have it (`axl_runtime::diag`), a
/// detail line wrapped in `open`/`close`, the recap's rule in `recap` and the
/// `ERROR` label of each error it quotes in `error`. Empty strings leave a
/// line plain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Style {
    pub open: String,
    pub close: String,
    pub info: String,
    pub recap: String,
    pub error: String,
}

impl Style {
    /// The theme's `label.info` for the `INFO` label, `muted` for the
    /// detail under it, `label.error` for the recap's rule, and for a quoted
    /// `ERROR` label `bazel.error` when the theme recolours Bazel's lines,
    /// else the theme's own `label.error` (not Bazel's codes, which a
    /// `bazel: tool` theme leaves on the live lines).
    pub fn themed(palette: &aspect_theme::Palette) -> Style {
        let open = palette.sgr("muted");
        Style {
            close: if open.is_empty() {
                String::new()
            } else {
                aspect_theme::RESET.into()
            },
            open,
            info: palette.sgr("label.info"),
            recap: palette.sgr("label.error"),
            error: palette.sgr(if palette.bazel() == aspect_theme::Bazel::Tool {
                "label.error"
            } else {
                "bazel.error"
            }),
        }
    }

    /// `── text ──`, the recap's rule.
    fn rule(&self, out: &mut Vec<u8>, text: &str) {
        let line = format!("── {text} ──");
        if self.recap.is_empty() {
            out.extend_from_slice(line.as_bytes());
        } else {
            out.extend_from_slice(
                format!("{}{line}{}", self.recap, aspect_theme::RESET).as_bytes(),
            );
        }
        out.push(b'\n');
    }

    /// `line`, a quoted error's header, its leading `ERROR` label in `error`.
    fn error_label(&self, line: Vec<u8>) -> Vec<u8> {
        match line.strip_prefix(b"ERROR:") {
            Some(rest) if !self.error.is_empty() => {
                let mut out = self.error.as_bytes().to_vec();
                out.extend_from_slice(b"ERROR");
                out.extend_from_slice(aspect_theme::RESET.as_bytes());
                out.push(b':');
                out.extend_from_slice(rest);
                out
            }
            _ => line,
        }
    }

    fn line(&self, out: &mut Vec<u8>, text: &str) {
        match text.strip_prefix("INFO:") {
            Some(rest) if !self.info.is_empty() => {
                out.extend_from_slice(
                    aspect_theme::label_line(&self.info, "INFO", rest).as_bytes(),
                );
            }
            _ => {
                out.extend_from_slice(self.open.as_bytes());
                out.extend_from_slice(text.as_bytes());
                out.extend_from_slice(self.close.as_bytes());
            }
        }
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
    Regex::new(r"^ERROR: (?:Build did NOT complete successfully|Build failed\. Not running tests|command succeeded, but )")
        .unwrap()
});

/// [`RecapErrors`]' `after` by default: lines after the first error for a
/// recap to be worth printing, below which the error is likely still on
/// screen.
pub const RECAP_AFTER_RECORDS: usize = 40;

/// [`RecapErrors`]' `max` by default.
pub const RECAP_MAX: usize = 10;

/// How many lines of an error's body (a failed action's output under it) a
/// recap holds to choose what it repeats from: the first half, and the last
/// half after them.
const RECAP_BODY_HELD: usize = 400;

/// A failed action's error header up to its exit code, or the signal that
/// ended it as Bazel's `TerminationStatus` names it (`(Killed)`,
/// `(Trace/breakpoint trap)`, `(User defined signal 1)`, `(Signal 24)`):
/// what follows (the tool that failed, its whole command line) the recap
/// leaves out.
static EXIT_TAIL: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"^(.*? failed: \((?:Exit [^)]*|[A-Z][^()]*)\)): .+$").unwrap()
});

/// Bazel's hint under every sandboxed failure, which a recapped body leaves
/// out for a line of the tool's own.
const SANDBOX_HINT: &str = "Use --sandbox_debug to see verbose messages from the sandbox";

/// The history's note of where it kept a failed action's output, which the
/// console writes between Bazel's lines (`bes::sink::keep`): never part of
/// an error's body, which names the kept output itself.
static KEPT_NOTE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^INFO: \S+ \S+ std(?:err|out): ").unwrap());

/// Bazel's status row with nothing after its label (`FAILED:`), as curses
/// draws it: never part of an error's body.
static BARE_STATUS: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"^[A-Z]+:$").unwrap());

/// Repeats the first `max` distinct `ERROR:` messages of bazel's, at the end,
/// once `after` lines followed the first (blank lines and the rows curses
/// draws do not count, so curses and plain lines count alike), enough to
/// have scrolled it away. Its heading (`── Error recap (2) ──`, or `── Error
/// recap (first 2) ──`) counts those it repeats and says whether there were more,
/// never how many: in a pipe some are not recognized. Each comes
/// with its body (what says what went wrong, for a failed action): the lines
/// the status surfaces' failure snippets would pick ([`excerpt::Shape::RECAP`]:
/// the first that reads as an error, with context, and the last), the rest
/// counted, Bazel's sandbox hint left out. The body ends at Bazel's next redraw (a
/// record that is `fresh`, in plain lines as under a PTY), so a tool's own
/// `WARNING:` lines are kept in it. Its header is cut after the exit code.
pub struct RecapErrors {
    max: usize,
    after: usize,
    style: Style,
    given: Given,
    errors: Vec<Recapped>,
    /// The error whose body the lines now coming are, while they are.
    body: Option<usize>,
    /// The error whose body bazel's progress area broke off: its output's
    /// held last line ([`Line::tail`]), an `ERROR:` in bazel's colours that
    /// is not bazel's own, is still its.
    held: Option<usize>,
    distinct: HashSet<String>,
    after_first: usize,
    /// Links what the repeated lines name, for a terminal, with what
    /// `relativize_paths` said of each line's paths.
    linker: Option<Linker>,
}

/// The links a line went by with, placed in its quoted text.
type LineLinks = Vec<Known>;

/// One error a recap repeats: its header, and what it holds of its body
/// ([`RECAP_BODY_HELD`]), each line with the links it went by with.
struct Recapped {
    header: (String, LineLinks),
    first: Vec<(String, LineLinks)>,
    last: VecDeque<(String, LineLinks)>,
    /// Lines between `first` and `last`, held by neither.
    dropped: usize,
}

/// A part of a recapped body: a line it repeats, or a count of those it
/// does not.
enum Part<'a> {
    Line(&'a (String, LineLinks)),
    Gap(usize),
}

impl Recapped {
    fn new(header: (String, LineLinks)) -> Self {
        Recapped {
            header,
            first: vec![],
            last: VecDeque::new(),
            dropped: 0,
        }
    }

    fn body_line(&mut self, text: String, links: LineLinks) {
        if self.first.len() < RECAP_BODY_HELD / 2 {
            self.first.push((text, links));
            return;
        }
        self.last.push_back((text, links));
        if self.last.len() > RECAP_BODY_HELD / 2 {
            self.last.pop_front();
            self.dropped += 1;
        }
    }

    /// The body as the recap repeats it: [`excerpt::select`]'s lines, each
    /// run of the others a count, those it did not hold among them.
    fn excerpt(&self) -> Vec<Part<'_>> {
        let held: Vec<&(String, LineLinks)> = self.first.iter().chain(&self.last).collect();
        let texts: Vec<&str> = held.iter().map(|(t, _)| t.as_str()).collect();
        if let Some(parts) = self.traceback(&held) {
            return parts;
        }
        // Where the lines not held were, between `first` and `last`.
        let join = self.first.len();
        let unheld = |from: usize, to: usize| {
            if self.dropped > 0 && from <= join && join <= to {
                self.dropped
            } else {
                0
            }
        };
        let mut parts = vec![];
        let mut at = 0;
        for span in excerpt::select(&texts, excerpt::Shape::RECAP) {
            let gap = span.start - at + unheld(at, span.start);
            if gap > 0 {
                parts.push(Part::Gap(gap));
            }
            for i in span.clone() {
                if i == join && i > span.start && self.dropped > 0 {
                    parts.push(Part::Gap(self.dropped));
                }
                parts.push(Part::Line(held[i]));
            }
            at = span.end;
        }
        let gap = held.len() - at
            + if at < held.len() {
                unheld(at, held.len())
            } else {
                0
            };
        if gap > 0 {
            parts.push(Part::Gap(gap));
        }
        parts
    }
}

impl Recapped {
    /// A Starlark `Traceback`'s excerpt: the first frame at a BUILD file
    /// (`File "nm/BUILD.bazel", line 2`) and its code line, then the last
    /// `Error in …:` line, what is between a gap; None for another body.
    fn traceback<'a>(&self, held: &[&'a (String, LineLinks)]) -> Option<Vec<Part<'a>>> {
        let traceback = self.header.0.contains("Traceback (most recent call last):")
            || held
                .first()
                .is_some_and(|(t, _)| t.trim() == "Traceback (most recent call last):");
        let error = held
            .iter()
            .rposition(|(t, _)| t.trim_start().starts_with("Error in "))?;
        if !traceback || self.dropped > 0 {
            return None;
        }
        let build = held[..error].iter().position(|(t, _)| {
            let t = t.trim_start();
            t.starts_with("File \"")
                && (t.contains("/BUILD\"")
                    || t.contains("/BUILD.bazel\"")
                    || t.starts_with("File \"BUILD"))
        });
        let mut parts = vec![];
        let mut at = 0;
        let keep = |i: usize, parts: &mut Vec<Part<'a>>, at: &mut usize| {
            if i > *at {
                parts.push(Part::Gap(i - *at));
            }
            parts.push(Part::Line(held[i]));
            *at = i + 1;
        };
        if held[0].0.trim() == "Traceback (most recent call last):" {
            keep(0, &mut parts, &mut at);
        }
        if let Some(b) = build {
            keep(b, &mut parts, &mut at);
            if b + 1 < error {
                keep(b + 1, &mut parts, &mut at);
            }
        }
        keep(error, &mut parts, &mut at);
        if at < held.len() {
            parts.push(Part::Gap(held.len() - at));
        }
        Some(parts)
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
            held: None,
            distinct: HashSet::new(),
            after_first: 0,
            linker: None,
        }
    }

    /// This recap with the paths and URLs it repeats linked ([`Linker`]),
    /// a path a stage before linked (`relativize_paths`) to the file it did.
    pub fn linking(mut self, linker: Option<Linker>) -> Self {
        self.linker = linker;
        self
    }
}

impl RecapErrors {
    /// The links the stages before put in `record` (`relativize_paths`'),
    /// placed in `quoted`, the start of its message text as the recap
    /// quotes it, when linking: a link the quote cut off is left out.
    fn links(&self, record: &[u8], quoted: &str) -> LineLinks {
        if self.linker.is_none() {
            return Vec::new();
        }
        let stripped = ansi::strip(record);
        let text = String::from_utf8_lossy(&stripped);
        let start = text.len() - line::message(&text).len();
        let kept = quoted.strip_suffix('…').unwrap_or(quoted).len();
        links_in(record)
            .into_iter()
            .filter_map(|k| {
                // Where the link is in the text as decoded, a byte not UTF-8
                // before it decoded longer.
                let decoded = String::from_utf8_lossy(stripped.get(..k.at)?).len();
                let at = decoded.checked_sub(start)?;
                (at + k.shown.len() <= kept).then_some(Known { at, ..k })
            })
            .collect()
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
        let held = self.held.take();
        if line.kind == Kind::Header && text.starts_with("ERROR: ") && !BOILERPLATE.is_match(text) {
            let short = EXIT_TAIL
                .captures(text)
                .map_or(text, |c| c.get(1).unwrap().as_str());
            let quoted = quote(short);
            if self.distinct.insert(quoted.clone()) && self.errors.len() < self.max {
                let links = self.links(record, &quoted);
                self.errors.push(Recapped::new((quoted, links)));
                self.body = Some(self.errors.len() - 1);
            }
        } else if let Some(i) = body
            && matches!(line.kind, Kind::Other | Kind::Blank)
            && !line.drawn
            && !line.fresh
        {
            if line.kind == Kind::Other
                && !text.starts_with(SANDBOX_HINT)
                && !KEPT_NOTE.is_match(text)
                && !BARE_STATUS.is_match(text)
            {
                let quoted = quote(text);
                let links = self.links(record, &quoted);
                self.errors[i].body_line(quoted, links);
            }
            self.body = Some(i);
            self.after_first += usize::from(line.kind == Kind::Other);
        } else if let Some(i) = held.or(body)
            && line.tail
            && line.kind == Kind::Other
            && text.starts_with("ERROR: ")
        {
            // An `ERROR:` the classifier did not take for bazel's own is
            // the output's held last line; any other tail may be bazel's
            // next message.
            let quoted = quote(text);
            let links = self.links(record, &quoted);
            self.errors[i].body_line(quoted, links);
            self.after_first += 1;
        } else if !self.distinct.is_empty()
            && matches!(line.kind, Kind::Header | Kind::Other)
            && !line.drawn
        {
            // Rows curses draws are not counted: they never push anything
            // off screen, and are gone from plain lines, which must count
            // the same.
            self.after_first += 1;
        }
        if line.drawn || line.kind == Kind::Blank {
            self.held = held.or(body);
        }
        Some(record.to_vec())
    }

    fn report(&mut self) -> Option<Vec<u8>> {
        if self.errors.is_empty() || self.after_first < self.after {
            return None;
        }
        let mut out = Vec::new();
        // The count is of the errors shown, never a total: in a pipe an error
        // right after a failed action's output is not recognized as one, so
        // a total could only understate it.
        let n = self.errors.len();
        let header = if self.distinct.len() > n {
            format!("Error recap (first {n})")
        } else {
            format!("Error recap ({n})")
        };
        self.style.rule(&mut out, &header);
        for e in self.rendered() {
            for (i, l) in e.into_iter().enumerate() {
                out.extend_from_slice(if i == 0 { b"  " } else { b"    " });
                out.extend(l);
                out.push(b'\n');
            }
        }
        Some(out)
    }
}

impl RecapErrors {
    /// Each error's lines as the recap repeats them, without their indent:
    /// the header styled as an error, then what it picked of the body and
    /// each count of the rest.
    fn rendered(&mut self) -> Vec<Vec<Vec<u8>>> {
        let style = self.style.clone();
        let mut linker = self.linker.take();
        let mut line = |text: &str, links: &[Known]| match linker.as_mut() {
            Some(l) if l.files.text => {
                let linked = l.link_with(text.as_bytes(), links);
                l.files.write_urls(&linked)
            }
            Some(l) => l.link_with(text.as_bytes(), links),
            None => text.as_bytes().to_vec(),
        };
        let mut out = vec![];
        for e in &self.errors {
            let mut lines = vec![style.error_label(line(&e.header.0, &e.header.1))];
            for part in e.excerpt() {
                match part {
                    Part::Line((b, links)) => lines.push(line(b, links)),
                    Part::Gap(n) => {
                        let s = if n == 1 { "" } else { "s" };
                        lines.push(format!("… {n} more line{s}").into_bytes());
                    }
                }
            }
            out.push(lines);
        }
        self.linker = linker;
        out
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
/// and nothing hidden; a progress line hidden is not counted. With `links`
/// it writes each path as an OSC 8 hyperlink as they say, for a terminal.
pub struct Summary {
    stats: std::sync::Arc<Stats>,
    log: RawLogState,
    style: Style,
    links: Option<FileLinks>,
    /// Whether the task's failure summary names the raw log
    /// ([`Summary::log_named_elsewhere`]).
    elsewhere: bool,
}

impl Summary {
    pub fn new(
        stats: std::sync::Arc<Stats>,
        log: RawLogState,
        style: Style,
        links: Option<FileLinks>,
    ) -> Self {
        Self {
            stats,
            log,
            style,
            links,
            elsewhere: false,
        }
    }

    /// This summary leaving a raw log kept whole to the task's failure
    /// summary, which names it on its counts line (`… · Bazel's raw log:
    /// <path>`); one it could not keep, or kept cut short, it still says so
    /// of.
    pub fn log_named_elsewhere(mut self, elsewhere: bool) -> Self {
        self.elsewhere = elsewhere;
        self
    }

    /// `path`, linked when files are; `restore` is the style of the text
    /// around it, taken up again after a styled written URL.
    fn path(&self, path: &Path, restore: &str) -> String {
        match &self.links {
            Some(files) => file_link(files, path, restore),
            None => path.display().to_string(),
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
                // On the `INFO:` line, whose text is unstyled.
                (!self.elsewhere || !cut.is_empty())
                    .then(|| format!("Bazel's raw log{cut} is in {}", self.path(path, "")))
            }
            RawLogState::Failed => Some("Bazel's raw log could not be saved".to_string()),
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
                    &format!("INFO: Aspect hid {n} console line{s}{log}"),
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
                        self.path(file, &self.style.open)
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

    /// More kinds than are held open at once each keep every line, one
    /// whose file was closed meanwhile appending to it again.
    #[test]
    fn many_kinds_keep_every_line_with_few_files_open() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("console.log");
        let stats = Stats::default();
        stats.keep_groups(&log);
        let kinds = max_open_groups() + 8;
        for k in 0..kinds {
            let slot = stats.template(&format!("kind {k}"));
            stats.group_line(slot, format!("kind {k} first").as_bytes());
            stats.collapse(slot);
            stats.group_line(slot, format!("kind {k} second").as_bytes());
        }
        stats.collapse(0);
        stats.group_line(0, b"kind 0 third");
        stats.finish_groups();
        let mut lines: Vec<usize> = stats
            .collapsed()
            .iter()
            .map(|k| {
                std::fs::read_to_string(k.file.as_ref().unwrap())
                    .unwrap()
                    .lines()
                    .count()
            })
            .collect();
        lines.sort();
        assert_eq!(lines.len(), kinds);
        assert_eq!((lines[0], lines[kinds - 1]), (2, 3));
        assert!(stats.collapsed().iter().all(|k| !k.incomplete));
    }

    fn dark() -> aspect_theme::Palette {
        aspect_theme::Palette::aspect_dark(aspect_theme::Depth::Ansi256)
    }

    /// A report's `INFO:` line has the CLI's own label, its text plain; a
    /// detail line under it is muted.
    #[test]
    fn an_info_line_looks_as_the_clis_own_do() {
        let mut out = Vec::new();
        Style::themed(&dark()).line(&mut out, "INFO: Bazel's raw log is in x");
        Style::themed(&dark()).line(&mut out, "  3 more like: x");
        Style::themed(&dark()).rule(&mut out, "Error recap (3)");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[1;38;5;247mINFO\x1b[0m: Bazel's raw log is in x\n\x1b[38;5;247m  3 more like: x\x1b[0m\n\x1b[1;38;5;203m── Error recap (3) ──\x1b[0m\n"
        );
        assert_eq!(
            Style::themed(&dark()).error_label(b"ERROR: //a:b failed".to_vec()),
            b"\x1b[1;38;5;203mERROR\x1b[0m: //a:b failed".to_vec(),
        );
        assert_eq!(
            Style::default().error_label(b"ERROR: //a:b failed".to_vec()),
            b"ERROR: //a:b failed".to_vec(),
        );
        let mono = aspect_theme::Palette::new(
            aspect_theme::Theme::builtin("mono").unwrap(),
            Some(aspect_theme::Depth::Ansi256),
            false,
        );
        let mut out = Vec::new();
        Style::themed(&mono).line(&mut out, "INFO: x");
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b[1mINFO\x1b[0m: x\n");
    }

    /// Run `records` through `p`, coloured as bazel colours its messages
    /// ([`bazel`]), and what it reports.
    fn feed(p: &mut dyn LineProcessor, records: &[&str]) -> String {
        for r in records {
            let r = bazel(r);
            assert_eq!(p.process(r.as_bytes()).as_deref(), Some(r.as_bytes()));
        }
        String::from_utf8(p.report().unwrap_or_default()).unwrap()
    }

    /// The recap `records` make at once (`after` 0), escapes left out.
    fn recap_of(records: &[&str]) -> String {
        let mut p = RecapErrors::new(10, 0, Style::default());
        let out = feed(&mut p, records);
        String::from_utf8_lossy(&ansi::strip(out.as_bytes())).into_owned()
    }

    /// What Bazel and the CLI write after an error is never part of its
    /// body: the history's note of a kept log, in the CLI's own colours
    /// (`INFO` then a reset, then the colon), and Bazel's status rows after
    /// its progress lines (w44's `//tc:tc`, Bazel 9.3.3).
    #[test]
    fn notes_and_status_rows_stay_out_of_a_body() {
        let toolchain = [
            "\x1b[31m\x1b[1mERROR: \x1b[0mtc/BUILD.bazel:3:9: While resolving toolchains for target //tc:tc (2d89340): No matching toolchains found for types:",
            "  //tc:toolchain_type",
            "To debug, rerun with --toolchain_resolution_debug='//tc:toolchain_type'",
            "For more information on platforms or toolchains see https://bazel.build/concepts/platforms-intro.",
        ];
        let note = "\x1b[1;90mINFO\x1b[0m: //:bad Genrule stderr: /h/bad.stderr";
        let analyzed =
            "\x1b[32mINFO: \x1b[0mAnalyzed 8 targets (2 packages loaded, 53 targets configured).";
        let cases: [(&str, Vec<&str>); 2] = [
            (
                "in a pipe",
                [&toolchain[..], &[note, "\x1b[32mAnalyzing:\x1b[0m 8 targets (2 packages loaded, 24 targets configured)", "\x1b[32m[4 / 4]\x1b[0m no actions running", analyzed]].concat(),
            ),
            ("the note right after", [&toolchain[..], &[note, "\x1b[32m[4 / 4]\x1b[0m no actions running", analyzed]].concat()),
        ];
        for (name, records) in cases {
            let out = recap_of(&records);
            assert!(
                out.ends_with(
                    "    //tc:toolchain_type\n    To debug, rerun with --toolchain_resolution_debug='//tc:toolchain_type'\n    For more information on platforms or toolchains see https://bazel.build/concepts/platforms-intro.\n"
                ),
                "{name}: {out}"
            );
            assert!(!out.contains("bad.stderr"), "{name}: {out}");
        }
    }

    /// Bazel's closing lines restate the errors before them: none is
    /// repeated (`command succeeded, but …`, every form Bazel 7 to 9 writes).
    #[test]
    fn bazels_closing_lines_are_not_recapped() {
        assert_eq!(
            recap_of(&[
                "ERROR: command succeeded, but there were errors parsing the target pattern",
                "ERROR: command succeeded, but there were loading phase errors",
                "ERROR: command succeeded, but not all targets were analyzed",
                "ERROR: Build did NOT complete successfully",
            ]),
            ""
        );
    }

    /// A signal-killed action's header is cut after the signal, as after an
    /// exit code (`//f:killed`, Bazel 7.7.3 and 9.3.3): no command line.
    #[test]
    fn a_killed_actions_header_is_cut_after_the_signal() {
        for (line, want) in [
            (
                "ERROR: f/BUILD.bazel:3:8: Executing genrule //f:killed failed: (Killed): bash failed: error executing Genrule command (from genrule rule target //f:killed) /bin/bash -c 'kill -9 $$'",
                "ERROR: f/BUILD.bazel:3:8: Executing genrule //f:killed failed: (Killed)",
            ),
            (
                "ERROR: f/BUILD.bazel:4:8: Executing genrule //f:segv failed: (Segmentation fault): bash failed: error executing Genrule command (from target //f:segv) /bin/bash -c 'kill -11 $$'",
                "ERROR: f/BUILD.bazel:4:8: Executing genrule //f:segv failed: (Segmentation fault)",
            ),
            (
                "ERROR: sig/BUILD.bazel:1:8: Executing genrule //sig:s5 failed: (Trace/breakpoint trap): bash failed: error executing Genrule command (from genrule rule target //sig:s5) /bin/bash -c 'kill -5 $$'",
                "ERROR: sig/BUILD.bazel:1:8: Executing genrule //sig:s5 failed: (Trace/breakpoint trap)",
            ),
            (
                "ERROR: sig/BUILD.bazel:1:8: Executing genrule //sig:s10 failed: (User defined signal 1): bash failed: error executing Genrule command (from target //sig:s10) /bin/bash -c 'kill -10 $$'",
                "ERROR: sig/BUILD.bazel:1:8: Executing genrule //sig:s10 failed: (User defined signal 1)",
            ),
            (
                "ERROR: sig/BUILD.bazel:1:8: Executing genrule //sig:s24 failed: (Signal 24): bash failed: error executing Genrule command (from target //sig:s24) /bin/bash -c 'kill -24 $$'",
                "ERROR: sig/BUILD.bazel:1:8: Executing genrule //sig:s24 failed: (Signal 24)",
            ),
        ] {
            let out = recap_of(&[line, "about to die", "\x1b[32m[1 / 9]\x1b[0m running"]);
            assert!(
                out.contains(&format!("  {want}\n    about to die\n")),
                "{out}"
            );
        }
    }

    /// Themed, the rule and each quoted `ERROR` label are red.
    #[test]
    fn a_themed_recap_reads_as_errors() {
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::themed(&dark()));
        let mut records = vec![
            "ERROR: /ws/a/BUILD:1:8: Executing genrule //a:gen failed: (Exit 1)",
            "\x1b[32m[1 / 9]\x1b[0m running",
        ];
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        assert_eq!(
            feed(&mut p, &records),
            "\x1b[1;38;5;203m── Error recap (1) ──\x1b[0m\n  \
             \x1b[1;38;5;203mERROR\x1b[0m: /ws/a/BUILD:1:8: Executing genrule //a:gen failed: (Exit 1)\n"
        );
    }

    /// A nested macro's `fail()` (ws/nm, Bazel 9.3.3): the excerpt keeps the
    /// BUILD frame and its code line, then the `Error in` line, the frames
    /// between a gap.
    #[test]
    fn a_tracebacks_excerpt_is_its_build_frame_and_its_error() {
        let out = recap_of(&[
            "ERROR: Traceback (most recent call last):",
            "\tFile \"nm/BUILD.bazel\", line 2, column 6, in <toplevel>",
            "\t\touter(name = \"m\")",
            "\tFile \"nm/defs.bzl\", line 4, column 10, in outer",
            "\t\tinner(name = name + \"_i\")",
            "\tFile \"nm/defs.bzl\", line 2, column 9, in inner",
            "\t\tfail(\"inner refuses \" + name)",
            "Error in fail: inner refuses m_i",
            "\x1b[32m[1 / 9]\x1b[0m running",
        ]);
        assert!(
            out.ends_with(
                "    \tFile \"nm/BUILD.bazel\", line 2, column 6, in <toplevel>\n    \t\touter(name = \"m\")\n    … 4 more lines\n    Error in fail: inner refuses m_i\n"
            ),
            "{out}"
        );
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
            "── Error recap (first 2) ──\n  \
             ERROR: /ws/BUILD:1:1: Compiling a.cc failed\n  \
             ERROR: /ws/BUILD:2:1: Compiling b.cc failed\n"
        );
    }

    /// With the failure summary naming the raw log, the end of the console
    /// leaves it out, but still says what it hid.
    #[test]
    fn the_summary_names_the_raw_log_elsewhere() {
        let log = std::path::PathBuf::from("/h/console.log");
        let cut = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
        let mut s = Summary::new(
            std::sync::Arc::new(Stats::default()),
            RawLogState::Kept(log, cut),
            Style::default(),
            None,
        )
        .log_named_elsewhere(true);
        assert_eq!(s.report(), None, "nothing hidden, the log named elsewhere");
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
        records.extend(std::iter::repeat_n("more output", 5));
        records.push("\x1b[32m[1 / 9]\x1b[0m running");
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        assert_eq!(
            feed(&mut p, &records),
            "── Error recap (1) ──\n  \
             ERROR: /ws/BUILD:1:1: Executing genrule //dep:bad failed: (Exit 3)\n    \
             boom\n    app/x.cc:3: error: what went wrong\n    more output\n    \
             more output\n    … 2 more lines\n    more output\n"
        );
    }

    #[test]
    fn a_recapped_body_shows_its_first_error_however_far_down() {
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::default());
        let mut records =
            vec!["ERROR: /ws/BUILD:1:1: Compiling a.cc failed: (Exit 1): clang failed"];
        records.extend(std::iter::repeat_n("In file included from a.h:1:", 8));
        records.push("a.cc:3:4: error: no member named 'x'");
        records.extend(std::iter::repeat_n("note: here", 6));
        records.push("1 error generated.");
        records.push("\x1b[32m[1 / 9]\x1b[0m running");
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        assert_eq!(
            feed(&mut p, &records),
            "── Error recap (1) ──\n  \
             ERROR: /ws/BUILD:1:1: Compiling a.cc failed: (Exit 1)\n    \
             … 7 more lines\n    In file included from a.h:1:\n    \
             a.cc:3:4: error: no member named 'x'\n    note: here\n    \
             note: here\n    … 4 more lines\n    1 error generated.\n"
        );
    }

    #[test]
    fn a_recapped_body_longer_than_it_holds_counts_every_line() {
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::default());
        let lines: Vec<String> = (0..1000)
            .map(|i| {
                if i == 100 {
                    "error: deep".to_string()
                } else {
                    format!("line {i}")
                }
            })
            .collect();
        let mut records =
            vec!["ERROR: /ws/BUILD:1:1: Executing genrule //:g failed: (Exit 1): bash failed"];
        records.extend(lines.iter().map(String::as_str));
        records.push("\x1b[32m[1 / 9]\x1b[0m running");
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        let out = feed(&mut p, &records);
        let counted: usize = out
            .lines()
            .filter_map(|l| {
                l.trim()
                    .strip_prefix("… ")?
                    .split(' ')
                    .next()?
                    .parse::<usize>()
                    .ok()
            })
            .sum();
        let shown = out
            .lines()
            .filter(|l| l.starts_with("    ") && !l.contains("more line"))
            .count();
        assert_eq!(counted + shown, 1000, "{out}");
        assert!(out.contains("    error: deep\n"), "{out}");
        assert!(out.ends_with("    line 999\n"), "{out}");
    }

    #[test]
    fn a_recapped_body_ends_at_bazels_redraw_and_keeps_its_last_line() {
        let mut p = RecapErrors::new(5, RECAP_AFTER_RECORDS, Style::default());
        let mut records = vec![
            "ERROR: Traceback (most recent call last):",
            "\tFile \"/ws/a.bzl\", line 1",
            "\tFile \"/ws/b.bzl\", line 2",
            "\tFile \"/ws/c.bzl\", line 3",
            "\tFile \"/ws/d.bzl\", line 4",
            "\tFile \"/ws/e.bzl\", line 5",
            "Error in fail: the real reason",
            // Bazel's next message comes after its redraw, which ends the
            // body; a tool's own `WARNING:` line in it does not.
            "\x1b[1A\x1b[KWARNING: Remote Cache: UNAVAILABLE",
        ];
        records.extend(std::iter::repeat_n("noise", RECAP_AFTER_RECORDS));
        assert_eq!(
            feed(&mut p, &records),
            "── Error recap (1) ──\n  \
             ERROR: Traceback (most recent call last):\n    \
             … 5 more lines\n    Error in fail: the real reason\n"
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
            None,
        );
        assert_eq!(
            feed(&mut p, &["a"]),
            "INFO: Bazel's raw log is in /tmp/c.log\n",
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
            "INFO: Aspect hid 6 console lines; Bazel's raw log is in /tmp/c.log\n  \
             3 more like: WARNING: many\n  1 more like: WARNING: few\n"
        );
    }

    #[test]
    fn a_linking_recap_links_what_its_errors_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("BUILD.bazel"), "").unwrap();
        let mut p = RecapErrors::new(5, 0, Style::default()).linking(Some(
            super::super::links::Linker::new(vec![dir.path().to_path_buf()]).with_files(
                super::super::links::FileLinks::new(
                    super::super::links::LinkFormat::parse("file-test://{path}").unwrap(),
                ),
            ),
        ));
        let out = feed(&mut p, &["ERROR: BUILD.bazel:1:1: see https://x.test/why"]);
        assert!(
            out.starts_with("── Error recap (1) ──\n  ERROR: \x1b]8;;file-test://"),
            "{out:?}"
        );
        assert!(
            out.contains("/BUILD.bazel\x1b\\BUILD.bazel\x1b]8;;\x1b\\:1:1"),
            "{out:?}"
        );
        assert!(
            out.contains("\x1b]8;;https://x.test/why\x1b\\https://x.test/why\x1b]8;;\x1b\\\n"),
            "{out:?}"
        );
    }

    #[test]
    fn a_recapped_line_keeps_the_links_it_went_by_with() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["src/a.ts", "pkg/src/a.ts"] {
            let at = dir.path().join(f);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, "").unwrap();
        }
        let mut p = RecapErrors::new(5, 0, Style::default()).linking(Some(
            super::super::links::Linker::new(vec![dir.path().to_path_buf()]).with_files(
                super::super::links::FileLinks::new(
                    super::super::links::LinkFormat::parse("file-test://{path}").unwrap(),
                ),
            ),
        ));
        let pkg = dir.path().join("pkg/src/a.ts").display().to_string();
        let linked = format!("\x1b]8;;file://{pkg}\x1b\\src/a.ts\x1b]8;;\x1b\\:1: absolute");
        feed(
            &mut p,
            &[
                "ERROR: x failed: (Exit 1): tsc",
                "src/a.ts(1,1): error",
                &linked,
            ],
        );
        let out = feed(&mut p, &[]);
        let ws = dir.path().join("src/a.ts").display().to_string();
        let first = out.lines().find(|l| l.contains("error")).unwrap();
        let second = out.lines().find(|l| l.contains("absolute")).unwrap();
        assert!(
            first.contains(&format!("{ws}\x1b\\")),
            "the workspace's: {first:?}"
        );
        assert!(
            second.contains(&format!("{pkg}\x1b\\")),
            "the link it had: {second:?}"
        );
    }

    #[test]
    fn a_recapped_link_goes_back_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["src/a.ts", "pkg/src/a.ts", "pkg/BUILD.bazel"] {
            let at = dir.path().join(f);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, "").unwrap();
        }
        let mut linker = super::super::links::Linker::new(vec![dir.path().to_path_buf()])
            .with_files(super::super::links::FileLinks::new(
                super::super::links::LinkFormat::parse("file-test://{path}").unwrap(),
            ));
        linker.files.host = String::new();
        let mut p = RecapErrors::new(5, 0, Style::default()).linking(Some(linker));
        let link = |rel: &str, shown: &str| {
            format!(
                "\x1b]8;;file-test://{}/{rel}\x1b\\{shown}\x1b]8;;\x1b\\",
                dir.path().display()
            )
        };
        let header = format!(
            "\x1b[31mERROR: \x1b[0m{}:3:1: Compiling src/a.ts failed: (Exit 1): tsc {}",
            link("pkg/BUILD.bazel", "BUILD.bazel"),
            link("pkg/src/a.ts", "src/a.ts")
        );
        let stale = format!(
            "src/a.ts:9: stale; see {}:1:1",
            link("pkg/src/a.ts", "src/a.ts")
        );
        let srcs = format!(
            "in srcs:{}; also src/a.ts",
            link("pkg/src/a.ts", "src/a.ts")
        );
        let out = feed(&mut p, &[&header, &stale, &srcs]);
        let ws = link("src/a.ts", "src/a.ts");
        let pkg = link("pkg/src/a.ts", "src/a.ts");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[1],
            format!(
                "  ERROR: {}:3:1: Compiling {ws} failed: (Exit 1)",
                link("pkg/BUILD.bazel", "BUILD.bazel")
            ),
            "the command line's link, cut off, goes to no other path"
        );
        assert_eq!(
            lines[2],
            format!("    {ws}:9: stale; see {pkg}:1:1"),
            "each where it was"
        );
        assert_eq!(
            lines[3],
            format!("    in srcs:{pkg}; also {ws}"),
            "a link after `:` too"
        );
    }

    #[test]
    fn a_byte_not_utf8_before_a_link_moves_it_not() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["src/a.ts", "pkg/src/a.ts"] {
            let at = dir.path().join(f);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, "").unwrap();
        }
        let mut linker = super::super::links::Linker::new(vec![dir.path().to_path_buf()])
            .with_files(super::super::links::FileLinks::new(
                super::super::links::LinkFormat::parse("file-test://{path}").unwrap(),
            ));
        linker.files.host = String::new();
        let mut p = RecapErrors::new(5, 0, Style::default()).linking(Some(linker));
        let pkg = format!(
            "\x1b]8;;file-test://{}/pkg/src/a.ts\x1b\\src/a.ts\x1b]8;;\x1b\\",
            dir.path().display()
        );
        let mut record = b"\x1b[31mERROR: \x1b[0mcaf\xe9 ".to_vec();
        record.extend_from_slice(pkg.as_bytes());
        record.extend_from_slice(b":1:1: bad");
        assert_eq!(p.process(&record).as_deref(), Some(record.as_slice()));
        let out = String::from_utf8(p.report().unwrap()).unwrap();
        assert!(out.contains(&pkg), "{out:?}");
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
            "── Error recap (first 1) ──\n  ERROR: A\n"
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
    fn a_written_url_on_the_info_line_leaves_no_style_open() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("console.log");
        std::fs::write(&log, "").unwrap();
        let links = crate::console::links::FileLinks::new(
            crate::console::links::LinkFormat::parse("vscode").unwrap(),
        )
        .written(true)
        .styled(Some(crate::console::links::LinkStyle::of(&dark())));
        let mut p = Summary::new(
            Arc::new(Stats::default()),
            RawLogState::Kept(log, Default::default()),
            Style::themed(&dark()),
            Some(links),
        );
        let out = feed(&mut p, &[]);
        let line = out.lines().find(|l| l.contains("Bazel's raw log")).unwrap();
        assert!(line.ends_with(">\x1b[0m"), "{line:?}");
    }

    #[test]
    fn summary_names_no_log_when_none_was_configured() {
        let stats = Arc::new(Stats::default());
        stats.hid(1);
        let mut p = Summary::new(stats.clone(), RawLogState::Off, Style::default(), None);
        assert_eq!(feed(&mut p, &[]), "INFO: Aspect hid 1 console line\n");
        let mut p = Summary::new(
            Arc::new(Stats::default()),
            RawLogState::Off,
            Style::default(),
            None,
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
        let p = |state| Summary::new(Arc::new(Stats::default()), state, Style::default(), None);
        assert_eq!(
            feed(&mut p(RawLogState::Failed), &[]),
            "INFO: Bazel's raw log could not be saved\n"
        );
        let cut = Arc::new(std::sync::atomic::AtomicU8::new(Cut::Failed as u8));
        assert_eq!(
            feed(&mut p(RawLogState::Kept("/tmp/c.log".into(), cut)), &[]),
            "INFO: Bazel's raw log (incomplete: writing it failed) is in /tmp/c.log\n"
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
            None,
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
            None,
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
                "INFO: Aspect hid 3 console lines; Bazel's raw log is in {}\n  \
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
        let files = FileLinks::new(crate::console::links::LinkFormat::parse("vscode").unwrap());
        let link = file_link(&files, Path::new("/tmp/a b.log"), "");
        assert!(link.starts_with("\x1b]8;;vscode://file"), "{link:?}");
        assert!(
            link.contains("/tmp/a%20b.log:1:1\x1b\\/tmp/a b.log\x1b]8;;\x1b\\"),
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
