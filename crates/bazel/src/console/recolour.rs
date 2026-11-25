//! Bazel's own colours in the theme's `bazel.*` roles, when the theme's
//! `bazel` setting is `shared` ([`aspect_theme::Bazel`]): the colour Bazel
//! opens a message's label with (`ERROR:`, `WARNING:`, `INFO:`, `DEBUG:`, and
//! a test's `FAIL:`, `TIMEOUT:`, `PASSED:`, `FLAKY:`), its end-of-build
//! status (the curses row's `INFO:` and `FAILED:`, and the `INFO:` of its
//! `Build completed…` message and the `ERROR:` of `Build did NOT
//! complete…`), its progress count and phase (`[3 / 9]`, `Loading:`) with the
//! row's `N failed` and `last test:`, the result word of each test summary
//! line (`PASSED`, `FLAKY`, `FAILED`, `TIMEOUT`, `NO STATUS`, `FAILED TO
//! BUILD`), and the summary footer's `N fails locally`.
//!
//! Only a record the [`line`] classifier takes for Bazel's own is touched: a
//! message's header, or the first record after an action's output that starts
//! with Bazel's coloured label (its next message, more often than the
//! output's held last line); a status or summary row only outside any
//! action's or test's output. Only colours change, never the text, so a
//! tool's or a nested Bazel's output keeps its colour and a row curses
//! redraws keeps its width.
//!
//! The curses status row after the build (`INFO:` with no text, redrawn while
//! Bazel waits) takes the status role of Bazel's closing message before it.
//!
//! Under `--test_output=streamed` a test's output has no header, so nothing
//! from the announcement on can be told from it, until Bazel's closing
//! `Build completed…` or `Build did NOT complete…` message straight after its
//! `Elapsed time:` and `N processes:` lines (blank records and its curses
//! rows aside): that and every unindented line after it (the test summary,
//! its footer) are Bazel's again. Bazel's labels between streamed outputs
//! keep Bazel's colours.
//!
//! A nested Bazel in a test writes those same three lines, and Bazel draws a
//! streamed test's lines as it draws its own (under curses, each after a
//! redraw of its progress rows), so in every mode a nested Bazel's own
//! closing block is taken for Bazel's.
//!
//! Bazel writes its bold red as `ESC[31m` then `ESC[1m`; rendered to plain
//! lines the two arrive as one sequence, `ESC[1;31m`. Both are matched.
//!
//! The codes Bazel writes for `TIMEOUT`, `NO STATUS`, `FAILED TO BUILD` and
//! `N fails locally` are matched by word; no recorded console has them.

use std::sync::{Arc, LazyLock};

use regex::bytes::{Captures, Regex};

use super::forward::LineProcessor;
use super::line::{self, Kind, Line};

/// Escapes other than SGR, then an optional `--show_timestamps` prefix, at
/// the start of a record; then Bazel's colour.
const LEAD: &str =
    r"^((?:\x1b\[[0-9;?]*[A-Za-ln-z])*(?:\(\d{2}:\d{2}:\d{2}(?:\.\d+)?\) )?)((?:\x1b\[[0-9;]+m)+)";

static LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{LEAD}(ERROR|WARNING|INFO|DEBUG|FAIL|TIMEOUT|PASSED|FLAKY): "
    ))
    .unwrap()
});

static STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{LEAD}(INFO|FAILED):\x1b\[0m")).unwrap());

static PROGRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{LEAD}(?:\[[\d,]+ / [\d,]+\]|(?:Computing main repo mapping|Loading|Analyzing|Fetching|Building|Checking|Executing|Testing):)\x1b\[0m"
    ))
    .unwrap()
});

/// Bazel's bold red, as it writes it or as a rendered line merges it.
const RED_BOLD: &str = r"(?:\x1b\[31m\x1b\[1m|\x1b\[1m\x1b\[31m|\x1b\[(?:31;1|1;31)m)";

/// The `N failed` and `last test:` of Bazel's progress row, coloured as
/// Bazel colours them.
static PROGRESS_FAILED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{RED_BOLD}(\d[\d,]* failed)\x1b\[0m")).unwrap());
static LAST_PASSED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"last test: \x1b\[32m").unwrap());
static LAST_FAILED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"last test: {RED_BOLD}")).unwrap());

