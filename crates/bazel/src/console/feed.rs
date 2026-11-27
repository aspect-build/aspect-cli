//! What a console shows, as data, for a caller drawing its own screen: the
//! lines the stages pass, and Bazel's progress area as it last drew it.
//!
//! Writers each write as their own [`Source`], so a line one leaves unended
//! is not joined to the other's: the console's forwarder as stderr, and
//! `std.term`'s capture of the process's stdout and stderr as both. The
//! caller drains it whenever it redraws. Lines are kept as text a screen can
//! draw in one row (`plain_line`: escape sequences stripped once the line
//! has ended, so one split across writes is still whole; what a carriage
//! return wrote over dropped, as a terminal would; other control characters,
//! a backspace among them, removed), up to [`MAX_LINES`] between drains;
//! older ones are counted in [`Drained::dropped`] instead. A line's OSC 8
//! links are kept beside it as spans of its text ([`Drained::links`]), for
//! the screen to draw and open: the console's Links stage's, `print()`'s,
//! and any a tool wrote itself, so a screen opens only what it trusts. Each
//! line comes with its kind (`error`, `warning` or none) and whether it is
//! one of Bazel's own ([`Kinds`]), for a screen filtering or folding the
//! console. The progress area is replaced on every redraw, so only the
//! latest is kept.

use std::collections::VecDeque;
use std::sync::Mutex;

use super::ansi;
use super::links;

/// A link in a drawable line: the characters `start..end` of its text, and
/// the URL they open.
pub type Link = (u32, u32, String);

/// Lines kept between drains; past it the oldest go.
pub const MAX_LINES: usize = 10_000;

/// The most of a line kept as written, escapes and all; the rest of it is
/// cut.
pub const MAX_LINE_BYTES: usize = 64 << 10;

#[derive(Debug, Default)]
pub struct Feed {
    state: Mutex<State>,
}

/// Which writer a line comes from: each keeps its own unended line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Stderr,
    Stdout,
}

#[derive(Debug, Default)]
struct State {
    lines: VecDeque<Kept>,
    /// The kind a line of Bazel's own last left for the lines after it
    /// ([`Kinds`]).
    kinds: Kinds,
    /// The bytes of the line each source is writing, not yet ended.
    partial: [Vec<u8>; 2],
    /// A carriage return each source wrote last, not yet known to end its
    /// line (`\r\n`) or to start a redraw over it.
    cr: [bool; 2],
    dropped: u64,
    progress: Vec<String>,
}

/// One line kept: as [`line_of`] makes it, with its kind and whether it is
/// one of Bazel's own ([`Kinds`]).
#[derive(Debug)]
struct Kept {
    text: String,
    links: Vec<Link>,
    coloured: String,
    kind: &'static str,
    bazel: bool,
}

/// What [`Feed::drain`] hands over.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Drained {
    /// Lines ended since the last drain, oldest first.
    pub lines: Vec<String>,
    /// Each of `lines`' links, at the same index.
    pub links: Vec<Vec<Link>>,
    /// Each of `lines` with its colour kept (`coloured_line`), at the same
    /// index: the same text, for a screen that shows the colour.
    pub colours: Vec<String>,
    /// Each of `lines`' kind, at the same index: `error`, `warning` or ``
    /// ([`Kinds::kind`]), the lines under an `ERROR:` taking its kind.
    pub kinds: Vec<&'static str>,
    /// Whether each of `lines` is one of Bazel's own, at the same index: it
    /// starts as Bazel's lines do and is not coloured as a tool colours
    /// such a line ([`Kinds::bazel`]).
    pub bazel: Vec<bool>,
    /// Lines that were dropped for [`MAX_LINES`] since the last drain.
    pub dropped: u64,
    /// The rows of Bazel's progress area as it was before its latest redraw,
    /// so at most one redraw behind: its status line (`[n / m] …`,
    /// `Analyzing: …`) first, then one per running action. Empty before it
    /// first redraws one, and after the build ends.
    pub progress: Vec<String>,
}

impl Feed {
    /// Take in bytes `source` would have written: whole lines are kept, a
    /// trailing partial one waits for its end.
    pub fn write(&self, source: Source, bytes: &[u8]) {
        let mut s = self.state.lock().unwrap();
        let at = source as usize;
        for &b in bytes {
            // A redraw writes over the line so far: only its newest state
            // is kept, however long the line runs before it ends.
            // A run of carriage returns is one, as on a terminal.
            if std::mem::take(&mut s.cr[at]) && b != b'\n' && b != b'\r' {
                s.partial[at].clear();
            }
            match b {
                b'\n' => {
                    let line = std::mem::take(&mut s.partial[at]);
                    s.push(line_of(&line));
                }
                b'\r' => s.cr[at] = true,
                _ if s.partial[at].len() < MAX_LINE_BYTES => s.partial[at].push(b),
                _ => {}
            }
        }
    }

