//! Bazel's curses output turned into plain lines, for a destination that is
//! not a terminal or asked for none: what a terminal would leave on screen,
//! without the rows curses drew only to erase them.
//!
//! [`Screen`] models the rows below the last one that can no longer change,
//! as cells: text is written over the cursor's row from its column, each
//! character in the colour then in effect, `\r` goes to the row's start,
//! `\n` down, `ESC[<n>A` up, `ESC[<n>B` down to the last row, and `ESC[K`
//! clears the rest of the row. Colour sequences are parsed into a colour
//! state (foreground, background, attributes); a row given back carries
//! each state as one sequence where it changes and ends reset, so no colour
//! reaches another row and no cut leaves part of a sequence. Curses erases
//! its progress area by moving up over it, then writes the next message at
//! its top; so once text is written after a move up, every row above the
//! cursor is permanent, and is given back as a [`Row`]. A row it erased is
//! gone. What is kept or erased does not depend on the classifier: a tool's
//! own line, however it looks, stays unless the stream erases it. Each row
//! carries what the record that last wrote it was classified as, for the
//! stages that run on the rows, and for when: a finished row classified as
//! not [`Line::drawn`] is settled without waiting for the next redraw, since
//! curses only erases rows it drew (but for a blank last row, which Bazel
//! writes as a timestamp-only row and erases). The one approximation: a tool
//! redrawing its own rows shows their earlier states too, since they were
//! given back before the stream moved up over them, and one erasing its own
//! last line leaves it showing; nothing below is lost.
//!
//! With `status_every`, the first row of an erased progress area (Bazel's
//! `[n / m] …` or `Analyzing: …` line) is given back too, marked
//! [`Row::status`], at most that often and the first once the build has run
//! that long, so a log shows a long build is still moving. A progress area
//! curses left unerased at the end is not given back, told by Bazel's
//! colouring of its first or status line and the classifier marking every
//! row under it drawn; one left mid-stream (on an interrupt) stays, as a
//! terminal shows it, since output Bazel coloured the same way cannot be told
//! from it there.

use std::time::{Duration, Instant};

use super::ansi;
use super::line::{self, Line};

/// Pending rows past which the oldest is given back regardless: a stream
/// that never moves up has nothing left to erase. Also the most a cursor
/// count moves, and the most rows a move up past them inserts.
const MAX_PENDING: usize = 512;

/// Cells pending rows may hold before the oldest finished ones are given back.
const MAX_PENDING_CELLS: usize = 1 << 20;

/// Distinct colour states kept; past it, text in a new one is written
/// uncoloured rather than in another's colour.
const MAX_STYLES: usize = 1 << 16;

/// The bit of [`Style::flags`] for overline (SGR 53).
const OVERLINE: u16 = 10;

/// A colour, as SGR parameters set it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Colour {
    #[default]
    Default,
    /// One of the 16: its parameter for a foreground (30–37, 90–97).
    Basic(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// The colour state text is written in: what SGR parameters have set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Style {
    /// Bold, dim, italic, underline, blink, inverse, hidden, strike: bits 1
    /// to 9 by their SGR parameter.
    flags: u16,
    fg: Colour,
    bg: Colour,
}