/// The test summary's footer, `Executed … tests: … N fails locally.`
static FAILS_LOCALLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{RED_BOLD}(\d[\d,]* fails? locally)\x1b\[0m")).unwrap());

/// A test summary line: a label, padding, then Bazel's reset and colour
/// before the result word.
static SUMMARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^((?:\x1b\[[0-9;?]*[A-Za-ln-z])*(?:@@?[^\s/]*)?//\S+ +)((?:\x1b\[[0-9;]+m)+)(?:\(cached\) )?(PASSED|FLAKY|FAILED TO BUILD|FAILED|TIMEOUT|NO STATUS)\x1b\[0m",
    )
    .unwrap()
});

/// The `bazel.*` roles' opening escapes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BazelColours {
    pub error: String,
    pub warning: String,
    pub info: String,
    pub debug: String,
    pub progress: String,
    pub status_success: String,
    pub status_failed: String,
    pub fail: String,
    pub test_passed: String,
    pub test_flaky: String,
    pub test_failed: String,
    pub test_no_status: String,
}

impl BazelColours {
    /// The roles of `palette`, None when its `bazel` setting is `tool` or it
    /// writes no SGR at all: Bazel's colours then pass as Bazel wrote them.
    pub fn of(palette: &aspect_theme::Palette) -> Option<BazelColours> {
        if palette.bazel() == aspect_theme::Bazel::Tool || palette.depth().is_none() {
            return None;
        }
        let role = |name: &str| palette.sgr(name);
        Some(BazelColours {
            error: role("bazel.error"),
            warning: role("bazel.warning"),
            info: role("bazel.info"),
            debug: role("bazel.debug"),
            progress: role("bazel.progress"),
            status_success: role("bazel.status.success"),
            status_failed: role("bazel.status.failed"),
            fail: role("bazel.fail"),
            test_passed: role("bazel.test.passed"),
            test_flaky: role("bazel.test.flaky"),
            test_failed: role("bazel.test.failed"),
            test_no_status: role("bazel.test.no_status"),
        })
    }

    /// The role of the label `word` on a message reading `text`: a build's
    /// last word on how it went takes the status roles.
    fn label(&self, word: &[u8], text: &str) -> &str {
        let rest = text.split_once(": ").map_or("", |(_, rest)| rest);
        match word {
            b"INFO" if rest.starts_with("Build completed successfully") => &self.status_success,
            b"INFO" if rest.starts_with("Build completed,") && rest.contains("FAILED") => {
                &self.status_failed
            }
            b"INFO" if rest.starts_with("Build completed") => &self.status_success,
            b"ERROR" if rest.starts_with("Build did NOT complete") => &self.status_failed,
            b"ERROR" => &self.error,
            b"WARNING" => &self.warning,
            b"INFO" => &self.info,
            b"DEBUG" => &self.debug,
            b"FAIL" | b"TIMEOUT" => &self.fail,
            b"PASSED" => &self.test_passed,
            b"FLAKY" => &self.test_flaky,
            _ => unreachable!("LABEL's words"),
        }
    }

    /// The status row's role: `INFO` is a success unless Bazel's closing
    /// message reported a failure (`failed`).
    fn status(&self, word: &[u8], failed: bool) -> &str {
        match word {
            b"INFO" if !failed => &self.status_success,
            _ => &self.status_failed,
        }
    }

    fn summary(&self, word: &[u8]) -> &str {
        match word {
            b"PASSED" => &self.test_passed,
            b"FLAKY" => &self.test_flaky,
            b"NO STATUS" => &self.test_no_status,
            _ => &self.test_failed,
        }
    }
}

/// The stage; see the module docs.
pub struct Recolour {
    colours: Arc<BazelColours>,
    line: Option<Line>,
    /// Bazel's closing message has come after streamed test output: what
    /// follows is Bazel's own again.
    closed: bool,
    /// Bazel's last record but blanks and its curses rows was its `Elapsed
    /// time:` or `N processes:` line, which come right before its closing
    /// message.
    ending: bool,
    /// Bazel's closing message reported a failure.
    failed: bool,
}

