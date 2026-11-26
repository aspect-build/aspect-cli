//! What a console record is, as the stages that hide or count lines need to
//! know: a message's header or body, a line of bazel's progress UI, or blank.
//!
//! Patterns match a record's message text ([`message`]): escape sequences
//! stripped, and the `(12:00:00) ` prefix `--show_timestamps` adds, so
//! `^WARNING:` matches whatever the flags.
//!
//! # Where a message starts
//!
//! A line shaped like a header (`WARNING: …`) is not always bazel's: the
//! output of an action or a test, printed under `INFO: From …:` or an
//! `ERROR:`, can contain such lines too, coloured exactly as bazel colours
//! its own when it is a nested bazel's. Mistaking one for a message would let
//! a stage collapse or hide part of that output, so a [`Classifier`] calls a
//! line a [`Kind::Header`] only when
//!
//! * bazel's colour marks it: a coloured severity immediately followed by a
//!   reset, as bazel writes `ESC[35mWARNING: ESC[0m` ([`Line::coloured`]). A
//!   capture asks bazel for colour for this reason, unless a processor says
//!   not to (the user's `--color` only decides what is shown); a console
//!   without colour has no headers, and nothing in it is hidden;
//! * and it is not in an action's output. That output runs from its header
//!   (`INFO: From …`, or an `ERROR:` about an action rather than the build
//!   graph) until bazel itself ends it: under curses by moving the cursor up
//!   over its progress area to redraw it ([`Line::fresh`] right after a line
//!   of that area; a tool's own cursor movement elsewhere does not count), in
//!   a pipe with one of its coloured `[n / m]` progress lines. Nothing a tool
//!   prints ends it, so a pipe may leave a message after an action's output
//!   uncollapsed until bazel's next progress line;
//! * and bazel has not announced `--test_output=streamed`, under which test
//!   output arrives with no header of its own at all.
//!
//! Right after bazel ends an action's output, the next line may still be
//! the output's: a last line without a newline, which bazel flushes only
//! before its next event. So that line is a message only if it opens more
//! action output. Under `--noshow_progress` nothing ends an action's output,
//! and nothing after the first is hidden.
//!
//! Each errs on the side of a line staying visible. Two cases are beyond
//! what the bytes can tell: a nested `--color=yes` bazel's coloured `[n / m]`
//! line inside an action's output reads as bazel's own and ends that output,
//! and a tool's own coloured phase line (`Loading: …`) followed by
//! its own cursor-up reads as curses erasing its progress area.
//!
//! # The progress area
//!
//! Bazel's progress UI is a first line (`[3 / 9] …`, `Loading: …`,
//! `Analyzing: …`) and, under curses, indented lines under it, one per running
//! action, and the continuation of a line bazel wrapped with a trailing `\`.
//! In an action's output only bazel's coloured `[n / m]` line is taken for
//! one: a tool may well print `Loading: model weights`.
//! Curses redraws the area by moving the cursor up over the lines it
//! last wrote there, so a record that starts by moving the cursor up, a blank
//! line, or an unindented line ends it.
//!
//! A [`Classifier`] tracks all this, so it needs to see every record its
//! stage is given, in order.

use std::sync::LazyLock;

use regex::Regex;

use super::ansi;

/// The prefix `--show_timestamps` adds to every message and progress line:
/// `(12:00:00) `, or with fractions.
const TIMESTAMP_RE: &str = r"\(\d{2}:\d{2}:\d{2}(?:\.\d+)?\) ";

/// Bazel's progress count, `[3 / 9]`.
const COUNT_RE: &str = r"\[[\d,]+ / [\d,]+\]";

/// The phases Bazel's progress area names, before their colon.
const PHASES_RE: &str = "(?:Computing main repo mapping|Loading|Analyzing|Fetching|Building|Checking|Executing|Testing)";

/// The colour Bazel starts a word of its own with: one SGR sequence or more.
const COLOUR_RE: &str = r"(?:\x1b\[[0-9;]+m)+";