impl Style {
    /// Apply the parameters of one SGR sequence (`1;31`, `38;5;244`,
    /// `38:2::255:0:0`, `00`), as xterm does: `;` separates parameters, `:`
    /// a parameter's own sub-parameters, an empty parameter is 0, and one it
    /// does not know (or a value out of range) is ignored.
    fn apply(&mut self, params: &[u8]) {
        let text = std::str::from_utf8(params).unwrap_or("");
        if text.is_empty() {
            *self = Style::default();
            return;
        }
        let num = |s: &str| -> Option<u32> {
            if s.is_empty() {
                Some(0)
            } else {
                s.parse::<u32>().ok()
            }
        };
        let byte = |n: Option<u32>| n.filter(|n| *n <= 255).map(|n| n as u8);
        let mut groups = text.split(';');
        while let Some(group) = groups.next() {
            let mut sub = group.split(':');
            let Some(p) = sub.next().and_then(num) else {
                continue;
            };
            let subs: Vec<&str> = sub.collect();
            // A colour in either form: `38;5;n` / `38;2;r;g;b`, or one group
            // `38:5:n` / `38:2:r:g:b` / `38:2:<space>:r:g:b`.
            let mut colour = || -> Option<Colour> {
                let args: Vec<Option<u32>> = if subs.is_empty() {
                    match groups.next().and_then(num) {
                        Some(5) => vec![Some(5), groups.next().and_then(num)],
                        Some(2) => vec![
                            Some(2),
                            groups.next().and_then(num),
                            groups.next().and_then(num),
                            groups.next().and_then(num),
                        ],
                        _ => vec![],
                    }
                } else {
                    let mut a: Vec<Option<u32>> = subs.iter().map(|s| num(s)).collect();
                    // `2:<colour space>:r:g:b`: the space is not kept.
                    if a.first() == Some(&Some(2)) && a.len() >= 5 {
                        a.remove(1);
                    }
                    a
                };
                match args.as_slice() {
                    [Some(5), n] => byte(*n).map(Colour::Indexed),
                    [Some(2), r, g, b] => Some(Colour::Rgb(byte(*r)?, byte(*g)?, byte(*b)?)),
                    _ => None,
                }
            };
            match p {
                0 => *self = Style::default(),
                // `4:n`: an underline style, 0 for none.
                4 if !subs.is_empty() => {
                    if num(subs[0]) == Some(0) {
                        self.flags &= !(1 << 4);
                    } else {
                        self.flags |= 1 << 4;
                    }
                }
                1..=9 => self.flags |= 1 << p,
                // Double underline, as xterm takes it.
                21 => self.flags |= 1 << 4,
                22 => self.flags &= !((1 << 1) | (1 << 2)),
                // Blink off ends either blink; 26 is proportional spacing.
                25 => self.flags &= !((1 << 5) | (1 << 6)),
                26 => {}
                23..=29 => self.flags &= !(1 << (p - 20)),
                53 => self.flags |= 1 << OVERLINE,
                55 => self.flags &= !(1 << OVERLINE),
                30..=37 | 90..=97 => self.fg = Colour::Basic(p as u8),
                40..=47 | 100..=107 => self.bg = Colour::Basic((p - 10) as u8),
                39 => self.fg = Colour::Default,
                49 => self.bg = Colour::Default,
                38 => {
                    if let Some(c) = colour() {
                        self.fg = c;
                    }
                }
                48 => {
                    if let Some(c) = colour() {
                        self.bg = c;
                    }
                }
                // Underline colour: its arguments are taken, and not kept.
                58 => {
                    colour();
                }
                _ => {}
            }
        }
    }

    /// The SGR sequence that sets this state from none.
    fn sgr(&self) -> Vec<u8> {
        let mut ps: Vec<String> = (1..=9)
            .filter(|b| self.flags & (1 << b) != 0)
            .map(|b| b.to_string())
            .collect();
        if self.flags & (1 << OVERLINE) != 0 {
            ps.push("53".into());
        }
        let colour = |c: Colour, base: u16, ps: &mut Vec<String>| match c {
            Colour::Default => {}
            Colour::Basic(n) => ps.push((n as u16 + base - 30).to_string()),
            Colour::Indexed(n) => ps.push(format!("{};5;{n}", base + 8)),
            Colour::Rgb(r, g, b) => ps.push(format!("{};2;{r};{g};{b}", base + 8)),
        };
        colour(self.fg, 30, &mut ps);
        colour(self.bg, 40, &mut ps);
        format!("\x1b[{}m", ps.join(";")).into_bytes()
    }
}

/// A row that can no longer change: its bytes (text, and the colour changes
/// between its characters, ending reset; no cursor movement), what wrote
/// it, and whether it is a status line `status_every` let through.
#[derive(Debug)]
pub struct Row {
    pub bytes: Vec<u8>,
    pub line: Option<Line>,
    pub status: bool,
}

/// One character on a row, and the colour it was written in: an index
/// into [`Screen::styles`], 0 for none.
#[derive(Clone, Copy, Default, PartialEq)]
struct Cell {
    ch: [u8; 4],
    len: u8,
    style: u32,
}