impl Recolour {
    pub fn new(colours: Arc<BazelColours>) -> Self {
        Recolour {
            colours,
            line: None,
            closed: false,
            ending: false,
            failed: false,
        }
    }
}

/// The text of message `text` after its label.
fn message(text: &str) -> &str {
    text.split_once(": ").map_or("", |(_, rest)| rest)
}

/// Whether `text`, a message, is Bazel's closing one.
fn closes_build(text: &str) -> bool {
    let rest = message(text);
    rest.starts_with("Build completed") || rest.starts_with("Build did NOT complete")
}

/// Whether closing message `text` reports a failure.
fn reports_failure(text: &str) -> bool {
    let rest = message(text);
    rest.starts_with("Build did NOT complete") || rest.contains("FAILED")
}

/// Whether `text`, a message, is one of the lines Bazel writes right before
/// its closing one: `Elapsed time: …` and `N processes: …` (`1 process: …`).
fn ends_build(text: &str) -> bool {
    let rest = message(text);
    let count = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    rest.starts_with("Elapsed time: ")
        || (count.len() < rest.len()
            && (count.starts_with(" processes: ") || count.starts_with(" process: ")))
}

impl LineProcessor for Recolour {
    fn see(&mut self, line: &Line) {
        self.line = Some(line.clone());
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        let Some(mut line) = self.line.take() else {
            return Some(record.to_vec());
        };
        let label = LABEL.is_match(record) && !line.indented;
        if line.streamed {
            self.closed |= label && self.ending && closes_build(&line.text);
            if self.closed && !line.indented {
                line.in_output = false;
                if label && line.kind == Kind::Other {
                    line.kind = Kind::Header;
                    line.coloured = true;
                }
            }
        }
        if line.kind != Kind::Blank && !line.drawn {
            self.ending = label && ends_build(&line.text);
        }
        let bazels = (line.kind == Kind::Header && line.coloured) || line.tail;
        if bazels && label && closes_build(&line.text) {
            self.failed = reports_failure(&line.text);
        }
        Some(recolour(&self.colours, &line, record, self.failed))
    }
}

/// `record` with the colour in `caps`' second group replaced by `sgr`,
/// keeping a reset Bazel put first.
fn swap(record: &[u8], caps: &Captures, sgr: &str) -> Vec<u8> {
    let colour = caps.get(2).expect("a colour");
    let mut out = Vec::with_capacity(record.len() + sgr.len() + 4);
    out.extend_from_slice(&record[..colour.start()]);
    if colour.as_bytes().starts_with(b"\x1b[0m") {
        out.extend_from_slice(b"\x1b[0m");
    }
    out.extend_from_slice(sgr.as_bytes());
    out.extend_from_slice(&record[colour.end()..]);
    out
}

/// Every match of `re` in `record`, its colour up to the first group
/// replaced by `sgr`.
fn swap_all(record: Vec<u8>, re: &Regex, sgr: &str, keep: &[u8]) -> Vec<u8> {
    re.replace_all(&record, |caps: &Captures| {
        let mut out = keep.to_vec();
        out.extend_from_slice(sgr.as_bytes());
        if let Some(word) = caps.get(1) {
            out.extend_from_slice(word.as_bytes());
            out.extend_from_slice(b"\x1b[0m");
        }
        out
    })
    .into_owned()
}