/// [`TIMESTAMP_RE`] at the start of a line.
static TIMESTAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^{TIMESTAMP_RE}")).unwrap());

/// A line shaped like a message's first: `INFO: `, `WARNING: `, `ERROR: `,
/// `FAIL: `…
static HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z]+: ").unwrap());

/// The first line of bazel's progress area, curses or not.
static PROGRESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^(?:{COUNT_RE} |{PHASES_RE}:)")).unwrap());

/// Whether `text` (escapes stripped) reads like the first line of Bazel's
/// progress area: `[3 / 9] …` or `Analyzing: …`, after any timestamp.
pub fn is_progress_text(text: &str) -> bool {
    PROGRESS.is_match(TIMESTAMP.replace(text, "").as_ref())
}

/// Whether `raw` is that line as Bazel colours it: told from a tool's line
/// of the same shape.
pub fn is_bazel_progress(raw: &[u8]) -> bool {
    BAZEL_COUNT.is_match(raw) || BAZEL_PHASE.is_match(raw)
}

/// Whether `raw` is Bazel's end-of-build status line as curses draws it
/// (`ESC[31mESC[1mFAILED:ESC[0m `, the colon inside the colour).
pub fn is_bazel_status(raw: &[u8]) -> bool {
    BAZEL_STATUS.is_match(raw)
}

/// Escape sequences, then an optional `--show_timestamps` prefix, at the
/// start of a raw record: what comes before bazel's colour.
static LEAD: LazyLock<String> =
    LazyLock::new(|| format!(r"^(?:\x1b\[[0-9;?]*[A-Za-z])*(?:{TIMESTAMP_RE})?"));

/// `body` as Bazel colours it at the start of a raw record: `LEAD`, its
/// colour, `body`, then a reset.
fn bazel_coloured(body: &str) -> regex::bytes::Regex {
    regex::bytes::Regex::new(&format!(r"{}{COLOUR_RE}{body}\x1b\[0m", *LEAD)).unwrap()
}

/// A severity as bazel colours it: `ESC[35mWARNING: ESC[0m`.
static BAZEL_LABEL: LazyLock<regex::bytes::Regex> = LazyLock::new(|| bazel_coloured("[A-Z]+: "));

/// Bazel's end-of-build status line, the colon inside the colour:
/// `ESC[32mINFO:ESC[0m `, `ESC[31mESC[1mFAILED:ESC[0m `.
static BAZEL_STATUS: LazyLock<regex::bytes::Regex> = LazyLock::new(|| bazel_coloured("[A-Z]+:"));

/// Bazel's own progress count, coloured: `ESC[32m[3 / 9]ESC[0m`.
static BAZEL_COUNT: LazyLock<regex::bytes::Regex> = LazyLock::new(|| bazel_coloured(COUNT_RE));

/// A phase of bazel's progress UI as it colours it: `ESC[32mLoading:ESC[0m`.
static BAZEL_PHASE: LazyLock<regex::bytes::Regex> =
    LazyLock::new(|| bazel_coloured(&format!("{PHASES_RE}:")));

/// Bazel's announcement that test output will be streamed as it comes.
const STREAMED: &str = "Streamed test output requested";

/// Bazel's `ERROR:`s that are about the build graph, not an action, and so
/// carry no tool output after them.
static GRAPH_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        // The phrase comes first or right after a `file:line:col` location:
        // an action's failure line can carry its own command's text, which
        // may say anything.
        r"^ERROR: (?:\S+:\d+(?::\d+)?: )?(?:no such package|no such target|errors encountered while analyzing",
        r"|Analysis of target .* failed|Analysis failed|Target parsing failed|Skipping '",
        r"|in (?:\S+ attribute of )?\S+ rule |in \S+ aspect on \S+ rule ",
        r"|Build did NOT complete|Build failed|command succeeded, but)"
    ))
    .unwrap()
});

/// Whether `text` (escapes stripped) is bare: a label and nothing after it,
/// as Bazel's own colon-inside status row always is (curses draws the
/// label, then erases it).
fn is_bare_status(text: &str) -> bool {
    text.split_once(':')
        .is_some_and(|(_, rest)| rest.trim().is_empty())
}