#[derive(Default, Clone)]
struct Pending {
    cells: Vec<Cell>,
    line: Option<Line>,
    /// Made for a move up past the rows already given back, and not written
    /// since: not a row the stream wrote, so not given back.
    inserted: bool,
}

pub struct Screen {
    /// The rows that may still change, oldest first.
    rows: Vec<Pending>,
    /// The cursor's row, an index into `rows` (`rows.len()` is a new row).
    row: usize,
    /// Its column, in cells.
    col: usize,
    /// The colour state text is written in now, and its index in `styles`.
    style: Style,
    style_id: u32,
    /// The colour states seen, by index (0 is none), and their indices.
    styles: Vec<Style>,
    style_ids: std::collections::HashMap<Style, u32>,
    /// The cursor moved up since text was last written: the next text ends
    /// an erase.
    moved_up: bool,
    status_every: Option<Duration>,
    /// When the last status line was given back, or the screen was made.
    last_status: Instant,
    /// The first row of the progress area last erased, for `status_every`.
    erased_status: Option<Pending>,
}

impl Screen {
    pub fn new(status_every: Option<Duration>) -> Self {
        Self {
            rows: vec![],
            row: 0,
            col: 0,
            style: Style::default(),
            style_id: 0,
            styles: vec![Style::default()],
            style_ids: std::collections::HashMap::from([(Style::default(), 0)]),
            moved_up: false,
            status_every,
            // The first status line comes only once a build has run that long.
            last_status: Instant::now(),
            erased_status: None,
        }
    }

    /// Take in `bytes` of the stream, a record and its boundary classified
    /// as `line`; the rows that became permanent.
    pub fn feed(&mut self, bytes: &[u8], line: Option<&Line>) -> Vec<Row> {
        let mut out = vec![];
        for (is_escape, seg) in ansi::segments(bytes) {
            if is_escape {
                self.escape(seg);
                continue;
            }
            let mut text = seg;
            while let Some(i) = text.iter().position(|&b| b == b'\r' || b == b'\n') {
                self.text(&text[..i], line, &mut out);
                if text[i] == b'\n' {
                    self.row += 1;
                }
                self.col = 0;
                text = &text[i + 1..];
            }
            self.text(text, line, &mut out);
        }
        // A finished row curses did not draw is settled without waiting for
        // the next redraw: curses only erases rows it drew. Not a blank last
        // row, though: Bazel writes a timestamp-only row (`(11:31:13) `) and
        // erases it at once.
        let finished = self.row.min(self.rows.len());
        let settled = self.rows[..finished]
            .iter()
            .enumerate()
            .take_while(|(i, r)| {
                r.line.as_ref().is_some_and(|l| {
                    !(l.drawn || (i + 1 == finished && l.kind == line::Kind::Blank))
                })
            })
            .count();
        self.commit(settled, &mut out);
        if self.rows.len() > MAX_PENDING {
            let over = self.rows.len() - MAX_PENDING;
            self.commit(over, &mut out);
        }
        // Huge rows held for a redraw are given back too, the oldest first:
        // what they hold costs memory many times their bytes.
        let mut cells: usize = self.rows.iter().map(|r| r.cells.len()).sum();
        let mut over = 0;
        for r in &self.rows[..self.row.min(self.rows.len())] {
            if cells <= MAX_PENDING_CELLS {
                break;
            }
            cells -= r.cells.len();
            over += 1;
        }
        self.commit(over, &mut out);
        // The cursor's own row past the cap goes out as it is, and writing
        // goes on below it, as a terminal would have wrapped it long before.
        if self
            .rows
            .get(self.row)
            .is_some_and(|r| r.cells.len() > MAX_PENDING_CELLS)
        {
            let above = self.row;
            self.commit(above, &mut out);
            // A character the read cut in two goes on with the rest of it;
            // the fresh row counts only once text is written on it.
            let carry = if self.col == self.rows[0].cells.len() {
                incomplete_char(&self.rows[0].cells)
            } else {
                0
            };
            let at = self.rows[0].cells.len() - carry;
            let cells = self.rows[0].cells.split_off(at);
            let line = self.rows[0].line.clone();
            self.commit(1, &mut out);
            self.col = cells.len();
            self.rows.insert(
                0,
                Pending {
                    inserted: cells.is_empty(),
                    cells,
                    line,
                },
            );
            self.row = 0;
        }
        out
    }