    /// Bazel's progress area as just drawn: `rows`, rendered.
    pub fn set_progress(&self, rows: Vec<String>) {
        self.state.lock().unwrap().progress = rows;
    }

    /// `source` ended: its partial line is kept, and with stderr, the
    /// console's, there is no progress area any more.
    pub fn finish(&self, source: Source) {
        let mut s = self.state.lock().unwrap();
        let line = std::mem::take(&mut s.partial[source as usize]);
        s.cr[source as usize] = false;
        if !line.is_empty() {
            s.push(line_of(&line));
        }
        if source == Source::Stderr {
            s.progress.clear();
        }
    }

    /// The lines since the last drain, and the progress area now.
    pub fn drain(&self) -> Drained {
        let mut s = self.state.lock().unwrap();
        let mut lines = Vec::new();
        let mut links = Vec::new();
        let mut colours = Vec::new();
        let mut kinds = Vec::new();
        let mut bazel = Vec::new();
        for kept in s.lines.drain(..) {
            lines.push(kept.text);
            links.push(kept.links);
            colours.push(kept.coloured);
            kinds.push(kept.kind);
            bazel.push(kept.bazel);
        }
        Drained {
            lines,
            links,
            colours,
            kinds,
            bazel,
            dropped: std::mem::take(&mut s.dropped),
            progress: s.progress.clone(),
        }
    }
}

/// A line as written (`raw`, its newline gone) made drawable in one row: a
/// trailing carriage return dropped, what one before wrote over gone
/// (`10%\r20%` is `20%`), then escape sequences stripped, then every control
/// character but a tab dropped (C0, DEL and C1, a backspace removed rather
/// than applied), as a screen drawing it would be thrown out by them.
fn plain_line(raw: &[u8]) -> String {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let shown = raw
        .iter()
        .rposition(|&b| b == b'\r')
        .map_or(raw, |i| &raw[i + 1..]);
    String::from_utf8_lossy(&ansi::strip(shown))
        .chars()
        .filter(|&c| c == '\t' || !(c < ' ' || ('\u{7f}'..='\u{9f}').contains(&c)))
        .collect()
}

/// A line as [`linked_line`] makes it, and as `coloured_line` does.
fn line_of(raw: &[u8]) -> (String, Vec<Link>, String) {
    let (text, links) = linked_line(raw);
    (text, links, coloured_line(raw))
}

/// `plain_line` with its colour (SGR) kept: the same visible text, every
/// other escape and control character but a tab dropped, the colour set
/// before a redraw (`\r`) carried over it.
fn coloured_line(raw: &[u8]) -> String {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let (before, shown) = match raw.iter().rposition(|&b| b == b'\r') {
        Some(i) => (&raw[..i], &raw[i + 1..]),
        None => (&raw[..0], raw),
    };
    let mut out = Vec::new();
    for (escape, seg) in ansi::segments(before) {
        if escape && ansi::is_sgr(seg) {
            out.extend_from_slice(seg);
        }
    }
    for (escape, seg) in ansi::segments(shown) {
        if !escape || ansi::is_sgr(seg) {
            out.extend_from_slice(seg);
        }
    }
    String::from_utf8_lossy(&out)
        .chars()
        .filter(|&c| c == '\t' || c == '\x1b' || !(c < ' ' || ('\u{7f}'..='\u{9f}').contains(&c)))
        .collect()
}

