//! Which lines of a failure's output say what went wrong: the one choice
//! behind the console's recap of an error and the status surfaces' failure
//! snippets (`lib/log_snippet.axl`, through `std.excerpt`), so both show the
//! same lines.
//!
//! The excerpt is anchored on the first line that reads as an error, with
//! lines of context around it, plus the output's last lines: a compiler's
//! first error is usually the cause of the rest, and a traceback's error
//! comes after its frames. Output with no such line is shown by its tail.
//! A line Bazel adds about how to debug a failure (`--sandbox_debug`) is
//! never the anchor. A Starlark traceback has a cut of its own
//! ([`traceback`]): its BUILD file's frame and its error.

use std::ops::Range;

/// Words that mark a line as one saying what failed, matched lowercased as
/// substrings, for compilers and test runners alike.
pub const MARKERS: &[&str] = &[
    "error",
    "fail",
    "fatal",
    "panic",
    "traceback",
    "exception",
    "assert",
    "expected",
    "segfault",
    "sigsegv",
    "core dumped",
    "undefined reference",
    "timed out",
    "timeout",
];

/// Lines Bazel adds under a failure, which read as errors but say only how
/// to look closer.
const BAZELS_OWN: &[&str] = &["--sandbox_debug", "--verbose_failures"];

/// How big an excerpt is: at most `max_lines`, with `before` and `after`
/// lines around the anchor and the output's last `tail`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub max_lines: usize,
    pub before: usize,
    pub after: usize,
    pub tail: usize,
}

impl Shape {
    /// The console's recap: an error's first line of output and the two
    /// after it, one before for a header the tool prints, and its last.
    pub const RECAP: Shape = Shape {
        max_lines: 5,
        before: 1,
        after: 2,
        tail: 1,
    };
}

/// Whether `line` reads as one saying what failed.
pub fn is_marker(line: &str) -> bool {
    let low = line.to_lowercase();
    !BAZELS_OWN.iter().any(|own| low.contains(own)) && MARKERS.iter().any(|m| low.contains(m))
}

/// The spans of `lines` an excerpt shows, in order and apart: all of them
/// when they fit; else the first marker line with its context and the last
/// `tail` lines, one span when they meet, either using what is left of
/// `max_lines` for more context; else, with no marker, the last
/// `max_lines`. What falls between spans, or before the first, is left out.
pub fn select<S: AsRef<str>>(lines: &[S], shape: Shape) -> Vec<Range<usize>> {
    let n = lines.len();
    let max = shape.max_lines;
    if n <= max {
        return if n == 0 { vec![] } else { vec![0..n] };
    }
    if max == 0 {
        return vec![];
    }
    let Some(anchor) = lines.iter().position(|l| is_marker(l.as_ref())) else {
        return vec![n - max..n];
    };
    let tail = shape.tail.min(max - 1);
    let tail_start = n - tail;
    let budget = max - tail;
    let mut start = anchor.saturating_sub(shape.before);
    let mut end = (anchor + shape.after + 1).min(n);
    // Over budget, context after the anchor goes first, then before it.
    while end - start > budget && end > anchor + 1 {
        end -= 1;
    }
    while end - start > budget && start < anchor {
        start += 1;
    }
    // Under budget (an anchor near the start), more context after it.
    end = end.max((start + budget).min(n));
    if end >= tail_start {
        // The anchor's window runs into the tail: one span, which fits, as
        // `n - start` is at most `tail + budget`, and starts earlier when
        // that leaves room.
        return vec![start.min(n - max)..n];
    }
    if tail == 0 {
        return vec![start..end];
    }
    vec![start..end, tail_start..n]
}

/// What a Starlark traceback's first line says, or ends with when it says
/// more before it (`package contains errors: mf: Traceback (most recent
/// call last):`).
const TRACEBACK: &str = "Traceback (most recent call last):";

/// Which of `lines` a Starlark traceback's excerpt keeps, in order, when they
/// hold one that starts on the first line or the next (after a line saying
/// what failed, `in r rule //rf:x: `) and has an `Error in …:` line after
/// that: the lines up to its start, its first frame at a BUILD file (`File
/// "nm/BUILD.bazel", line 2`) and that frame's code line, and its last
/// `Error in` line. None for any other lines. The console's recap and
/// `std.excerpt(traceback = True)` both cut a traceback so.
pub fn traceback<S: AsRef<str>>(lines: &[S]) -> Option<Vec<usize>> {
    let start = lines
        .iter()
        .take(2)
        .position(|l| l.as_ref().trim().ends_with(TRACEBACK))?;
    let error = lines
        .iter()
        .rposition(|l| l.as_ref().trim_start().starts_with("Error in "))?;
    if error <= start {
        return None;
    }
    let mut kept: Vec<usize> = (0..=start).collect();
    if let Some(b) = (start..error).find(|&k| is_build_frame(lines[k].as_ref())) {
        kept.push(b);
        if b + 1 < error {
            kept.push(b + 1);
        }
    }
    kept.push(error);
    kept.dedup();
    Some(kept)
}

/// Whether `line` is a traceback's frame at a BUILD file.
fn is_build_frame(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("File \"")
        && (t.contains("/BUILD\"") || t.contains("/BUILD.bazel\"") || t.starts_with("File \"BUILD"))
}