    /// End of stream: every row left, but the rows with no text at the end
    /// (erased, or only a colour change) and a progress area Bazel drew and
    /// did not erase.
    pub fn finish(&mut self) -> Vec<Row> {
        let mut out = vec![];
        self.flush_status(&mut out);
        let mut rows = std::mem::take(&mut self.rows);
        while rows.last().is_some_and(|r| r.cells.is_empty()) {
            rows.pop();
        }
        // A progress area Bazel left at the end: its coloured first or status
        // line, classified as Bazel's, when every row under it is one curses
        // drew too (its actions, its BES wait).
        if let Some(top) = rows.iter().rposition(|p| self.is_area(p))
            && rows[top + 1..]
                .iter()
                .all(|r| r.line.as_ref().is_some_and(|l| l.drawn))
        {
            rows.truncate(top);
        }
        for p in rows {
            self.give(p, &mut out);
        }
        self.row = 0;
        self.col = 0;
        out
    }

    fn escape(&mut self, seg: &[u8]) {
        let Some(csi) = seg.strip_prefix(b"\x1b[") else {
            return;
        };
        let Some((&last, params)) = csi.split_last() else {
            return;
        };
        // A count past every row that may still change does no more than
        // one to the edge of them: no terminal draws more.
        let n = || {
            std::str::from_utf8(params)
                .ok()
                .and_then(|p| p.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, MAX_PENDING)
        };
        match last {
            b'A' => {
                let n = n();
                // Above every pending row: rows already given back, which
                // cannot change. What is written there goes on new rows, and
                // the rows below keep what they hold.
                if n > self.row {
                    let room = MAX_PENDING.saturating_sub(self.rows.len());
                    let extra = (n - self.row).min(room);
                    let inserted = Pending {
                        inserted: true,
                        ..Pending::default()
                    };
                    self.rows.splice(0..0, std::iter::repeat_n(inserted, extra));
                    self.row = 0;
                } else {
                    self.row -= n;
                }
                self.moved_up = true;
            }
            // Down stays on rows already written, as a terminal's cursor
            // stops at its last line.
            b'B' => self.row = (self.row + n()).min(self.rows.len().saturating_sub(1)),
            b'K' => {
                let col = self.col;
                if let Some(row) = self.rows.get_mut(self.row) {
                    // Erasing from the row's start erases it all, however it is
                    // spelled; for `status_every`, Bazel's status line goes too.
                    let keep = match params {
                        b"" | b"0" => col,
                        b"2" => 0,
                        _ => return,
                    };
                    let erased =
                        (keep == 0 && self.status_every.is_some() && !row.cells.is_empty())
                            .then(|| row.clone());
                    row.cells.truncate(keep);
                    if let Some(r) = erased
                        && is_status(&self.render(&r.cells))
                    {
                        self.erased_status = Some(r);
                    }
                }
            }
            b'm' => self.colour(params),
            _ => {}
        }
    }

    /// Change the colour state by the parameters of an SGR sequence. One
    /// with a private marker (`ESC[>4;2m`) is not colour, as a terminal
    /// takes it.
    fn colour(&mut self, params: &[u8]) {
        if params.first().is_some_and(|b| b"<=>?".contains(b)) {
            return;
        }
        self.style.apply(params);
        self.style_id = match self.style_ids.get(&self.style) {
            Some(&id) => id,
            None if self.styles.len() < MAX_STYLES => {
                let id = self.styles.len() as u32;
                self.styles.push(self.style);
                self.style_ids.insert(self.style, id);
                id
            }
            None => 0,
        };
    }