/// A header whose body is an action's or a test's output.
static ACTION_OUTPUT: LazyLock<Regex> =
    // A cached test's `INFO: From Testing //:t` has no trailing colon.
    LazyLock::new(|| Regex::new(r"^(?:INFO: From |ERROR: )").unwrap());

/// `text` (escapes already stripped) without a `--show_timestamps` prefix.
pub fn message(text: &str) -> &str {
    match TIMESTAMP.find(text) {
        Some(m) => &text[m.end()..],
        None => text,
    }
}

/// The message text of a raw record.
pub fn message_of(record: &[u8]) -> String {
    let text = ansi::text(record);
    message(&text).to_string()
}

/// What a record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Blank,
    /// The first line of the progress area.
    Progress,
    /// An indented line under it.
    ProgressMore,
    /// A message's first line: `SEVERITY: …`, where a message can start.
    Header,
    /// Anything else: a message's body, or output with no header.
    Other,
}

impl Kind {
    pub fn is_progress(self) -> bool {
        matches!(self, Kind::Progress | Kind::ProgressMore)
    }
}

/// One classified record.
#[derive(Debug, Clone)]
pub struct Line {
    pub kind: Kind,
    /// The message text, see [`message`].
    pub text: String,
    /// Whether the record starts by moving the cursor up: under a PTY, curses
    /// erasing its progress area to write something new, or a tool's own
    /// cursor movement, which the classifier tells apart.
    pub fresh: bool,
    /// Whether the record starts the way bazel colours a message's severity,
    /// or its progress count; see the module docs.
    pub coloured: bool,
    /// Whether the text starts with whitespace.
    pub indented: bool,
    /// Whether it is the header of an action's or a test's output
    /// (`INFO: From …`, or an `ERROR:` about an action rather than the build
    /// graph), whose indented lines are that output.
    pub opens_output: bool,
    /// Whether curses may have drawn it in its progress area, to erase and
    /// redraw: every progress line, and also a coloured count or phase line
    /// inside action output, the end-of-build status line, the BES upload
    /// line, and the indented lines under them. Under a PTY no stage changes
    /// such a row's height; see [`super::filters`]. Errs toward true.
    pub drawn: bool,
    /// Whether the record ended in a bare `\r` under a PTY, so what comes
    /// next writes over it (a tool's spinner). Set by the splitter, which
    /// sees the boundary.
    pub returns: bool,
    /// Whether it came while an action's or a test's output was open, or
    /// after streamed test output was announced: a tool's, whatever its
    /// shape.
    pub in_output: bool,
    /// Whether streamed test output was announced before it: from there on
    /// test output has no header, and only what follows Bazel's closing
    /// `Build completed…` message is surely Bazel's.
    pub streamed: bool,
    /// Whether it is the first record after an action's output ended,
    /// starting the way bazel colours a message's severity: bazel's next
    /// message or the output's held last line, which the bytes cannot tell
    /// apart (see the module docs). It is [`Kind::Other`] unless it opens
    /// more output.
    pub tail: bool,
}

/// What a record is, for a stage: what the chain said it is
/// ([`super::forward::LineProcessor::see`]), or, for a stage run on its own,
/// what a classifier of the stage's own makes of it.
#[derive(Default)]
pub struct Given {
    line: Option<Line>,
    own: Classifier,
}

impl Given {
    /// What the chain said the next record is.
    pub fn see(&mut self, line: &Line) {
        self.line = Some(line.clone());
    }

    /// What `record` is, kept until [`Given::take`].
    pub fn peek(&mut self, record: &[u8]) -> &Line {
        let own = &mut self.own;
        self.line.get_or_insert_with(|| own.classify(record))
    }

    /// What `record` is.
    pub fn take(&mut self, record: &[u8]) -> Line {
        self.peek(record);
        self.line.take().expect("peek set it")
    }
}