/// `plain_line`, with the line's OSC 8 links as spans of that text: none
/// for a line that is not UTF-8, whose text is a lossy copy.
fn linked_line(raw: &[u8]) -> (String, Vec<Link>) {
    let text = plain_line(raw);
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let shown = raw
        .iter()
        .rposition(|&b| b == b'\r')
        .map_or(raw, |i| &raw[i + 1..]);
    let found = links::links_in(shown);
    if found.is_empty() {
        return (text, Vec::new());
    }
    let stripped = ansi::strip(shown);
    let Ok(stripped) = std::str::from_utf8(&stripped) else {
        return (text, Vec::new());
    };
    // Each byte offset among the stripped text, as a character of `text`,
    // which drops control characters but a tab.
    let mut kept = 0u32;
    let mut at = vec![0u32; stripped.len() + 1];
    for (i, c) in stripped.char_indices() {
        at[i] = kept;
        if c == '\t' || !(c < ' ' || ('\u{7f}'..='\u{9f}').contains(&c)) {
            kept += 1;
        }
    }
    at[stripped.len()] = kept;
    let spans = found
        .into_iter()
        .filter(|k| k.at + k.shown.len() <= stripped.len())
        // A URL a tool wrote may carry what a screen drawing it would run
        // (a C1 control); the console's own are printable ASCII.
        .filter(|k| !k.url.is_empty() && k.url.bytes().all(|b| (0x21..=0x7e).contains(&b)))
        .map(|k| (at[k.at], at[k.at + k.shown.len()], k.url))
        .filter(|(start, end, _)| start < end)
        .collect();
    (text, spans)
}

impl State {
    fn push(&mut self, (text, links, coloured): (String, Vec<Link>, String)) {
        let kind = self.kinds.kind(&text);
        let bazel = Kinds::bazel(&text, &coloured);
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(Kept {
            text,
            links,
            coloured,
            kind,
            bazel,
        });
    }
}

/// What starts a line of Bazel's own: a line that starts none (a tool's,
/// such as a compiler's diagnostic under an `ERROR:`) takes the kind of the
/// one it follows ([`Kinds::kind`]).
const STARTS: &[&str] = &[
    "INFO:",
    "WARNING:",
    "ERROR:",
    "DEBUG:",
    "FAIL:",
    "TIMEOUT:",
    "Target ",
    "Loading:",
    "Analyzing:",
    "Executed ",
    "Build ",
    "//",
    "@",
    "[",
];

/// Each line's kind as a screen filtering the console by errors and
/// warnings reads it, in order: a line's own ([`severity`]); a line that
/// starts no line of Bazel's own (`STARTS`) and has none takes the kind
/// the last that did left, an `ERROR:`'s `error` (so a compiler's
/// diagnostic under its action's `ERROR:` is an error too), any other's
/// none (a `WARNING:` is its line alone, Bazel's one line, a tool's among
/// its output). A `--show_timestamps` prefix and leading spaces are left
/// out first.
#[derive(Debug, Default)]
pub struct Kinds {
    carried: &'static str,
}

impl Kinds {
    /// `text`'s kind, given the lines before it; what it leaves for the
    /// next.
    pub fn kind(&mut self, text: &str) -> &'static str {
        let own = severity(text);
        let line = lead_stripped(text);
        if STARTS.iter().any(|s| line.starts_with(s)) {
            self.carried = if line.starts_with("ERROR:") { own } else { "" };
            own
        } else if own.is_empty() {
            self.carried
        } else {
            own
        }
    }

    /// Whether `text`, written as `coloured` (`coloured_line`), is one of
    /// Bazel's own lines: it starts as one does (`STARTS`), and is not
    /// coloured as a tool colours `INFO:` (the colon inside the colour,
    /// which Bazel puts after its space).
    pub fn bazel(text: &str, coloured: &str) -> bool {
        let line = lead_stripped(text);
        STARTS.iter().any(|s| line.starts_with(s)) && !tools_own(coloured)
    }
}

/// `text` without a `--show_timestamps` prefix and leading spaces.
fn lead_stripped(text: &str) -> &str {
    super::line::message(text).trim_start_matches(' ')
}

/// `error`, `warning` or `` for a console line by itself: Bazel's `ERROR:`,
/// `FAIL:` and `TIMEOUT:` (indented too, as the recap lists them), a test
/// summary's `FAILED`, `TIMEOUT` and `FAILED TO BUILD`, and its `WARNING:`;
/// with or without a `--show_timestamps` prefix.
pub fn severity(text: &str) -> &'static str {
    let line = lead_stripped(text);
    if ["ERROR:", "FAIL:", "TIMEOUT:"]
        .iter()
        .any(|h| line.starts_with(h))
        || line.contains(" FAILED ")
        || line.ends_with(" FAILED")
        || line.contains(" TIMEOUT ")
        || line.contains(" FAILED TO BUILD")
    {
        "error"
    } else if line.starts_with("WARNING:") {
        "warning"
    } else {
        ""
    }
}