    /// Write `text` at the cursor, over what the row holds there.
    fn text(&mut self, text: &[u8], line: Option<&Line>, out: &mut Vec<Row>) {
        if self.rows.len() <= self.row {
            self.rows.resize(self.row + 1, Pending::default());
        }
        if text.is_empty() {
            // A blank row is classified too, so early settling can pass it.
            if let Some(row) = self.rows.get_mut(self.row)
                && row.line.is_none()
            {
                row.line = line.cloned();
            }
            return;
        }
        if self.moved_up {
            self.moved_up = false;
            let above = self.row;
            self.commit(above, out);
            self.flush_status(out);
        }
        let style = self.style_id;
        let col = self.col;
        let row = &mut self.rows[self.row];
        row.inserted = false;
        if row.cells.len() < col {
            row.cells.resize(col, blank());
        }
        let mut at = col;
        for cell in cells(text, style) {
            if at < row.cells.len() {
                row.cells[at] = cell;
            } else {
                row.cells.push(cell);
            }
            at += 1;
        }
        if line.is_some() {
            row.line = line.cloned();
        }
        self.col = at;
    }

    /// Give back the first `n` pending rows: they can no longer change.
    fn commit(&mut self, n: usize, out: &mut Vec<Row>) {
        let n = n.min(self.rows.len());
        let rows: Vec<Pending> = self.rows.drain(..n).collect();
        for p in rows {
            self.give(p, out);
        }
        self.row -= n.min(self.row);
    }

    /// Give back a row that can no longer change, but one only inserted for
    /// a move up and never written.
    fn give(&mut self, p: Pending, out: &mut Vec<Row>) {
        if p.inserted && p.cells.is_empty() {
            return;
        }
        out.push(Row {
            bytes: self.render(&p.cells),
            line: p.line,
            status: false,
        });
    }

    /// `cells` as bytes: each character after the colour changes that put
    /// it in its colour, reset at the end if one is left on.
    fn render(&self, cells: &[Cell]) -> Vec<u8> {
        let mut out = Vec::with_capacity(cells.len());
        let mut current = 0;
        for c in cells {
            if c.style != current {
                if current != 0 {
                    out.extend_from_slice(b"\x1b[0m");
                }
                if c.style != 0 {
                    out.extend_from_slice(&self.styles[c.style as usize].sgr());
                }
                current = c.style;
            }
            out.extend_from_slice(&c.ch[..c.len as usize]);
        }
        if current != 0 {
            out.extend_from_slice(b"\x1b[0m");
        }
        out
    }

    fn flush_status(&mut self, out: &mut Vec<Row>) {
        let Some(status) = self.erased_status.take() else {
            return;
        };
        let Some(every) = self.status_every else {
            return;
        };
        if self.last_status.elapsed() >= every {
            self.last_status = Instant::now();
            out.push(Row {
                bytes: self.render(&status.cells),
                line: status.line,
                status: true,
            });
        }
    }

    /// Whether `p` is the first line of a progress area Bazel drew: its
    /// colouring, and classified as Bazel's rather than an action's output,
    /// which a tool may colour the same way.
    fn is_area(&self, p: &Pending) -> bool {
        p.line.as_ref().is_some_and(|l| {
            let bytes = self.render(&p.cells);
            (line::is_bazel_progress(&bytes) && l.kind.is_progress())
                || (line::is_bazel_status(&bytes) && l.drawn)
        })
    }
}

/// How many cells at the end of `cells` are the start of a UTF-8
/// character whose remaining bytes have not come yet.
fn incomplete_char(cells: &[Cell]) -> usize {
    let lone = |c: &Cell| c.len == 1 && c.ch[0] >= 0x80;
    let tail = cells.iter().rev().take(3).take_while(|c| lone(c)).count();
    for k in (1..=tail).rev() {
        let need = match cells[cells.len() - k].ch[0] {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => continue,
        };
        let rest = &cells[cells.len() - k + 1..];
        if k < need && rest.iter().all(|c| c.ch[0] & 0xc0 == 0x80) {
            return k;
        }
    }
    0
}

fn blank() -> Cell {
    Cell {
        ch: [b' ', 0, 0, 0],
        len: 1,
        style: 0,
    }
}