/// `text` as the lines an excerpt is chosen from: escapes removed, a line
/// redrawn in place (`\r`) only as it was last drawn, a run of the same line
/// once, and no empty last line.
pub fn clean_lines(text: &[u8]) -> Vec<String> {
    let text = super::ansi::strip(text);
    let text = String::from_utf8_lossy(&text);
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.trim_end_matches('\r');
        let line = raw.rsplit('\r').next().unwrap_or(raw);
        if lines.last().is_some_and(|l| l == line) {
            continue;
        }
        lines.push(line.to_string());
    }
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(max_lines: usize, before: usize, after: usize, tail: usize) -> Shape {
        Shape {
            max_lines,
            before,
            after,
            tail,
        }
    }

    fn numbered(n: usize, marked: &[usize]) -> Vec<String> {
        (0..n)
            .map(|i| {
                if marked.contains(&i) {
                    format!("{i}: error: bad")
                } else {
                    format!("{i}: ok")
                }
            })
            .collect()
    }

    #[test]
    fn what_fits_is_shown_whole() {
        assert_eq!(select(&numbered(3, &[]), shape(5, 1, 1, 1)), vec![0..3]);
        assert_eq!(
            select::<String>(&[], shape(5, 1, 1, 1)),
            Vec::<Range<usize>>::new()
        );
    }

    #[test]
    fn no_marker_is_shown_by_its_tail() {
        assert_eq!(select(&numbered(10, &[]), shape(3, 1, 1, 1)), vec![7..10]);
    }

    #[test]
    fn the_first_error_is_the_anchor_and_the_tail_comes_too() {
        let lines = numbered(40, &[5, 20, 30]);
        assert_eq!(select(&lines, shape(6, 1, 2, 2)), vec![4..8, 38..40]);
    }

    #[test]
    fn an_anchor_near_the_end_meets_the_tail_in_one_span() {
        let lines = numbered(12, &[9]);
        assert_eq!(select(&lines, shape(5, 1, 2, 1)), vec![7..12]);
    }

    #[test]
    fn an_anchor_near_the_start_takes_more_after_it() {
        let lines = numbered(40, &[0]);
        assert_eq!(select(&lines, shape(6, 2, 1, 1)), vec![0..5, 39..40]);
    }

    #[test]
    fn context_after_goes_before_context_before_when_over_budget() {
        let lines = numbered(40, &[10]);
        assert_eq!(select(&lines, shape(4, 2, 5, 1)), vec![8..11, 39..40]);
        assert_eq!(select(&lines, shape(2, 2, 5, 1)), vec![10..11, 39..40]);
    }

    #[test]
    fn bazels_debugging_hint_is_never_the_anchor() {
        let mut lines = numbered(30, &[20]);
        lines[2] = "Use --sandbox_debug to see verbose messages from the sandbox and retain the sandbox build root for debugging".into();
        assert_eq!(select(&lines, shape(4, 1, 1, 1)), vec![19..22, 29..30]);
    }

    #[test]
    fn the_recap_keeps_an_error_and_its_last_line() {
        let mut lines = numbered(30, &[]);
        lines[3] = "src/a.cc:3:4: error: no member named 'x'".into();
        lines[29] = "1 error generated.".into();
        assert_eq!(select(&lines, Shape::RECAP), vec![2..6, 29..30]);
    }

    /// The recorded tracebacks' excerpts: a frame line at a BUILD file and
    /// its code line kept, the start's line with what it says before it,
    /// and a line saying what failed before the start.
    #[test]
    fn a_traceback_keeps_its_start_build_frame_and_error() {
        let nm = [
            "package contains errors: nm: Traceback (most recent call last):",
            "\tFile \"nm/BUILD.bazel\", line 2, column 6, in <toplevel>",
            "\t\touter(name = \"m\")",
            "\tFile \"nm/defs.bzl\", line 4, column 10, in outer",
            "\t\tinner(name = name + \"_i\")",
            "Error in fail: inner refuses m_i",
        ];
        assert_eq!(traceback(&nm), Some(vec![0, 1, 2, 5]));
        let rf = [
            "in r rule //rf:x: ",
            "Traceback (most recent call last):",
            "\tFile \"/ws/rf/defs.bzl\", line 2, column 9, in _impl",
            "\t\tfail(\"impl refuses \" + ctx.label.name)",
            "Error in fail: impl refuses x",
            "after",
        ];
        assert_eq!(traceback(&rf), Some(vec![0, 1, 4]));
    }

    /// No traceback: none at the start, or no `Error in` line after it.
    #[test]
    fn other_lines_are_no_traceback() {
        assert_eq!(
            traceback(&["a", "b", "Traceback (most recent call last):", "Error in x"]),
            None
        );
        assert_eq!(
            traceback(&["Traceback (most recent call last):", "  File \"a.py\""]),
            None
        );
        assert_eq!(
            traceback(&["Error in x", "Traceback (most recent call last):"]),
            None
        );
        assert_eq!(traceback::<&str>(&[]), None);
    }

    #[test]
    fn lines_are_cleaned_as_a_terminal_shows_them() {
        assert_eq!(
            clean_lines(b"\x1b[31mred\x1b[0m\nsame\nsame\n10%\r50%\r100%\r\nlast\n"),
            vec!["red", "same", "100%", "last"]
        );
        assert_eq!(clean_lines(b""), Vec::<String>::new());
    }
}