/// Classifies records in stream order.
#[derive(Default)]
pub struct Classifier {
    in_progress: bool,
    /// The last row was [`Line::drawn`], so an indented one under it is too.
    in_drawn: bool,
    /// The last row was the end-of-build status line; the one after it is
    /// curses' too (the BES upload wait, in whatever form fits).
    after_status: bool,
    /// The last progress line ended with bazel's wrapping `\`.
    wrapped: bool,
    in_action_output: bool,
    /// Bazel announced streamed test output: no line is a header from here.
    streamed: bool,
    /// Whether the last unindented, non-blank record was a line of bazel's
    /// progress area (its first line, or a coloured count or phase).
    after_progress: bool,
    /// An action's output just ended: the next line may be its last, which
    /// bazel holds until it ends with a newline and flushes only after
    /// drawing its progress, before its next event.
    maybe_tail: bool,
}

impl Classifier {
    /// The same rules serve a PTY capture and a pipe: what tells them apart,
    /// colour and cursor movement, is in the records.
    pub fn classify(&mut self, record: &[u8]) -> Line {
        let fresh = moves_cursor_up(ansi::leading(record));
        // Curses erases its progress area before writing anything new, so a
        // cursor-up right after it ends an action's output; one anywhere
        // else is the action's own (a tool's progress bar, whose `\r` before
        // its `ESC[1A` reads as a blank record, as curses' own does).
        let erases_progress = self.after_progress;
        if fresh {
            self.in_progress = false;
            self.in_drawn = false;
            if erases_progress && self.in_action_output {
                self.in_action_output = false;
                self.maybe_tail = true;
            }
        }
        let text = message_of(record);
        let inside = self.in_action_output || self.streamed;
        let mut was_tail = false;
        let label = BAZEL_LABEL.is_match(record);
        let count = BAZEL_COUNT.is_match(record);
        let phase = !self.in_action_output && BAZEL_PHASE.is_match(record);
        let indented = text.starts_with([' ', '\t']);
        let wrapped = std::mem::take(&mut self.wrapped);
        let progress = if self.in_action_output {
            count
        } else {
            PROGRESS.is_match(&text)
        };
        let mut opens_output = false;
        let kind = if text.trim().is_empty() {
            self.in_progress = false;
            Kind::Blank
        } else if progress {
            self.in_progress = true;
            self.maybe_tail |= self.in_action_output;
            self.in_action_output = false;
            Kind::Progress
        } else if self.in_progress && (indented || wrapped) {
            Kind::ProgressMore
        } else {
            self.in_progress = false;
            let tail = std::mem::take(&mut self.maybe_tail);
            was_tail = tail;
            let opens = ACTION_OUTPUT.is_match(&text) && !GRAPH_ERROR.is_match(&text);
            // A possible tail is taken for a message only if it opens more
            // output, as bazel's next `INFO: From …` does.
            if label
                && !self.in_action_output
                && !self.streamed
                && (opens || !tail)
                && HEADER.is_match(&text)
            {
                opens_output = opens;
                self.in_action_output = opens_output;
                self.streamed |= text.contains(STREAMED);
                Kind::Header
            } else {
                Kind::Other
            }
        };
        // A tool's own line coloured as Bazel colours its status line (the
        // colon inside, the space after) is the tool's: never drawn inside
        // an action's output. Streamed test output never ends, so there
        // only a bare row, as Bazel's own always is, is its status line.
        let status = !self.in_action_output
            && BAZEL_STATUS.is_match(record)
            && (!self.streamed || is_bare_status(&text));
        let after_status = std::mem::replace(&mut self.after_status, status);
        let drawn = kind.is_progress()
            || count
            || BAZEL_PHASE.is_match(record)
            || status
            || (!self.in_action_output
                && (text.starts_with("Waiting for build events upload")
                    || text.starts_with("Waiting for: ")))
            || (after_status && kind != Kind::Blank)
            || (self.in_drawn && kind != Kind::Blank && (indented || wrapped));
        let announces = text.contains(STREAMED);
        self.in_drawn = drawn;
        // Bazel wraps a row of its progress area with a `\`, whatever kind.
        self.wrapped = drawn && text.trim_end().ends_with('\\');
        if kind == Kind::Progress || ((count || BAZEL_PHASE.is_match(record)) && !indented) {
            self.after_progress = true;
        } else if kind != Kind::Blank && !indented && !drawn {
            self.after_progress = false;
        }
        Line {
            kind,
            text,
            fresh,
            // Streamed test output has no header to be inside of, so from
            // the announcement on nothing is taken for bazel's own.
            coloured: !self.streamed && (label || count || phase),
            indented,
            opens_output,
            drawn,
            returns: false,
            in_output: inside,
            streamed: self.streamed && !announces,
            tail: was_tail && label && !self.streamed,
        }
    }
}