/// `text` as cells in `style`: one per character, and one per byte that is
/// not valid UTF-8.
fn cells(text: &[u8], style: u32) -> impl Iterator<Item = Cell> + '_ {
    text.utf8_chunks().flat_map(move |chunk| {
        let valid = chunk.valid().chars().map(move |c| {
            let mut ch = [0; 4];
            let len = c.encode_utf8(&mut ch).len() as u8;
            Cell { ch, len, style }
        });
        let invalid = chunk.invalid().iter().map(move |&b| Cell {
            ch: [b, 0, 0, 0],
            len: 1,
            style,
        });
        valid.chain(invalid)
    })
}

/// Whether `row` is the first line of Bazel's progress area.
fn is_status(row: &[u8]) -> bool {
    line::is_progress_text(&ansi::text(row))
}

/// `rows` as the lines a destination is given.
pub fn joined(rows: &[Row]) -> Vec<u8> {
    let mut out = vec![];
    for r in rows {
        out.extend_from_slice(&r.bytes);
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(stream: &[u8], every: Option<Duration>) -> String {
        let mut s = Screen::new(every);
        let mut rows = s.feed(stream, None);
        rows.extend(s.finish());
        String::from_utf8(ansi::strip(&joined(&rows)).into_owned()).unwrap()
    }

    /// Curses' cycle: a message, the progress area under it, erased before
    /// the next message, and a last area the end of the build leaves.
    const CYCLE: &[u8] = b"INFO: Analyzed 3 targets\n\
        \x1b[32m[1 / 3]\x1b[0m 2 actions running\n    Compiling a.cc; 1s\n    Compiling b.cc; 1s\n\
        \r\x1b[1A\x1b[K\r\x1b[1A\x1b[K\r\x1b[1A\x1b[K\x1b[31mERROR: \x1b[0ma.cc failed\n\
        a.cc:3: error: nope\n\
        \x1b[32m[2 / 3]\x1b[0m 1 action running\n    Compiling b.cc; 2s\n\
        \r\x1b[1A\x1b[K\r\x1b[1A\x1b[KINFO: Build completed\n\
        \x1b[32m[3 / 3]\x1b[0m no actions running\n";

    #[test]
    fn erased_rows_are_gone_and_messages_stay() {
        assert_eq!(
            render(CYCLE, None),
            "INFO: Analyzed 3 targets\nERROR: a.cc failed\na.cc:3: error: nope\nINFO: Build completed\n\
             [3 / 3] no actions running\n",
            "unclassified, a last area stays as a terminal shows it"
        );
    }

    #[test]
    fn a_status_line_is_kept_at_most_so_often() {
        assert_eq!(
            render(CYCLE, Some(Duration::ZERO)),
            "INFO: Analyzed 3 targets\n[1 / 3] 2 actions running\nERROR: a.cc failed\n\
             a.cc:3: error: nope\n[2 / 3] 1 action running\nINFO: Build completed\n\
             [3 / 3] no actions running\n"
        );
        assert_eq!(
            render(CYCLE, Some(Duration::from_secs(3600))),
            "INFO: Analyzed 3 targets\nERROR: a.cc failed\na.cc:3: error: nope\nINFO: Build completed\n\
             [3 / 3] no actions running\n",
            "none before the build has run that long"
        );
    }

    /// A tool's line that looks like progress stays when nothing erases it,
    /// and a tool's own `\r` updates leave their last state.
    #[test]
    fn only_what_the_stream_erases_is_dropped() {
        assert_eq!(
            render(b"[1 / 2] a tool's own count\nok\n", None),
            "[1 / 2] a tool's own count\nok\n"
        );

        assert_eq!(render(b"10%\r50%\r100%\ndone\n", None), "100%\ndone\n");
    }

    /// Erasing a row is the same whether the stream says so with `ESC[K`
    /// from its start or `ESC[2K`; `ESC[1K` is not modelled and leaves it.
    #[test]
    fn a_row_erased_either_way_keeps_its_status_line() {
        let two_k = b"INFO: Analyzed 3 targets\n\x1b[32m[1 / 3]\x1b[0m 2 actions running\n\
            \x1b[1A\x1b[2KINFO: Build completed\n";
        assert_eq!(
            render(two_k, Some(Duration::ZERO)),
            "INFO: Analyzed 3 targets\n[1 / 3] 2 actions running\nINFO: Build completed\n"
        );
        assert_eq!(render(b"ab\x1b[1Kc\n", None), "abc\n");
    }

    /// The rows it may still change are bounded: past [`MAX_PENDING`] the
    /// oldest are given back, and a move up past every row writes on new
    /// ones without making blank rows of the rest.
    #[test]
    fn pending_rows_stay_bounded() {
        let mut s = Screen::new(None);
        let lines: Vec<u8> = (0..MAX_PENDING + 10)
            .flat_map(|i| format!("l{i}\n").into_bytes())
            .collect();
        let given = s.feed(&lines, None);
        assert_eq!(
            given.len(),
            11,
            "those past the limit, the cursor's empty row after the last counted"
        );
        assert_eq!(String::from_utf8_lossy(&given[0].bytes), "l0");
        let out = render(b"a\n\x1b[5Ab\n", None);
        assert!(out.contains('a') && out.contains('b'), "{out:?}");
        assert!(!out.lines().any(str::is_empty), "{out:?}");
    }

    /// Past [`MAX_STYLES`] distinct colour states, text in a new one comes
    /// out uncoloured rather than in another's colour.
    #[test]
    fn a_colour_past_the_limit_comes_out_plain() {
        let mut stream = vec![];
        for i in 0..MAX_STYLES {
            stream.extend(
                format!(
                    "\x1b[38;2;{};{};{}mx\x1b[0m",
                    i % 256,
                    (i / 256) % 256,
                    i / 65536
                )
                .bytes(),
            );
        }
        stream.extend(b"\n\x1b[38;2;1;2;255mLAST\x1b[0m\n");
        let mut s = Screen::new(None);
        let mut rows = s.feed(&stream, None);
        rows.extend(s.finish());
        assert_eq!(rows.last().unwrap().bytes, b"LAST");
    }

    #[test]
    fn colour_sequences_set_a_state_whatever_their_spelling() {
        let style = |seqs: &[&str]| {
            let mut s = Style::default();
            for p in seqs {
                s.apply(p.as_bytes());
            }
            s
        };
        assert_eq!(style(&["31", "00"]), Style::default(), "`ls`'s reset");
        assert_eq!(
            style(&["1;31", "39;49;00"]),
            Style::default(),
            "a highlighter's reset"
        );
        assert_eq!(
            style(&["1", "31", "39"]).sgr(),
            b"\x1b[1m",
            "a partial reset keeps bold"
        );
        assert_eq!(style(&["38;5;244"]).sgr(), b"\x1b[38;5;244m");
        assert_eq!(style(&["48;2;1;2;3", "4"]).sgr(), b"\x1b[4;48;2;1;2;3m");
        assert_eq!(style(&["31", "1"]).sgr(), b"\x1b[1;31m", "Bazel's bold red");
        assert_eq!(
            style(&["5", "6", "25"]),
            Style::default(),
            "blink off ends either blink"
        );
        assert_eq!(style(&["6", "26"]).sgr(), b"\x1b[6m", "26 is not a blink");
    }

    #[test]
    fn colour_sequences_are_read_as_xterm_reads_them() {
        let style = |p: &str| {
            let mut s = Style::default();
            s.apply(p.as_bytes());
            s.sgr()
        };
        assert_eq!(
            style("38:2::255:0:0"),
            b"\x1b[38;2;255;0;0m",
            "colon form with a colour space"
        );
        assert_eq!(
            style("38:2:255:128:1"),
            b"\x1b[38;2;255;128;1m",
            "colon form without one"
        );
        assert_eq!(style("38:5:244"), b"\x1b[38;5;244m");
        assert_eq!(
            style("4:3"),
            b"\x1b[4m",
            "a curly underline is an underline"
        );
        assert_eq!(style("4;4:0"), b"\x1b[m", "`4:0` is none");
        assert_eq!(
            style("31;58;5;1"),
            b"\x1b[31m",
            "underline colour takes its arguments"
        );
        assert_eq!(style("31;58:2::1:2:0"), b"\x1b[31m");
        assert_eq!(style("38;5;300"), b"\x1b[m", "out of range is ignored");
        assert_eq!(
            style("31;70000"),
            b"\x1b[31m",
            "a huge parameter is ignored"
        );
        assert_eq!(style("53"), b"\x1b[53m", "overline kept");
        assert_eq!(style("101"), b"\x1b[101m");
        let mut s = Screen::new(None);
        let mut rows = s.feed(b"\x1b[31m\x1b[>4;2mred\n", None);
        rows.extend(s.finish());
        assert_eq!(
            rows[0].bytes, b"\x1b[31mred\x1b[0m",
            "a private sequence is not colour"
        );
    }

    /// A split line adds no blank row when its end comes in the next read,
    /// and a character the read cut in two stays whole.
    #[test]
    fn a_split_line_ends_cleanly() {
        let mut s = Screen::new(None);
        let mut rows = s.feed(&vec![b'x'; MAX_PENDING_CELLS + 1], None);
        rows.extend(s.feed(b"\nnext\n", None));
        rows.extend(s.finish());
        let lens: Vec<usize> = rows.iter().map(|r| r.bytes.len()).collect();
        assert_eq!(lens, [MAX_PENDING_CELLS + 1, 4]);

        let mut s = Screen::new(None);
        let mut input = "中".repeat(MAX_PENDING_CELLS).into_bytes();
        input.push(0xe4);
        let mut rows = s.feed(&input, None);
        rows.extend(s.feed(b"\xb8\xad\n", None));
        rows.extend(s.finish());
        for r in &rows {
            assert!(std::str::from_utf8(&r.bytes).is_ok());
        }
        assert_eq!(rows.last().unwrap().bytes, "中".as_bytes());
    }

    /// One endless line is given back in pieces, not held whole.
    #[test]
    fn an_endless_line_is_given_back_in_pieces() {
        let mut s = Screen::new(None);
        let chunk = vec![b'x'; 1 << 16];
        let mut given = 0;
        for _ in 0..64 {
            given += s
                .feed(&chunk, None)
                .iter()
                .map(|r| r.bytes.len())
                .sum::<usize>();
        }
        given += s.finish().iter().map(|r| r.bytes.len()).sum::<usize>();
        assert_eq!(given, 64 << 16);
        assert!(s.rows.is_empty());
    }

    /// Many colours, and many changes on one row, keep each character's own.
    #[test]
    fn colours_stay_right_however_many() {
        let mut input = vec![];
        for i in 0..1100 {
            input.extend(format!("\x1b[38;2;{};{};7mq\x1b[0m\n", i % 256, i / 256).bytes());
        }
        input.extend(b"\x1b[32mGREEN\x1b[0m\n");
        let out = render(&input, None);
        let mut s = Screen::new(None);
        let mut rows = s.feed(&input, None);
        rows.extend(s.finish());
        assert_eq!(rows.last().unwrap().bytes, b"\x1b[32mGREEN\x1b[0m", "{out}");

        let mut row = b"\x1b[1m".to_vec();
        for i in 0..40 {
            row.extend(format!("\x1b[31mx{i}\x1b[39my").bytes());
        }
        row.extend(b"\x1b[0m\n");
        let mut s = Screen::new(None);
        let mut rows = s.feed(&row, None);
        rows.extend(s.finish());
        let bytes = String::from_utf8(rows[0].bytes.clone()).unwrap();
        assert!(
            bytes.contains("\x1b[1;31mx39\x1b[0m\x1b[1my"),
            "bold kept to the end: {bytes}"
        );
    }

    /// Lines come out as soon as the next erase settles them, not at the end.
    #[test]
    fn a_message_is_written_once_the_next_redraw_settles_it() {
        let mut s = Screen::new(None);
        let out = s.feed(b"WARNING: first\n\x1b[32m[1 / 2]\x1b[0m running\n", None);
        assert!(out.is_empty(), "the area may still be erased");
        let out = s.feed(
            b"\r\x1b[1A\x1b[K\x1b[32m[1 / 2]\x1b[0m still running\n",
            None,
        );
        assert_eq!(String::from_utf8(joined(&out)).unwrap(), "WARNING: first\n");
    }
}