/// Whether `coloured` starts as a tool colours a severity: its SGR
/// sequences (at most 8), then an uppercase word, a colon and a reset
/// (`ESC[32mINFO:ESC[0m`), where Bazel writes the space inside its colour
/// too (`ESC[32mINFO: ESC[0m`).
fn tools_own(coloured: &str) -> bool {
    let mut rest = coloured;
    for _ in 0..8 {
        let Some(after) = rest.strip_prefix("\x1b[") else {
            break;
        };
        let Some(end) = after.find('m') else {
            return false;
        };
        rest = &after[end + 1..];
    }
    let Some((head, after)) = rest.split_once(':') else {
        return false;
    };
    !head.is_empty()
        && head.chars().all(char::is_alphabetic)
        && head.chars().any(char::is_uppercase)
        && !head.chars().any(char::is_lowercase)
        && after.starts_with("\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_coloured_line_keeps_its_colour_and_only_it() {
        let raw = b"\x1b[32mINFO:\x1b[0m done \x1b]8;;https://x\x1b\\x\x1b]8;;\x1b\\\x1b[2K";
        let coloured = coloured_line(raw);
        assert_eq!(coloured, "\x1b[32mINFO:\x1b[0m done x");
        assert_eq!(
            String::from_utf8_lossy(&ansi::strip(coloured.as_bytes())),
            plain_line(raw)
        );
        assert_eq!(
            coloured_line(b"\x1b[33m10%\r99%"),
            "\x1b[33m99%",
            "colour carried over a redraw"
        );
    }

    /// A line's OSC 8 links come out as spans of its drawable text, by
    /// character, across colour and a dropped control character.
    #[test]
    fn a_lines_links_are_kept_as_spans_of_its_text() {
        let feed = Feed::default();
        let url = "cursor://file/w/src/a.rs:3:1";
        feed.write(
            Source::Stderr,
            format!(
                "\x1b[31mERROR:\x1b[0m \u{7}é \x1b]8;;{url}\x1b\\src/a.rs:3\x1b]8;;\x1b\\ bad\n"
            )
            .as_bytes(),
        );
        feed.write(Source::Stderr, b"plain\n");
        let got = feed.drain();
        assert_eq!(got.lines, vec!["ERROR: é src/a.rs:3 bad", "plain"]);
        let (start, end, ref link) = got.links[0][0];
        let shown: String = got.lines[0]
            .chars()
            .skip(start as usize)
            .take((end - start) as usize)
            .collect();
        assert_eq!((shown.as_str(), link.as_str()), ("src/a.rs:3", url));
        assert!(got.links[1].is_empty());
    }

    /// A URL a tool wrote with a control in it (a C1 CSI, which a screen
    /// drawing it would run) is no link.
    #[test]
    fn a_url_with_a_control_in_it_is_no_link() {
        let feed = Feed::default();
        feed.write(
            Source::Stderr,
            "\x1b]8;;https://x.test/\u{9b}2J\x1b\\here\x1b]8;;\x1b\\\n".as_bytes(),
        );
        let got = feed.drain();
        assert_eq!(got.lines, vec!["here"]);
        assert!(got.links[0].is_empty(), "{:?}", got.links);
    }

    /// Each line's kind: its own, else what the last line of Bazel's left
    /// (an `ERROR:`'s, so its diagnostics are errors), across drains; a
    /// timestamp and an indent left out.
    #[test]
    fn a_lines_kind_is_its_own_or_its_errors() {
        let feed = Feed::default();
        for line in [
            "ERROR: a.cc:1: compile failed",
            "a.cc:1:2: error: no x",
            "  1 | x",
            "INFO: From y:",
            "y said",
            "WARNING: z",
            "z more",
            "(23:04:25) ERROR: //a:b failed",
        ] {
            feed.write(Source::Stderr, format!("{line}\n").as_bytes());
        }
        let got = feed.drain();
        assert_eq!(
            got.kinds,
            ["error", "error", "error", "", "", "warning", "", "error"]
        );
        feed.write(
            Source::Stderr,
            b"more\n  ERROR: //a:b failed\n//t:x    TIMEOUT in 4.0s\n//t:y FAILED\n",
        );
        assert_eq!(feed.drain().kinds, ["error", "error", "error", "error"]);
        for (line, want) in [
            ("INFO: z", ""),
            ("WARNING: y", "warning"),
            ("TIMEOUT: //t:x (Summary)", "error"),
            ("(23:04:25.123) WARNING: x", "warning"),
            ("(see a.cc) ERROR: x", ""),
        ] {
            assert_eq!(severity(line), want, "{line:?}");
        }
    }

    /// A line of Bazel's own starts as one does and is not coloured as a
    /// tool colours one.
    #[test]
    fn bazels_own_lines_are_told_from_a_tools() {
        let feed = Feed::default();
        feed.write(
            Source::Stderr,
            b"\x1b[32mINFO: \x1b[0mFrom x:\n\x1b[32mINFO:\x1b[0m a tool's\nplain output\n(12:00:00) Loading: 1 package\n//a:b PASSED\n",
        );
        assert_eq!(feed.drain().bazel, [true, false, false, true, true]);
        assert!(!tools_own("\x1b[1;31mFAILED: \x1b[0mx"));
        assert!(tools_own("\x1b[1m\x1b[31mERROR:\x1b[0m x"));
        assert!(!tools_own("no colour: x"));
    }

    #[test]
    fn whole_lines_are_kept_plain_and_a_partial_one_waits() {
        let feed = Feed::default();
        feed.write(Source::Stderr, b"\x1b[32mINFO:\x1b[0m one\r\ntw");
        assert_eq!(feed.drain().lines, vec!["INFO: one"]);
        feed.write(Source::Stderr, b"o\n");
        assert_eq!(feed.drain().lines, vec!["two"]);
        assert_eq!(feed.drain().lines, Vec::<String>::new());
    }

    #[test]
    fn a_line_is_what_a_terminal_would_leave_on_its_row() {
        let feed = Feed::default();
        // An escape split across writes is stripped whole.
        feed.write(Source::Stdout, b"\x1b[3");
        feed.write(Source::Stdout, b"2mINFO:\x1b[0m ok\n");
        // A redraw with carriage returns shows its last state.
        feed.write(Source::Stdout, b"10%\r20%\r30%\n");
        // Backspaces, a bell, a vertical tab and a C1 control go; a tab stays.
        feed.write(Source::Stdout, "a\x08b\x07c\x0bd\u{9b}e\tf\r\n".as_bytes());
        // A line ended \r\r\n (a PTY's ONLCR over \r\n) is the line.
        feed.write(Source::Stdout, b"abc\r\r\ndef\n");
        assert_eq!(
            feed.drain().lines,
            vec!["INFO: ok", "30%", "abcde\tf", "abc", "def"]
        );
        // Past the cut, redraws still show the newest state.
        feed.write(Source::Stdout, &vec![b'x'; MAX_LINE_BYTES + 10]);
        feed.write(Source::Stdout, b"\rdone");
        feed.write(Source::Stdout, b"\r");
        feed.write(Source::Stdout, b"\nnext\n");
        assert_eq!(feed.drain().lines, vec!["done", "next"]);
    }

    #[test]
    fn a_long_line_is_cut() {
        let feed = Feed::default();
        feed.write(Source::Stderr, &vec![b'x'; MAX_LINE_BYTES + 10]);
        feed.write(Source::Stderr, b"\n");
        assert_eq!(feed.drain().lines[0].len(), MAX_LINE_BYTES);
    }

    #[test]
    fn each_source_keeps_its_own_unended_line() {
        let feed = Feed::default();
        feed.write(Source::Stdout, b"a");
        feed.write(Source::Stderr, b"INFO: x\n");
        feed.write(Source::Stdout, b"b\n");
        assert_eq!(feed.drain().lines, vec!["INFO: x", "ab"]);
    }

    #[test]
    fn past_the_cap_the_oldest_lines_are_counted_instead() {
        let feed = Feed::default();
        for i in 0..MAX_LINES + 3 {
            feed.write(Source::Stderr, format!("{i}\n").as_bytes());
        }
        let got = feed.drain();
        assert_eq!(got.lines.len(), MAX_LINES);
        assert_eq!(got.lines[0], "3");
        assert_eq!(got.dropped, 3);
    }

    #[test]
    fn the_latest_progress_area_is_kept_until_the_console_ends() {
        let feed = Feed::default();
        feed.set_progress(vec!["[1 / 2] 1 action running".into()]);
        feed.set_progress(vec![
            "[2 / 3] 1 action running".into(),
            "    Compiling a.cc; 1s".into(),
        ]);
        assert_eq!(feed.drain().progress.len(), 2);
        assert_eq!(feed.drain().progress.len(), 2, "kept across drains");
        feed.write(Source::Stderr, b"last");
        feed.finish(Source::Stderr);
        let got = feed.drain();
        assert_eq!(
            (got.lines, got.progress),
            (vec!["last".to_string()], vec![])
        );
    }
}