/// Whether `escapes` include a cursor-up (`ESC [ n A`).
fn moves_cursor_up(escapes: &[u8]) -> bool {
    escapes.windows(2).enumerate().any(|(i, w)| {
        w == b"\x1b[" && {
            let rest = &escapes[i + 2..];
            let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
            rest.get(digits) == Some(&b'A')
        }
    })
}

/// Tracks whether the records after a hidden message are its body, hidden
/// with it: the indented [`Kind::Other`] lines right under it, up to the
/// first record that is not one or where curses starts something new. An
/// unindented line is taken only in the one shape bazel's own call stacks
/// have (`Repository rule http_archive defined at:`), since nothing else
/// marks it as the message's rather than, say, the next action's output.
#[derive(Default)]
pub struct Body {
    active: bool,
}

impl Body {
    /// Start a body: the message before it was hidden.
    pub fn open(&mut self) {
        self.active = true;
    }

    /// Whether `line` is the open body's, and so hidden too. The first line
    /// that is not ends the body.
    pub fn swallows(&mut self, line: &Line) -> bool {
        static STACK: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^Repository rule \S+ defined at:$").unwrap());
        self.active &= line.kind == Kind::Other
            && (line.indented || STACK.is_match(line.text.trim_end()))
            && !line.fresh;
        self.active
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A stage run on its own classifies each record once, however often it
    /// looks before it takes it; one the chain classified is not classified.
    #[test]
    fn given_classifies_a_record_once() {
        let mut given = Given::default();
        let first = given.peek(b"INFO: From x:").kind;
        assert_eq!(
            given.peek(b"anything").kind,
            first,
            "still the first record's"
        );
        let first = given.take(b"ignored, already classified");
        assert!(first.text.contains("INFO: From x:"), "{}", first.text);
        let said = Classifier::default().classify(b"plain");
        given.see(&said);
        assert_eq!(given.take(b"INFO: From y:").kind, said.kind);
    }

    /// `line` as bazel writes it with colour: a leading severity, after any
    /// timestamp, wrapped in its colour. A line starting `~` is a tool's,
    /// passed as it is without the `~`; one with escapes already is passed
    /// as it is.
    pub(crate) fn bazel(line: &str) -> String {
        if let Some(tool) = line.strip_prefix('~') {
            return tool.to_string();
        }
        if line.contains('\x1b') {
            return line.to_string();
        }
        let (stamp, rest) = match TIMESTAMP.find(line) {
            Some(m) => line.split_at(m.end()),
            None => ("", line),
        };
        match HEADER.find(rest) {
            Some(m) => format!("{stamp}{OPEN}{}{CLOSE}{}", m.as_str(), &rest[m.end()..]),
            None => line.to_string(),
        }
    }

    /// The colour [`bazel`] adds, in bazel's form but with a code no test
    /// spells out, so [`unbazel`] can take exactly it back out.
    const OPEN: &str = "\x1b[1;35m";
    const CLOSE: &str = "\x1b[0m";

    /// `out` without the colour [`bazel`] added.
    pub(crate) fn unbazel(out: &[u8]) -> String {
        static ADDED: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\x1b\[1;35m([A-Z]+: )?\x1b\[0m").unwrap());
        ADDED
            .replace_all(&String::from_utf8_lossy(out), "$1")
            .into_owned()
    }

    fn kinds(records: &[&str]) -> Vec<Kind> {
        let mut c = Classifier::default();
        records
            .iter()
            .map(|r| c.classify(bazel(r).as_bytes()).kind)
            .collect()
    }

    /// A tool's line coloured as Bazel colours its status line, inside an
    /// action's output, is the tool's; Bazel's own status rows, after
    /// streamed test output too, are still drawn.
    #[test]
    fn a_tools_line_coloured_as_bazels_status_is_not_drawn() {
        for label in ["INFO", "WARNING", "ERROR", "DEBUG", "FAILED"] {
            let mut c = Classifier::default();
            c.classify(bazel("INFO: From Executing genrule //:g:").as_bytes());
            let shaped = format!("\x1b[32m{label}:\x1b[0m two");
            for tool in ["one", shaped.as_str(), "three", "four"] {
                assert!(!c.classify(tool.as_bytes()).drawn, "{label}: {tool:?}");
            }
        }
        let mut c = Classifier::default();
        assert!(
            c.classify(b"\x1b[32mINFO:\x1b[0m Build completed successfully")
                .drawn
        );
    }

    #[test]
    fn bazels_status_rows_after_streamed_test_output_are_drawn() {
        let mut c = Classifier::default();
        c.classify(bazel("INFO: Streamed test output requested.").as_bytes());
        assert!(
            c.classify(b"\x1b[1;32mINFO:\x1b[0m ").drawn,
            "a bare status row"
        );
        assert!(
            c.classify(b"\x1b[31m\x1b[1mFAILED:\x1b[0m ").drawn,
            "a bare failure row"
        );
        for line in [
            &b"\x1b[32mINFO:\x1b[0m two"[..],
            b"\x1b[32mINFO:\x1b[0m Build completed successfully, 12 total actions",
            b"\x1b[32mINFO:\x1b[0m Elapsed time: 1.2s, Critical Path: 1.0s",
            b"\x1b[31m\x1b[1mFAILED:\x1b[0m Build did NOT complete successfully",
        ] {
            let mut c = Classifier::default();
            c.classify(bazel("INFO: Streamed test output requested.").as_bytes());
            assert!(!c.classify(line).drawn, "a test's own line: {line:?}");
        }
    }

    /// A tool's own "Waiting for" line is its output, not bazel's BES wait.
    #[test]
    fn a_tools_waiting_line_is_not_drawn() {
        let mut c = Classifier::default();
        c.classify(bazel("INFO: From Executing genrule //:g:").as_bytes());
        for tool in [
            "Waiting for: postgres on :5432 \\",
            "connected",
            "  detail line",
        ] {
            assert!(!c.classify(tool.as_bytes()).drawn, "{tool}");
        }
        let mut c = Classifier::default();
        assert!(c.classify(b"Waiting for: Build Eve... 1s").drawn);
    }

    #[test]
    fn a_timestamp_prefix_is_not_part_of_the_message() {
        assert_eq!(message("(12:00:01) WARNING: x"), "WARNING: x");
        assert_eq!(message("(12:00:01.123) INFO: x"), "INFO: x");
        assert_eq!(message("WARNING: (12:00:01) x"), "WARNING: (12:00:01) x");
        assert_eq!(
            kinds(&["(12:00:00) DEBUG: x", "(12:00:01) [4 / 5] Compiling a.cc"]),
            [Kind::Header, Kind::Progress]
        );
    }

    #[test]
    fn without_colour_nothing_is_a_message() {
        assert_eq!(
            kinds(&[
                "~WARNING: plain",
                "~INFO: plain",
                "~\x1b[0mERROR: a reset is not a colour"
            ]),
            [Kind::Other, Kind::Other, Kind::Other]
        );
    }

    /// Bazel 9's curses UI, as a PTY capture records it: each clear of a
    /// progress line is `\r ESC[1A ESC[K`, the last one starting the message.
    #[test]
    fn the_progress_area_runs_until_curses_moves_up_over_it() {
        assert_eq!(
            kinds(&[
                "\x1b[32m[1 / 8]\x1b[0m 6 actions running",
                "    Executing genrule //:g1; 0s darwin-sandbox",
                "    @@local_config_xcode//:host_xcodes; \\",
                "local_config//:x",
                "\x1b[1A\x1b[K",
                "\x1b[1A\x1b[K\x1b[32mINFO: \x1b[0mFound 1 target...",
                "\x1b[1A\x1b[K  indented after a redraw",
                "\tFile \"BUILD\", line 7",
                "",
                "Target //:top up-to-date:",
            ]),
            [
                Kind::Progress,
                Kind::ProgressMore,
                Kind::ProgressMore,
                Kind::ProgressMore,
                Kind::Blank,
                Kind::Header,
                Kind::Other,
                Kind::Other,
                Kind::Blank,
                Kind::Other,
            ]
        );
    }

    #[test]
    fn an_unindented_line_ends_the_progress_area() {
        assert_eq!(
            kinds(&["Loading: 1 packages", "plain", "  indented"]),
            [Kind::Progress, Kind::Other, Kind::Other]
        );
    }

    /// A tool's lines in an action's output: header-shaped, progress-shaped,
    /// coloured, after blank lines. None is bazel's while curses has not
    /// started something new.
    #[test]
    fn action_output_is_never_a_message() {
        assert_eq!(
            kinds(&[
                "\x1b[1A\x1b[K\x1b[32mINFO: \x1b[0mFrom Executing genrule //:g1:",
                "~WARNING: tool warning",
                "~INFO: tool says hello",
                "~Loading: model weights",
                "~  indented",
                "",
                "~\x1b[35mWARNING: \x1b[0mtool colour",
                "~\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mtool's own cursor movement",
                "\x1b[32m[2 / 3]\x1b[0m 1 action running",
                "\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mmaybe the output's flushed tail",
                "\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mbazel's own",
            ]),
            [
                Kind::Header,
                Kind::Other,
                Kind::Other,
                Kind::Other,
                Kind::Other,
                Kind::Blank,
                Kind::Other,
                Kind::Other,
                Kind::Progress,
                Kind::Other,
                Kind::Header,
            ]
        );
    }

    /// In a pipe there is no cursor movement: an action's output, a nested
    /// bazel's coloured lines included, runs until bazel's own coloured
    /// progress count.
    #[test]
    fn in_a_pipe_action_output_runs_until_bazels_progress() {
        assert_eq!(
            kinds(&[
                "WARNING: bazel's own",
                "INFO: From Testing //:t:",
                "~\x1b[35mWARNING: \x1b[0mfrom a nested bazel",
                "~[1 / 2] a tool's count",
                "~\x1b[32mLoading:\x1b[0m a tool's",
                "\x1b[32m[3 / 9]\x1b[0m 2 actions running",
                "WARNING: maybe the output's flushed tail",
                "WARNING: bazel's own again",
            ]),
            [
                Kind::Header,
                Kind::Header,
                Kind::Other,
                Kind::Other,
                Kind::Other,
                Kind::Progress,
                Kind::Other,
                Kind::Header,
            ]
        );
    }

    /// Streamed test output has no header to be under; once bazel says it
    /// streams, nothing more is taken for a message.
    #[test]
    fn streamed_test_output_ends_all_headers() {
        assert_eq!(
            kinds(&[
                "WARNING: Streamed test output requested. All tests will be run locally, without sharding, one at a time",
                "\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mfrom the test",
            ]),
            [Kind::Header, Kind::Other]
        );
    }

    /// A tool's own cursor movement (a progress bar) is part of its output;
    /// only curses erasing the progress area ends it.
    #[test]
    fn a_tools_cursor_movement_does_not_end_its_output() {
        assert_eq!(
            kinds(&[
                "INFO: From Executing genrule //:g:",
                "~\x1b[1A\x1b[Ktool progress 50%",
                "",
                "~\x1b[1A\x1b[Ktool progress 60%, after the `\\r` a redraw starts with",
                "~\x1b[35mWARNING: \x1b[0mtool says thing",
                "\x1b[32m[2 / 3]\x1b[0m 1 action running",
                "\x1b[1A\x1b[K",
                "\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mmaybe the output's flushed tail",
                "\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mbazel's own",
            ]),
            [
                Kind::Header,
                Kind::Other,
                Kind::Blank,
                Kind::Other,
                Kind::Other,
                Kind::Progress,
                Kind::Blank,
                Kind::Other,
                Kind::Header,
            ]
        );
    }

    #[test]
    fn bazels_phase_lines_are_coloured_progress() {
        let mut c = Classifier::default();
        let line = c.classify(b"\x1b[32mLoading:\x1b[0m 3 packages loaded");
        assert_eq!((line.kind, line.coloured), (Kind::Progress, true));
        assert!(!c.classify(b"Loading: plain").coloured);
    }

    /// A tool's last line without a newline arrives after bazel's progress:
    /// it is no message of bazel's, but bazel's next action output still is.
    #[test]
    fn an_actions_flushed_last_line_is_not_a_message() {
        assert_eq!(
            kinds(&[
                "INFO: From Executing genrule //:a:",
                "\x1b[32m[3 / 4]\x1b[0m Executing genrule //:b",
                "~\x1b[35mDEBUG: \x1b[0mRule 'y' indicated that a canonical (tool tail)",
                "INFO: From Executing genrule //:b:",
                "~\x1b[35mWARNING: \x1b[0mtool tail",
                "\x1b[32m[4 / 4]\x1b[0m done",
                "WARNING: bazel's own, once the tail has passed",
                "WARNING: and again",
            ]),
            [
                Kind::Header,
                Kind::Progress,
                Kind::Other,
                Kind::Header,
                Kind::Other,
                Kind::Progress,
                Kind::Other,
                Kind::Header,
            ]
        );
    }

    #[test]
    fn graph_errors_open_no_output() {
        let mut c = Classifier::default();
        for e in [
            "ERROR: /ws/BUILD:2:9: in cmd attribute of genrule rule //:bad0: label '//:x' is not declared",
            "ERROR: /ws/BUILD:2:9: Analysis of target '//:bad0' (config: 2d89340) failed",
            "ERROR: no such package 'nope': BUILD file not found",
            "ERROR: Build did NOT complete successfully",
            "ERROR: /ws/BUILD:3:11: in //tools:lint.bzl%lint_aspect aspect on cc_library rule //:x: bad",
        ] {
            assert!(!c.classify(bazel(e).as_bytes()).opens_output, "{e}");
        }
        // An action's failure line quoting a command that says "Build failed".
        let quoting = Classifier::default().classify(
            bazel("ERROR: /ws/BUILD:1:8: Executing genrule //:lint failed: (Exit 1): bash -c 'echo lint: Build failed'")
                .as_bytes(),
        );
        assert!(quoting.opens_output);
        let action = Classifier::default()
            .classify(bazel("ERROR: /ws/BUILD:1:1: Compiling a.cc failed: (Exit 1)").as_bytes());
        assert!(action.opens_output);
    }

    #[test]
    fn the_colour_after_a_timestamp_counts() {
        let mut c = Classifier::default();
        let line = c.classify(b"(11:01:59) \x1b[35mWARNING: \x1b[0mx");
        assert!(line.coloured);
        assert_eq!(line.kind, Kind::Header);
        assert_eq!(line.text, "WARNING: x");
    }

    #[test]
    fn a_body_takes_only_the_indented_lines_right_under_it() {
        let mut c = Classifier::default();
        let mut b = Body::default();
        let mut see = |r: &str| c.classify(bazel(r).as_bytes());
        see("WARNING: hidden");
        b.open();
        assert!(b.swallows(&see("  indented detail")));
        assert!(!b.swallows(&see("Target //:x up-to-date:")));
        assert!(!b.swallows(&see("  bazel-bin/x")), "a body ends for good");
        b.open();
        assert!(!b.swallows(&see("\x1b[1A\x1b[K  after a redraw")));
        b.open();
        assert!(!b.swallows(&see("Analysis failed")));
    }
}