/// `record`, classified as `line`, with Bazel's colours replaced; `failed`
/// when Bazel's closing message before it reported a failure.
pub fn recolour(colours: &BazelColours, line: &Line, record: &[u8], failed: bool) -> Vec<u8> {
    if line.drawn && !line.in_output && line::is_bazel_status(record) {
        if let Some(caps) = STATUS.captures(record) {
            return swap(record, &caps, colours.status(&caps[3], failed));
        }
    }
    if (line.kind == Kind::Header && line.coloured) || line.tail {
        if let Some(caps) = LABEL.captures(record) {
            return swap(record, &caps, colours.label(&caps[3], &line.text));
        }
    }
    if line.kind.is_progress() || line.drawn {
        if let Some(caps) = PROGRESS.captures(record) {
            let out = swap(record, &caps, &colours.progress);
            let out = swap_all(out, &PROGRESS_FAILED, &colours.test_failed, b"");
            let out = swap_all(out, &LAST_PASSED, &colours.test_passed, b"last test: ");
            return swap_all(out, &LAST_FAILED, &colours.test_failed, b"last test: ");
        }
    }
    if !line.indented && !line.opens_output && !line.in_output {
        if let Some(caps) = SUMMARY.captures(record) {
            return swap(record, &caps, colours.summary(&caps[3]));
        }
        if line.text.starts_with("Executed ") {
            return swap_all(record.to_vec(), &FAILS_LOCALLY, &colours.test_failed, b"");
        }
    }
    record.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::line::Classifier;
    use aspect_theme::{Depth, Palette, Theme};

    fn run(colours: &BazelColours, records: &[&str]) -> Vec<String> {
        let mut classifier = Classifier::default();
        records
            .iter()
            .map(|r| {
                let line = classifier.classify(r.as_bytes());
                String::from_utf8(recolour(colours, &line, r.as_bytes(), false)).unwrap()
            })
            .collect()
    }

    fn dark() -> BazelColours {
        BazelColours::of(&Palette::aspect_dark(Depth::Ansi256)).unwrap()
    }

    #[test]
    fn bazel_labels_and_progress_take_the_theme_colours() {
        let out = run(
            &dark(),
            &[
                "\x1b[31m\x1b[1mERROR: \x1b[0m/ws/BUILD:1:1: no such target",
                "\x1b[35mWARNING: \x1b[0mdeprecated",
                "\x1b[32mINFO: \x1b[0mAnalyzed 3 targets",
                "\x1b[33mDEBUG: \x1b[0m/ws/defs.bzl:1:6: hello",
                "(12:00:00) \x1b[35mWARNING: \x1b[0mstamped",
                "\x1b[32m[3 / 9]\x1b[0m Compiling a.cc",
                "\x1b[32mLoading:\x1b[0m 1 packages loaded",
                "\x1b[1A\x1b[K\x1b[31m\x1b[1mERROR: \x1b[0mafter curses",
            ],
        );
        assert_eq!(
            out,
            [
                "\x1b[1;38;5;203mERROR: \x1b[0m/ws/BUILD:1:1: no such target",
                "\x1b[1;38;5;221mWARNING: \x1b[0mdeprecated",
                "\x1b[1;38;5;247mINFO: \x1b[0mAnalyzed 3 targets",
                "\x1b[38;5;247mDEBUG: \x1b[0m/ws/defs.bzl:1:6: hello",
                "(12:00:00) \x1b[1;38;5;221mWARNING: \x1b[0mstamped",
                "\x1b[38;5;247m[3 / 9]\x1b[0m Compiling a.cc",
                "\x1b[38;5;247mLoading:\x1b[0m 1 packages loaded",
                "\x1b[1A\x1b[K\x1b[1;38;5;203mERROR: \x1b[0mafter curses",
            ]
        );
    }

    /// The forms in `testdata/` (Bazel 7 to 9): the status line, the test
    /// headers and the test summary words.
    #[test]
    fn the_status_line_test_headers_and_summary_words() {
        let out = run(
            &dark(),
            &[
                "\x1b[32mINFO:\x1b[0m ",
                "\x1b[31m\x1b[1mFAILED:\x1b[0m ",
                "\x1b[31m\x1b[1mFAIL: \x1b[0m//:t_fail (Exit 1)",
                "\x1b[0m\x1b[32mPASSED: \x1b[0m//:t_pass (see /tmp/test.log)",
                "\x1b[35mFLAKY: \x1b[0m//:flaky (Summary)",
                "//:t_pass                  \x1b[0m\x1b[32mPASSED\x1b[0m in 0.4s",
                "//:t_pass                  \x1b[0m\x1b[32m(cached) PASSED\x1b[0m in 0.4s",
                "//:t_fail                  \x1b[0m\x1b[31m\x1b[1mFAILED\x1b[0m in 0.6s",
                "//:flaky                   \x1b[0m\x1b[35mFLAKY\x1b[0m, failed in 1 out of 2",
                "@@r//:t                    \x1b[0m\x1b[35mNO STATUS\x1b[0m",
            ],
        );
        assert_eq!(
            out,
            [
                "\x1b[1;38;5;79mINFO:\x1b[0m ",
                "\x1b[1;38;5;203mFAILED:\x1b[0m ",
                "\x1b[1;38;5;203mFAIL: \x1b[0m//:t_fail (Exit 1)",
                "\x1b[0m\x1b[38;5;79mPASSED: \x1b[0m//:t_pass (see /tmp/test.log)",
                "\x1b[38;5;221mFLAKY: \x1b[0m//:flaky (Summary)",
                "//:t_pass                  \x1b[0m\x1b[38;5;79mPASSED\x1b[0m in 0.4s",
                "//:t_pass                  \x1b[0m\x1b[38;5;79m(cached) PASSED\x1b[0m in 0.4s",
                "//:t_fail                  \x1b[0m\x1b[38;5;203mFAILED\x1b[0m in 0.6s",
                "//:flaky                   \x1b[0m\x1b[38;5;221mFLAKY\x1b[0m, failed in 1 out of 2",
                "@@r//:t                    \x1b[0m\x1b[38;5;247mNO STATUS\x1b[0m",
            ]
        );
    }

    #[test]
    fn output_that_is_not_bazels_keeps_its_colour() {
        let out = run(
            &dark(),
            &[
                "\x1b[32mINFO: \x1b[0mFrom Compiling a.cc:",
                "  \x1b[35mWARNING: \x1b[0mnested, indented",
                "\x1b[31mred tool text\x1b[0m",
                "WARNING: uncoloured",
                "  //:x   \x1b[0m\x1b[32mPASSED\x1b[0m indented",
            ],
        );
        assert_eq!(out[1], "  \x1b[35mWARNING: \x1b[0mnested, indented");
        assert_eq!(out[2], "\x1b[31mred tool text\x1b[0m");
        assert_eq!(out[3], "WARNING: uncoloured");
        assert_eq!(out[4], "  //:x   \x1b[0m\x1b[32mPASSED\x1b[0m indented");
    }

    /// A tool's own status-shaped and summary-shaped lines, in an action's
    /// output or streamed test output, keep the tool's colours.
    #[test]
    fn status_and_summary_shapes_in_tool_output_are_the_tools() {
        let tool_status = "\x1b[32mINFO:\x1b[0m tool status line";
        let tool_summary = "//fake:target   \x1b[0m\x1b[32mPASSED\x1b[0m in 0.1s";
        let out = run(
            &dark(),
            &[
                "\x1b[32mINFO: \x1b[0mFrom Executing genrule //:g:",
                tool_status,
                tool_summary,
            ],
        );
        assert_eq!(out[1], tool_status);
        assert_eq!(out[2], tool_summary);
        let out = run(
            &dark(),
            &[
                "\x1b[32mINFO: \x1b[0mStreamed test output requested so all tests will be run locally",
                tool_status,
                tool_summary,
            ],
        );
        assert_eq!(out[1], tool_status);
        assert_eq!(out[2], tool_summary);
    }

    /// Bazel's first message after an action's output, which the classifier
    /// cannot tell from the output's last line, still takes the theme.
    #[test]
    fn the_message_after_action_output_is_recoloured() {
        let out = run(
            &dark(),
            &[
                "\x1b[32mINFO: \x1b[0mFrom Compiling a.cc:",
                "a.cc:1: warning: x",
                "\x1b[32m[1 / 2]\x1b[0m Compiling b.cc",
                "\x1b[32mINFO: \x1b[0mFound 1 target...",
            ],
        );
        assert_eq!(out[3], "\x1b[1;38;5;247mINFO: \x1b[0mFound 1 target...");
    }

    /// A build's last word is its status, in the message form that stays as
    /// well as in the curses row that is erased.
    #[test]
    fn the_build_completed_message_takes_the_status_roles() {
        let out = run(
            &dark(),
            &[
                "\x1b[32mINFO: \x1b[0mBuild completed successfully, 1 total action",
                "\x1b[32mINFO: \x1b[0mBuild completed, 1 test FAILED, 12 total actions",
                "\x1b[31m\x1b[1mERROR: \x1b[0mBuild did NOT complete successfully",
                "\x1b[32mINFO: \x1b[0mElapsed time: 3.7s",
            ],
        );
        assert_eq!(
            out[0],
            "\x1b[1;38;5;79mINFO: \x1b[0mBuild completed successfully, 1 total action"
        );
        assert_eq!(
            out[1],
            "\x1b[1;38;5;203mINFO: \x1b[0mBuild completed, 1 test FAILED, 12 total actions"
        );
        assert_eq!(
            out[2],
            "\x1b[1;38;5;203mERROR: \x1b[0mBuild did NOT complete successfully"
        );
        assert_eq!(out[3], "\x1b[1;38;5;247mINFO: \x1b[0mElapsed time: 3.7s");
    }

    /// The progress row's `N failed` and `last test:`, and the summary
    /// footer's `N fails locally`, as `testdata/` has Bazel colour them.
    #[test]
    fn the_progress_rows_test_words_and_the_footer() {
        let out = run(
            &dark(),
            &[
                "\x1b[32m[14 / 14]\x1b[0m 2 / 2 tests, \x1b[31m\x1b[1m1 failed\x1b[0m;\x1b[0m no actions running\x1b[0m; last test: \x1b[31m\x1b[1m//:t_fail\x1b[0m",
                "\x1b[32m[2 / 2]\x1b[0m 1 / 1 tests;\x1b[0m no actions running\x1b[0m; last test: \x1b[32m//:t_pass\x1b[0m",
                "Executed 2 out of 2 tests: 1 test passes and \x1b[31m\x1b[1m1 fails locally\x1b[0m.",
            ],
        );
        assert_eq!(
            out[0],
            "\x1b[38;5;247m[14 / 14]\x1b[0m 2 / 2 tests, \x1b[38;5;203m1 failed\x1b[0m;\x1b[0m no actions running\x1b[0m; last test: \x1b[38;5;203m//:t_fail\x1b[0m"
        );
        assert_eq!(
            out[1],
            "\x1b[38;5;247m[2 / 2]\x1b[0m 1 / 1 tests;\x1b[0m no actions running\x1b[0m; last test: \x1b[38;5;79m//:t_pass\x1b[0m"
        );
        assert_eq!(
            out[2],
            "Executed 2 out of 2 tests: 1 test passes and \x1b[38;5;203m1 fails locally\x1b[0m."
        );
    }

    /// `records` through the stage, which keeps state across them.
    fn staged(colours: &BazelColours, records: &[&str]) -> Vec<String> {
        let mut classifier = Classifier::default();
        let mut stage = Recolour::new(Arc::new(colours.clone()));
        records
            .iter()
            .map(|r| {
                stage.see(&classifier.classify(r.as_bytes()));
                String::from_utf8(stage.process(r.as_bytes()).unwrap()).unwrap()
            })
            .collect()
    }

    /// Rendered to plain lines (a pipe with colour: CI), Bazel's bold red
    /// arrives as one sequence; it is still Bazel's.
    #[test]
    fn bazels_bold_red_is_matched_as_rendered_lines_merge_it() {
        let out = run(
            &dark(),
            &[
                "\x1b[32m[14 / 14]\x1b[0m 2 / 2 tests, \x1b[1;31m1 failed\x1b[0m;\x1b[0m no actions running\x1b[0m; last test: \x1b[1;31m//:t_fail\x1b[0m",
                "Executed 2 out of 2 tests: 1 test passes and \x1b[1;31m1 fails locally\x1b[0m.",
            ],
        );
        assert!(
            out[0].contains("\x1b[38;5;203m1 failed\x1b[0m"),
            "{:?}",
            out[0]
        );
        assert!(
            out[0].contains("last test: \x1b[38;5;203m//:t_fail"),
            "{:?}",
            out[0]
        );
        assert!(!out[0].contains("1;31"), "{:?}", out[0]);
        assert_eq!(
            out[1],
            "Executed 2 out of 2 tests: 1 test passes and \x1b[38;5;203m1 fails locally\x1b[0m."
        );
    }

    /// Streamed test output has no header, so Bazel's labels inside it keep
    /// its colours; from Bazel's closing message on, the block is Bazel's.
    #[test]
    fn streamed_test_output_and_the_closing_block() {
        let out = staged(
            &dark(),
            &[
                "\x1b[32mINFO: \x1b[0mStreamed test output requested so all tests will be run locally",
                "\x1b[32mINFO: \x1b[0ma test printing an INFO line",
                "\x1b[32mINFO: \x1b[0mElapsed time: 0.496s, Critical Path: 0.22s",
                "\x1b[32mINFO: \x1b[0m3 processes: 11 action cache hit, 2 internal, 1 darwin-sandbox.",
                "\x1b[32mINFO: \x1b[0mBuild completed, 1 test FAILED, 4 total actions",
                "//:t_fail                  \x1b[0m\x1b[1;31mFAILED\x1b[0m in 0.1s",
                "  /tmp/test.log",
                "Executed 1 out of 1 test: \x1b[1;31m1 fails locally\x1b[0m.",
            ],
        );
        assert_eq!(
            out[1], "\x1b[32mINFO: \x1b[0ma test printing an INFO line",
            "raw"
        );
        assert_eq!(
            out[4],
            "\x1b[1;38;5;203mINFO: \x1b[0mBuild completed, 1 test FAILED, 4 total actions"
        );
        assert_eq!(
            out[5],
            "//:t_fail                  \x1b[0m\x1b[38;5;203mFAILED\x1b[0m in 0.1s"
        );
        assert_eq!(
            out[7],
            "Executed 1 out of 1 test: \x1b[38;5;203m1 fails locally\x1b[0m."
        );
    }

    /// `bytes`, a recorded console, split at each line end and carriage
    /// return as the splitter splits it, its blank records left out.
    fn records(bytes: &[u8]) -> Vec<&str> {
        std::str::from_utf8(bytes)
            .unwrap()
            .split(['\r', '\n'])
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// The curses status row Bazel redraws after its closing message carries
    /// no text (`testdata/pty_bes_narrow.txt`): it takes that message's role.
    #[test]
    fn the_status_row_after_a_failed_build_reports_a_failure() {
        let narrow = include_bytes!("testdata/pty_bes_narrow.txt");
        let rows: Vec<&str> = records(narrow)
            .into_iter()
            .filter(|r| r.ends_with("\x1b[32mINFO:\x1b[0m "))
            .collect();
        assert!(rows.len() >= 5, "{rows:?}");
        let after = |closing: &'static str| {
            let mut all = vec![closing];
            all.extend(&rows);
            staged(&dark(), &all)
        };
        for closing in [
            "\x1b[1A\x1b[K\x1b[31m\x1b[1mERROR: \x1b[0mBuild did NOT complete successfully",
            "\x1b[1A\x1b[K\x1b[32mINFO: \x1b[0mBuild completed, 2 tests FAILED, 17 total actions",
        ] {
            for row in &after(closing)[1..] {
                assert!(row.contains("\x1b[1;38;5;203mINFO:"), "{row:?}");
            }
        }
        let passed =
            after("\x1b[1A\x1b[K\x1b[32mINFO: \x1b[0mBuild completed successfully, 1 total action");
        for row in &passed[1..] {
            assert!(row.contains("\x1b[1;38;5;79mINFO:"), "{row:?}");
        }
    }

    /// A palette that writes no SGR (`--color=yes` into a pipe, `color_depth`
    /// 0) leaves Bazel's colours alone rather than blanking them.
    #[test]
    fn no_depth_recolours_nothing() {
        assert_eq!(BazelColours::of(&Palette::plain()), None);
    }

    #[test]
    fn tool_mode_recolours_nothing() {
        let mut theme = Theme::aspect_dark();
        theme.bazel = Some(aspect_theme::Bazel::Tool);
        assert_eq!(
            BazelColours::of(&Palette::new(theme, Some(Depth::Ansi256), false)),
            None
        );
    }
}
