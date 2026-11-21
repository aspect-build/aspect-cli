//! What a console shows, as data, for a caller drawing its own screen: the
//! lines the stages pass, and Bazel's progress area as it last drew it.
//!
//! Writers each write as their own [`Source`], so a line one leaves unended
//! is not joined to the other's: the console's forwarder as stderr, and
//! `std.term`'s capture of the process's stdout and stderr as both. The
//! caller drains it whenever it redraws. Lines are kept as plain text,
//! escape sequences stripped, up to [`MAX_LINES`] between drains; older ones
//! are counted in [`Drained::dropped`] instead. The progress area is replaced
//! on every redraw, so only the latest is kept.

use std::collections::VecDeque;
use std::sync::Mutex;

use super::ansi;

/// Lines kept between drains; past it the oldest go.
pub const MAX_LINES: usize = 10_000;

/// The longest line kept; the rest of it is cut.
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
    lines: VecDeque<String>,
    /// The text of the line each source is writing, not yet ended.
    partial: [Vec<u8>; 2],
    dropped: u64,
    progress: Vec<String>,
}

/// What [`Feed::drain`] hands over.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Drained {
    /// Lines ended since the last drain, oldest first.
    pub lines: Vec<String>,
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
        let text = ansi::strip(bytes);
        let mut s = self.state.lock().unwrap();
        for &b in text.iter() {
            match b {
                b'\n' => {
                    let line = std::mem::take(&mut s.partial[source as usize]);
                    s.push(String::from_utf8_lossy(&line).into_owned());
                }
                b'\r' => {}
                _ if s.partial[source as usize].len() < MAX_LINE_BYTES => {
                    s.partial[source as usize].push(b)
                }
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
        if !line.is_empty() {
            s.push(String::from_utf8_lossy(&line).into_owned());
        }
        if source == Source::Stderr {
            s.progress.clear();
        }
    }

    /// The lines since the last drain, and the progress area now.
    pub fn drain(&self) -> Drained {
        let mut s = self.state.lock().unwrap();
        Drained {
            lines: s.lines.drain(..).collect(),
            dropped: std::mem::take(&mut s.dropped),
            progress: s.progress.clone(),
        }
    }
}

impl State {
    fn push(&mut self, line: String) {
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
