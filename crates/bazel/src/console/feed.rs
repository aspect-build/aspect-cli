//! What a console shows, as data, for a caller drawing its own screen: the
//! lines the stages pass, and Bazel's progress area as it last drew it.
//!
//! The forwarder thread writes a [`Feed`]; the caller drains it whenever it
//! redraws. Lines are kept as text a screen can draw in one row
//! ([`plain_line`]: escape sequences stripped once the line has ended, so
//! one split across writes is still whole; a carriage return's redraw
//! resolved as a terminal would, keeping its newest state; other control
//! characters, a backspace among them, removed), up to [`MAX_LINES`]
//! between drains; older ones are counted in [`Drained::dropped`] instead.
//! The progress area is replaced on every redraw, so only the latest is
//! kept.

use std::collections::VecDeque;
use std::sync::Mutex;

use super::ansi;

/// Lines kept between drains; past it the oldest go.
pub const MAX_LINES: usize = 10_000;

/// The most of a line kept as written, escapes and all; the rest of it is
/// cut.
pub const MAX_LINE_BYTES: usize = 64 << 10;

#[derive(Debug, Default)]
pub struct Feed {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    lines: VecDeque<String>,
    /// The bytes of the line being written, not yet ended.
    partial: Vec<u8>,
    /// A carriage return written last, not yet known to end its line
    /// (`\r\n`) or to start a redraw over it.
    cr: bool,
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
    /// Take in bytes the forwarder would have written: whole lines are kept,
    /// a trailing partial one waits for its end.
    pub fn write(&self, bytes: &[u8]) {
        let mut s = self.state.lock().unwrap();
        for &b in bytes {
            // A redraw writes over the line so far: only its newest state is
            // kept, however long the line runs before it ends. A run of
            // carriage returns is one, as on a terminal.
            if std::mem::take(&mut s.cr) && b != b'\n' && b != b'\r' {
                s.partial.clear();
            }
            match b {
                b'\n' => {
                    let line = std::mem::take(&mut s.partial);
                    s.push(plain_line(&line));
                }
                b'\r' => s.cr = true,
                _ if s.partial.len() < MAX_LINE_BYTES => s.partial.push(b),
                _ => {}
            }
        }
    }

    /// Bazel's progress area as just drawn: `rows`, rendered.
    pub fn set_progress(&self, rows: Vec<String>) {
        self.state.lock().unwrap().progress = rows;
    }

    /// The console ended: a partial line is kept, and there is no progress
    /// area any more.
    pub fn finish(&self) {
        let mut s = self.state.lock().unwrap();
        let line = std::mem::take(&mut s.partial);
        s.cr = false;
        if !line.is_empty() {
            s.push(plain_line(&line));
        }
        s.progress.clear();
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

/// A line as written (`raw`, its newline gone) made drawable in one row:
/// escape sequences stripped, then every control character but a tab
/// dropped (C0, DEL and C1, a backspace removed rather than applied), as a
/// screen drawing it would be thrown out by them. A redraw's carriage return
/// was resolved as it was written ([`Feed::write`]).
pub fn plain_line(raw: &[u8]) -> String {
    String::from_utf8_lossy(&ansi::strip(raw))
        .chars()
        .filter(|&c| c == '\t' || !(c < ' ' || ('\u{7f}'..='\u{9f}').contains(&c)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_lines_are_kept_plain_and_a_partial_one_waits() {
        let feed = Feed::default();
        feed.write(b"\x1b[32mINFO:\x1b[0m one\r\ntw");
        assert_eq!(feed.drain().lines, vec!["INFO: one"]);
        feed.write(b"o\n");
        assert_eq!(feed.drain().lines, vec!["two"]);
        assert_eq!(feed.drain().lines, Vec::<String>::new());
    }

    #[test]
    fn a_line_is_what_a_terminal_would_leave_on_its_row() {
        let feed = Feed::default();
        // An escape split across writes is stripped whole.
        feed.write(b"\x1b[3");
        feed.write(b"2mINFO:\x1b[0m ok\n");
        // A redraw with carriage returns shows its last state.
        feed.write(b"10%\r20%\r30%\n");
        // Backspaces, a bell, a vertical tab and a C1 control go; a tab stays.
        feed.write("a\x08b\x07c\x0bd\u{9b}e\tf\r\n".as_bytes());
        // A line ended \r\r\n (a PTY's ONLCR over \r\n) is the line.
        feed.write(b"abc\r\r\ndef\n");
        assert_eq!(
            feed.drain().lines,
            vec!["INFO: ok", "30%", "abcde\tf", "abc", "def"]
        );
        // Past the cut, redraws still show the newest state.
        feed.write(&vec![b'x'; MAX_LINE_BYTES + 10]);
        feed.write(b"\rdone");
        feed.write(b"\r");
        feed.write(b"\nnext\n");
        assert_eq!(feed.drain().lines, vec!["done", "next"]);
    }

    #[test]
    fn a_long_line_is_cut() {
        let feed = Feed::default();
        feed.write(&vec![b'x'; MAX_LINE_BYTES + 10]);
        feed.write(b"\n");
        assert_eq!(feed.drain().lines[0].len(), MAX_LINE_BYTES);
    }

    #[test]
    fn past_the_cap_the_oldest_lines_are_counted_instead() {
        let feed = Feed::default();
        for i in 0..MAX_LINES + 3 {
            feed.write(format!("{i}\n").as_bytes());
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
        feed.write(b"last");
        feed.finish();
        let got = feed.drain();
        assert_eq!(
            (got.lines, got.progress),
            (vec!["last".to_string()], vec![])
        );
    }
}
