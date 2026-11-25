//! Console writes that survive a closed pipe.
//!
//! `println!` / `eprintln!` panic when the write fails, and a closed reader is
//! the common case: `aspect build … | head`, or a CI assertion piping into
//! `grep -q`, closes the pipe as soon as it has what it wants. The panic aborts
//! the process mid-task, so nothing runs the task's terminal update and its
//! GitHub check run is left "running" until the API sweeper finalizes it as
//! DISCONNECTED.
//!
//! These macros discard the write error instead. Output past the close is lost
//! either way — nobody is reading it — but the process finishes its work,
//! reports its real result, and exits with its real code.
//!
//! For human-facing console output only. Anything a *program* consumes — a
//! credential-helper response, a machine-readable dump — must propagate the
//! failure rather than report success on output nobody received.
//!
//! AXL `print()` goes through [`TolerantPrintHandler`], which on a person's
//! terminal ([`terminal_links`]) also writes a line's URLs and paths to files
//! that exist as links, as Bazel's console does.
//!
//! What these macros, `print()`, AXL's `ctx.std.io.stdout` and `stderr`, and
//! Bazel's console write is also copied to the task history's `output.log`
//! and `output.ansi` ([`keep_output`]).

/// `println!` that ignores a failed write (notably a closed pipe), and copies
/// the line to the task's output ([`copy_output`]).
#[macro_export]
macro_rules! outln {
    () => {{
        $crate::out!("\n");
    }};
    ($($arg:tt)*) => {{
        $crate::out!("{}\n", format_args!($($arg)*));
    }};
}

/// `print!` that ignores a failed write (notably a closed pipe), and copies
/// the text to the task's output ([`copy_output`]).
#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let text = format!($($arg)*);
        let _ = std::io::stdout().write_all(text.as_bytes());
        $crate::out::copy_output(text.as_bytes());
    }};
}

/// `eprintln!` that ignores a failed write (notably a closed pipe), and
/// copies the line to the task's output ([`copy_output`]).
#[macro_export]
macro_rules! errln {
    () => {{
        $crate::errln!("");
    }};
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let text = format!("{}\n", format_args!($($arg)*));
        let _ = std::io::stderr().write_all(text.as_bytes());
        $crate::out::copy_output(text.as_bytes());
    }};
}

/// The most each of a task's output files keeps.
const MAX_OUTPUT_LOG: u64 = 64 * 1024 * 1024;

/// The most of an unended line the output files hold back for the rest of it.
const MAX_PENDING: usize = 64 * 1024;

/// The most output kept from before a task's files open ([`keep_output`]).
const MAX_EARLY: usize = 64 * 1024;

/// The file a task's history keeps its output in as plain text.
pub const OUTPUT_LOG: &str = "output.log";

/// The file a task's history keeps its output in with its colour.
pub const OUTPUT_ANSI: &str = "output.ansi";

/// Copy what AXL's `ctx.std.io.stdout` and `stderr` write as this process's
/// own output is copied ([`copy_output`]): before a task's files open, to
/// what they get first. Called once at start-up.
pub fn copy_axl_output() {
    axl_types::stream::copy_output_to(copy_output);
}

/// One of the task's output files.
struct LogFile {
    name: &'static str,
    path: std::path::PathBuf,
    file: std::fs::File,
    written: u64,
    /// Whether it has stopped at the cap.
    full: bool,
}

impl LogFile {
    fn open(dir: &std::path::Path, name: &'static str) -> std::io::Result<LogFile> {
        let path = dir.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&path)?;
        Ok(LogFile {
            name,
            path,
            file,
            written: 0,
            full: false,
        })
    }

    /// Write `text`, or as much of it as fits under `cap`, cut at a line's
    /// end (else where no escape or character is split), then a note.
    fn write(&mut self, text: &[u8], cap: u64) {
        use std::io::Write as _;
        if self.full || text.is_empty() {
            return;
        }
        let room = cap.saturating_sub(self.written);
        if (text.len() as u64) < room {
            self.written += text.len() as u64;
            let _ = self.file.write_all(text);
            return;
        }
        let fits = &text[..room as usize];
        let cut = match fits.iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => safe_cut(fits, fits.len()),
        };
        self.written += cut as u64;
        self.full = true;
        let _ = self.file.write_all(&text[..cut]);
        let note = format!("\n... {} stops here: {} MiB\n", self.name, cap >> 20);
        let _ = self.file.write_all(note.as_bytes());
    }

    /// The newest `max` bytes of the file up to `end`, then `tail`, from
    /// the start of a line when the cut leaves one, else where no escape or
    /// character is split.
    fn newest(&self, end: u64, tail: &[u8], max: u64) -> Vec<u8> {
        use std::io::{Read as _, Seek as _};
        let want = (end + tail.len() as u64).saturating_sub(max);
        // A little before the cut, to tell whether it is at a line's start
        // or inside an escape.
        let from = want.saturating_sub(32).min(end);
        let mut text = vec![];
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return vec![];
        };
        if file.seek(std::io::SeekFrom::Start(from)).is_err()
            || file.take(end - from).read_to_end(&mut text).is_err()
        {
            return vec![];
        }
        text.extend_from_slice(tail);
        let at = (want - from) as usize;
        let start = if at == 0 || text[at - 1] == b'\n' {
            at
        } else if let Some(i) = text[at..].iter().position(|&b| b == b'\n') {
            at + i + 1
        } else {
            safe_start(&text, at)
        };
        text.split_off(start)
    }
}

/// The largest index at most `at` that splits no escape sequence and no
/// UTF-8 character of `b`, one left unfinished at its end included.
fn safe_cut(b: &[u8], at: usize) -> usize {
    let mut at = at.min(b.len());
    let mut i = 0;
    for (is_escape, seg) in bazel::console::ansi::segments(b) {
        let end = i + seg.len();
        if end > at || (end == at && is_escape && !escape_ended(seg)) {
            if is_escape {
                at = i;
            }
            break;
        }
        i = end;
    }
    for lead in at.saturating_sub(3)..at {
        let len = match b[lead] {
            0xF0.. => 4,
            0xE0.. => 3,
            0xC0.. => 2,
            _ => continue,
        };
        if lead + len > at {
            return lead;
        }
    }
    at
}

/// Whether `seg`, an escape sequence, has its end, rather than the bytes
/// that finish it still to come.
fn escape_ended(seg: &[u8]) -> bool {
    match seg.get(1) {
        None => false,
        Some(b'[') => seg.len() > 2 && seg.last().is_some_and(|b| (0x40..=0x7e).contains(b)),
        Some(b']' | b'P' | b'X' | b'^' | b'_') => {
            seg.ends_with(b"\x07") || (seg.len() > 3 && seg.ends_with(b"\x1b\\"))
        }
        Some(_) => true,
    }
}

/// The smallest index at least `at` where `b` starts no further inside an
/// escape sequence or a UTF-8 character.
fn safe_start(b: &[u8], at: usize) -> usize {
    let mut i = 0;
    let mut at = at;
    for (is_escape, seg) in bazel::console::ansi::segments(b) {
        if i + seg.len() > at {
            if is_escape && i < at {
                at = i + seg.len();
            }
            break;
        }
        i += seg.len();
    }
    while at < b.len() && b[at] & 0xC0 == 0x80 {
        at += 1;
    }
    at
}

/// One writer's lines on their way to the files: this process's own
/// writes, or Bazel's console's.
#[derive(Default, Clone)]
struct Stream {
    /// Its line written so far without its end, kept until it ends so a
    /// carriage return, an escape or a secret split across writes is
    /// handled whole.
    pending: Vec<u8>,
    /// Its colour as its last line left it.
    style: Style,
}

impl Stream {
    /// What of `pending` can be written: its whole lines, or all of it with
    /// `all`, or, past [`MAX_PENDING`], all but its last `keep` bytes,
    /// a secret, an escape or a character it would split held back too.
    fn take(&mut self, secrets: &[String], all: bool) -> Option<Vec<u8>> {
        let end = match self.pending.iter().rposition(|&b| b == b'\n') {
            _ if all => self.pending.len(),
            Some(i) => i + 1,
            None if self.pending.len() > MAX_PENDING => {
                let keep = secrets.iter().map(|s| s.len()).max().unwrap_or(1) - 1;
                let mut at = safe_cut(&self.pending, self.pending.len() - keep);
                for secret in secrets.iter().map(|s| s.as_bytes()) {
                    let window = at.saturating_sub(secret.len() - 1);
                    let near =
                        &self.pending[window..(at + secret.len() - 1).min(self.pending.len())];
                    if let Some(i) = near.windows(secret.len()).position(|w| w == secret)
                        && window + i < at
                    {
                        at = at.min(window + i);
                    }
                }
                if at == 0 { self.pending.len() } else { at }
            }
            None => return None,
        };
        (end > 0).then(|| self.pending.drain(..end).collect())
    }
}

/// Where Bazel's console began in each file, and what of the task's own
/// unended line was held back then: for [`output_before_console`].
struct ConsoleStart {
    plain: u64,
    ansi: u64,
    task: Stream,
    emitted: Style,
}

/// Where this process copies what it writes to its stdout and stderr: the
/// task history's `output.log` and `output.ansi`; see [`keep_output`].
struct OutputLog {
    /// Which [`keep_output`] this is, for [`flush_output`].
    generation: u64,
    plain: LogFile,
    ansi: LogFile,
    /// The most each file keeps.
    cap: u64,
    secrets: Vec<String>,
    /// This process's own writes.
    task: Stream,
    /// Bazel's console's lines.
    console: Stream,
    /// The colour `output.ansi` leaves set.
    emitted: Style,
    console_start: Option<ConsoleStart>,
}

/// Where this process's output is copied: the task's files, or before
/// they open, what they get first.
struct Copies {
    log: Option<OutputLog>,
    early: Early,
}

impl Default for Copies {
    fn default() -> Self {
        Copies::new()
    }
}

impl Copies {
    const fn new() -> Self {
        Copies {
            log: None,
            early: Early(vec![]),
        }
    }

    /// Copy to `log` from now on, what was written before first.
    fn keep(&mut self, mut log: OutputLog) {
        log.push(&self.early.take());
        if let Some(mut earlier) = self.log.replace(log) {
            earlier.flush();
        }
    }

    fn begin_early(&mut self) {
        if let Some(mut log) = self.log.take() {
            log.flush();
        }
    }

    fn copy(&mut self, bytes: &[u8], console: bool) {
        match self.log.as_mut() {
            Some(log) if console => log.push_console(bytes),
            Some(log) => log.push(bytes),
            None => self.early.push(bytes),
        }
    }
}

static OUTPUT: std::sync::Mutex<Copies> = std::sync::Mutex::new(Copies::new());

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The newest [`MAX_EARLY`] bytes written before a task's files open: the
/// banner and the task's header, printed before its history begins.
struct Early(Vec<u8>);

impl Early {
    fn push(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
        if self.0.len() > MAX_EARLY {
            let over = self.0.len() - MAX_EARLY;
            self.0.drain(..over);
            // From a line's start, so no escape the cut split shows as text.
            if let Some(i) = self.0.iter().position(|&b| b == b'\n') {
                self.0.drain(..=i);
            }
        }
    }

    fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

/// Copy what this process writes to its stdout and stderr from now on to
/// `dir` (the task history's entry): its own lines, `print()`'s, AXL's
/// `ctx.std.io.stdout` and `stderr`, and Bazel's console as the console
/// forwards it, but not what a child writes to a stream it inherited (a
/// build without a console processor writes to the terminal itself). What
/// was written before (the banner, the task's header; see [`begin_early`])
/// comes first, its newest [`MAX_EARLY`] bytes. The files are kept until
/// the next task's [`begin_early`], so what is printed after the task ends
/// is in them too. Returns which this is, for [`flush_output`].
///
/// Both files keep each line as it settled, column by column as a
/// terminal shows it: a carriage return or a backspace moves back along the
/// line, what is written then overwrites what was there (a wide character
/// two columns, a combining mark joining the one before), a tab spans to
/// the next stop of 8 and is kept as a tab unless written over in part
/// (then spaces), the cursor moves along the line (`ESC [ C`, `D`, `G`, no
/// further than [`MAX_COLUMN`]), and an erase in line (`ESC [ K`) erases.
/// A line nothing moves back over is written as it is, cell by cell only
/// otherwise. Lines a
/// cursor movement up or an erase of the screen removed are kept: this is
/// no terminal. An unended line past [`MAX_PENDING`] is written in parts, a
/// carriage return in one overwriting only that part, and a secret a colour
/// change splits right at the cut between parts can be written unredacted. `secrets` (the task's secret flag values, those
/// [`task_record::scrubs`](crate::engine::task_record::scrubs)) are
/// redacted, and each file stops at [`MAX_OUTPUT_LOG`]. [`OUTPUT_LOG`] is
/// plain text, every escape and control character (C1 included) but tab
/// and newline dropped. [`OUTPUT_ANSI`] is the same text with its colour
/// (SGR, `ESC [ ... m`). A secret is found in the plain text, so one split
/// by a colour change is redacted too, `<REDACTED>` in the colour of its
/// first character.
pub fn keep_output(dir: &std::path::Path, secrets: Vec<String>) -> std::io::Result<u64> {
    let log = OutputLog::open(dir, secrets)?;
    let generation = log.generation;
    if let Ok(mut copies) = OUTPUT.lock() {
        copies.keep(log);
    }
    copy_axl_output();
    Ok(generation)
}

/// Stop copying to the last task's files: what is written from now on is
/// the next task's ([`keep_output`]), its header first.
pub fn begin_early() {
    if let Ok(mut copies) = OUTPUT.lock() {
        copies.begin_early();
    }
}

/// Copy `bytes`, as written to stdout or stderr, to the task's output
/// files, a line at a time (an unended one held back, up to
/// [`MAX_PENDING`], until it ends or [`flush_output`]); before
/// [`keep_output`], to what it writes first.
pub fn copy_output(bytes: &[u8]) {
    if let Ok(mut copies) = OUTPUT.lock() {
        copies.copy(bytes, false);
    }
}

/// [`copy_output`] for Bazel's console as the console forwards it, whole
/// lines: kept apart from the task's own unended line, and where the first
/// of it begins is kept, for [`output_before_console`].
pub fn copy_console_output(bytes: &[u8]) {
    if let Ok(mut copies) = OUTPUT.lock() {
        copies.copy(bytes, true);
    }
}

/// What the task wrote before Bazel's console began (all it has written,
/// when no console has), its unended line included, as `output.log` keeps
/// it, its newest `max` bytes from a line's start: for a screen taken once
/// the console is under way to show the task's output from its start
/// without Bazel's lines twice.
pub fn output_before_console(max: u64) -> Vec<u8> {
    before_console(max, false)
}

/// [`output_before_console`] as `output.ansi` keeps it, with its colour.
pub fn output_before_console_ansi(max: u64) -> Vec<u8> {
    before_console(max, true)
}

fn before_console(max: u64, colour: bool) -> Vec<u8> {
    let Ok(copies) = OUTPUT.lock() else {
        return vec![];
    };
    copies
        .log
        .as_ref()
        .map_or_else(Vec::new, |log| log.before_console(max, colour))
}

/// Write what the output files hold back of an unended line: the task that
/// [`keep_output`] returned `generation` for is over. A later task's files
/// are left alone.
pub fn flush_output(generation: u64) {
    let Ok(mut copies) = OUTPUT.lock() else {
        return;
    };
    if let Some(log) = copies
        .log
        .as_mut()
        .filter(|log| log.generation == generation)
    {
        log.flush();
    }
}

impl OutputLog {
    fn open(dir: &std::path::Path, secrets: Vec<String>) -> std::io::Result<OutputLog> {
        Ok(OutputLog {
            generation: GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            plain: LogFile::open(dir, OUTPUT_LOG)?,
            ansi: LogFile::open(dir, OUTPUT_ANSI)?,
            cap: MAX_OUTPUT_LOG,
            secrets: secrets
                .into_iter()
                .filter(|s| crate::engine::task_record::scrubs(s))
                .collect(),
            task: Stream::default(),
            console: Stream::default(),
            emitted: Style::default(),
            console_start: None,
        })
    }

    /// The task's own `bytes` as written: whole lines kept, an unended one
    /// held back.
    fn push(&mut self, bytes: &[u8]) {
        if !self.full() {
            self.task.pending.extend_from_slice(bytes);
            self.drain(false, false);
        }
    }

    /// [`copy_console_output`]'s copy: where the console begins marked
    /// first, past what the task has written, with its unended line.
    fn push_console(&mut self, bytes: &[u8]) {
        if self.console_start.is_none() {
            self.console_start = Some(ConsoleStart {
                plain: self.plain.written,
                ansi: self.ansi.written,
                task: self.task.clone(),
                emitted: self.emitted,
            });
        }
        if !self.full() {
            self.console.pending.extend_from_slice(bytes);
            self.drain(true, false);
        }
    }

    fn full(&self) -> bool {
        self.plain.full && self.ansi.full
    }

    /// What is held back of an unended line, kept.
    fn flush(&mut self) {
        self.drain(false, true);
        self.drain(true, true);
    }

    /// Write what the console's or the task's stream can give
    /// ([`Stream::take`]) to both files, as [`render`] has it.
    fn drain(&mut self, console: bool, all: bool) {
        let stream = if console {
            &mut self.console
        } else {
            &mut self.task
        };
        let Some(lines) = stream.take(&self.secrets, all) else {
            return;
        };
        let (plain, ansi) = render(&lines, &mut stream.style, &mut self.emitted, &self.secrets);
        self.plain.write(&plain, self.cap);
        self.ansi.write(&ansi, self.cap);
    }

    /// [`output_before_console`]'s bytes, or with `colour`
    /// [`output_before_console_ansi`]'s.
    fn before_console(&self, max: u64, colour: bool) -> Vec<u8> {
        let (plain_end, ansi_end, mut task, mut emitted) = match &self.console_start {
            Some(start) => (start.plain, start.ansi, start.task.clone(), start.emitted),
            None => (
                self.plain.written,
                self.ansi.written,
                self.task.clone(),
                self.emitted,
            ),
        };
        let (plain, ansi) = render(&task.pending, &mut task.style, &mut emitted, &self.secrets);
        match colour {
            false => self.plain.newest(plain_end, &plain, max),
            true => self.ansi.newest(ansi_end, &ansi, max),
        }
    }
}

/// `bytes` as [`OUTPUT_LOG`] and [`OUTPUT_ANSI`] keep them (see
/// [`keep_output`]): the same visible text, the second with its colour.
/// `style` is the writer's colour, carried from line to line, and
/// `emitted` the colour `output.ansi` was left in.
fn render(
    bytes: &[u8],
    style: &mut Style,
    emitted: &mut Style,
    secrets: &[String],
) -> (Vec<u8>, Vec<u8>) {
    let text = String::from_utf8_lossy(bytes);
    let mut plain = Vec::with_capacity(bytes.len());
    let mut ansi = Vec::with_capacity(bytes.len());
    for line in text.split_inclusive('\n') {
        let (body, end) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        Line::of(body, style).write(&mut plain, &mut ansi, emitted, secrets);
        plain.extend_from_slice(end.as_bytes());
        ansi.extend_from_slice(end.as_bytes());
    }
    (plain, ansi)
}

/// Colour and text attributes as SGR sequences leave them: each attribute
/// set or cleared on its own (`22`, `39`, ...), so a writer that never
/// resets whole keeps a state of the same size.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Style {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    hidden: bool,
    strike: bool,
    fg: Colour,
    bg: Colour,
}

/// A foreground or background colour.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Colour {
    #[default]
    Default,
    /// One of the 16, by its own code (`31`, `92`, `44`).
    Code(u8),
    /// One of the 256 (`38;5;n`).
    Index(u8),
    Rgb(u8, u8, u8),
}

impl Colour {
    /// The colour `38`/`48` (`base`) gives with `parts`: `5;n` or `2;r;g;b`,
    /// `;`- or `:`-separated (the latter with an optional colour space).
    fn extended(parts: &[&str]) -> Colour {
        let num = |s: &&str| s.parse::<u8>().ok();
        match parts.first().copied() {
            Some("5") => parts
                .get(1)
                .and_then(num)
                .map_or(Colour::Default, Colour::Index),
            Some("2") => {
                let rgb: Vec<u8> = parts[parts.len().saturating_sub(3).max(1)..]
                    .iter()
                    .filter_map(num)
                    .collect();
                match rgb[..] {
                    [r, g, b] => Colour::Rgb(r, g, b),
                    _ => Colour::Default,
                }
            }
            _ => Colour::Default,
        }
    }

    /// Its parameters, `base` (38 or 48) for an extended one.
    fn push_params(self, base: u8, out: &mut String) {
        use std::fmt::Write as _;
        if !out.is_empty() && self != Colour::Default {
            out.push(';');
        }
        let _ = match self {
            Colour::Default => Ok(()),
            Colour::Code(code) => write!(out, "{code}"),
            Colour::Index(n) => write!(out, "{base};5;{n}"),
            Colour::Rgb(r, g, b) => write!(out, "{base};2;{r};{g};{b}"),
        };
    }
}

impl Style {
    /// Apply the parameters of one SGR sequence, `ESC [ params m`.
    fn apply(&mut self, params: &str) {
        if params.is_empty() {
            *self = Style::default();
            return;
        }
        let mut params = params.split(';').peekable();
        while let Some(p) = params.next() {
            // `38:5:208`: one parameter, its parts after colons.
            let mut parts = p.split(':');
            let code: u32 = match parts.next().unwrap_or("").parse() {
                Ok(code) => code,
                Err(_) if p.is_empty() => 0,
                Err(_) => continue,
            };
            match code {
                0 => *self = Style::default(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 | 21 => self.underline = true,
                5 | 6 => self.blink = true,
                7 => self.inverse = true,
                8 => self.hidden = true,
                9 => self.strike = true,
                22 => (self.bold, self.dim) = (false, false),
                23 => self.italic = false,
                24 => self.underline = false,
                25 => self.blink = false,
                27 => self.inverse = false,
                28 => self.hidden = false,
                29 => self.strike = false,
                30..=37 | 90..=97 => self.fg = Colour::Code(code as u8),
                39 => self.fg = Colour::Default,
                40..=47 | 100..=107 => self.bg = Colour::Code(code as u8),
                49 => self.bg = Colour::Default,
                // The underline's colour (58) is consumed and dropped.
                38 | 48 | 58 => {
                    let mut more: Vec<&str> = parts.collect();
                    if more.is_empty() {
                        let count = match params.peek().copied() {
                            Some("5") => 2,
                            Some("2") => 4,
                            _ => 0,
                        };
                        more.extend(params.by_ref().take(count));
                    }
                    let colour = Colour::extended(&more);
                    match code {
                        38 => self.fg = colour,
                        48 => self.bg = colour,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    /// The parameters that set this style from none, `;`-separated.
    fn params(&self) -> String {
        let flags = [
            (self.bold, "1"),
            (self.dim, "2"),
            (self.italic, "3"),
            (self.underline, "4"),
            (self.blink, "5"),
            (self.inverse, "7"),
            (self.hidden, "8"),
            (self.strike, "9"),
        ];
        let mut params = String::new();
        for (_, p) in flags.iter().filter(|(on, _)| *on) {
            if !params.is_empty() {
                params.push(';');
            }
            params.push_str(p);
        }
        self.fg.push_params(38, &mut params);
        self.bg.push_params(48, &mut params);
        params
    }
}

/// Change `output.ansi`'s colour from `emitted` to `to`, as one sequence.
fn restyle(ansi: &mut Vec<u8>, emitted: &mut Style, to: &Style) {
    if emitted == to {
        return;
    }
    let params = to.params();
    let sequence = match (*emitted == Style::default(), params.is_empty()) {
        (_, true) => "\x1b[0m".to_owned(),
        (true, false) => format!("\x1b[{params}m"),
        (false, false) => format!("\x1b[0;{params}m"),
    };
    ansi.extend_from_slice(sequence.as_bytes());
    *emitted = *to;
}

/// The furthest column a cursor movement reaches (`ESC [ 999 C`): a line's
/// own text goes as far as it is long.
const MAX_COLUMN: usize = 4096;

/// A line as it settled: its visible text, which colour each run of it is
/// in (where the run ends in `visible`, an index into `styles`), and the
/// colours, the last the one the line ends in.
struct Line {
    visible: String,
    runs: Vec<(usize, usize)>,
    styles: Vec<Style>,
}

/// Whether `c` is a control character the files drop (C0 but tab, DEL, C1).
fn dropped(c: char) -> bool {
    (c < ' ' && c != '\t') || ('\x7f'..='\u{9f}').contains(&c)
}

/// The parameters of `seg`, an escape sequence `ESC [ params final`, or ""
/// for another kind.
fn csi_params(seg: &[u8]) -> &str {
    match seg.strip_prefix(b"\x1b[") {
        Some(rest) => std::str::from_utf8(&rest[..rest.len().saturating_sub(1)]).unwrap_or(""),
        None => "",
    }
}

/// Apply `seg`, an SGR sequence, to `style`; a private one
/// (`ESC [ > 4 ; 2 m`) is not colour, and leaves it.
fn apply_sgr(style: &mut Style, seg: &[u8]) {
    let params = csi_params(seg);
    if !params.starts_with(['<', '=', '>', '?']) {
        style.apply(params);
    }
}

impl Line {
    fn of(line: &str, style: &mut Style) -> Line {
        let before = *style;
        Line::simple(line, style).unwrap_or_else(|| {
            *style = before;
            Cells::of(line, style).line()
        })
    }

    /// A line nothing in it moves back over (no carriage return, backspace,
    /// cursor movement, erase or combining mark), its text as written; else
    /// None, for [`Cells`].
    fn simple(line: &str, style: &mut Style) -> Option<Line> {
        use bazel::console::ansi;
        let mut out = Line {
            visible: String::with_capacity(line.len()),
            runs: vec![],
            styles: vec![*style],
        };
        for (is_escape, seg) in ansi::segments(line.as_bytes()) {
            if is_escape {
                if ansi::is_sgr(seg) {
                    apply_sgr(style, seg);
                    if out.styles.last() != Some(style) {
                        out.end_run();
                        out.styles.push(*style);
                    }
                } else if seg.starts_with(b"\x1b[")
                    && matches!(seg.last(), Some(b'C' | b'D' | b'G' | b'K'))
                {
                    return None;
                }
                continue;
            }
            let text = std::str::from_utf8(seg).unwrap_or_default();
            if seg.is_ascii() && !seg.iter().any(|&b| b < b' ' || b == 0x7f) {
                out.visible.push_str(text);
                continue;
            }
            for c in text.chars() {
                match c {
                    '\r' | '\x08' => return None,
                    '\t' => out.visible.push(c),
                    c if dropped(c) => {}
                    c if !c.is_ascii() && ansi::char_width(c) == 0 => return None,
                    c => out.visible.push(c),
                }
            }
        }
        out.end_run();
        Some(out)
    }

    /// End the run of text in the current colour.
    fn end_run(&mut self) {
        let end = self.visible.len();
        if self.runs.last().is_none_or(|(at, _)| *at < end) {
            self.runs.push((end, self.styles.len() - 1));
        }
    }

    /// Write the line to `plain` and `ansi`, each secret as `<REDACTED>` in
    /// the colour of its first character.
    fn write(
        &self,
        plain: &mut Vec<u8>,
        ansi: &mut Vec<u8>,
        emitted: &mut Style,
        secrets: &[String],
    ) {
        let hidden = redactions(&self.visible, secrets);
        let mut hidden = hidden.iter().peekable();
        let text = |plain: &mut Vec<u8>, ansi: &mut Vec<u8>, s: &str| {
            plain.extend_from_slice(s.as_bytes());
            ansi.extend_from_slice(s.as_bytes());
        };
        let mut pos = 0;
        for &(end, style) in &self.runs {
            while pos < end {
                restyle(ansi, emitted, &self.styles[style]);
                match hidden.next_if(|r| r.start < end) {
                    Some(range) => {
                        text(plain, ansi, &self.visible[pos..range.start]);
                        text(plain, ansi, bazel::bes::redact::REDACTED);
                        pos = range.end;
                    }
                    None => {
                        text(plain, ansi, &self.visible[pos..end]);
                        pos = end;
                    }
                }
            }
        }
        restyle(ansi, emitted, self.styles.last().unwrap());
    }
}

/// One column of a line that moves back over itself.
#[derive(Clone)]
enum Cell {
    /// A character, the combining marks that join it, how many columns it
    /// takes (a tab, to the next stop) and its colour, an index into
    /// [`Cells::styles`].
    Char(char, Option<Box<str>>, usize, usize),
    /// A further column of the character before it.
    Rest,
}

/// A line settled column by column, as a terminal shows it.
struct Cells {
    cells: Vec<Cell>,
    styles: Vec<Style>,
}

impl Cells {
    fn of(line: &str, style: &mut Style) -> Cells {
        use bazel::console::ansi;
        let mut out = Cells {
            cells: vec![],
            styles: vec![*style],
        };
        let mut col = 0usize;
        for (is_escape, seg) in ansi::segments(line.as_bytes()) {
            if is_escape {
                if ansi::is_sgr(seg) {
                    apply_sgr(style, seg);
                    if out.styles.last() != Some(style) {
                        out.styles.push(*style);
                    }
                } else if seg.starts_with(b"\x1b[") {
                    let params = csi_params(seg);
                    // A count too large to parse is as far as it goes.
                    let n = match params.parse::<usize>() {
                        Ok(n) => n.max(1),
                        Err(_)
                            if !params.is_empty() && params.bytes().all(|b| b.is_ascii_digit()) =>
                        {
                            usize::MAX
                        }
                        Err(_) => 1,
                    };
                    let furthest = MAX_COLUMN.max(out.cells.len());
                    match (seg[seg.len() - 1], params) {
                        (b'K', "" | "0") => out.erase(col, usize::MAX),
                        (b'K', "1") => out.erase(0, col.saturating_add(1)),
                        (b'K', _) => out.erase(0, usize::MAX),
                        (b'C', _) => col = col.saturating_add(n).min(furthest),
                        (b'D', _) => col = col.saturating_sub(n),
                        (b'G', _) => col = (n - 1).min(furthest),
                        _ => {}
                    }
                }
                continue;
            }
            // An escape is ASCII, so the text between is whole characters.
            let text = std::str::from_utf8(seg).unwrap_or_default();
            for c in text.chars() {
                match c {
                    '\r' => col = 0,
                    '\x08' => col = col.saturating_sub(1),
                    c if dropped(c) => {}
                    c => col = out.put(col, c),
                }
            }
        }
        out
    }

    /// Erase the columns from `from` up to `to`: blanks within the line,
    /// nothing past its end.
    fn erase(&mut self, from: usize, to: usize) {
        if to >= self.cells.len() {
            self.cells.truncate(from);
            return;
        }
        if from >= to {
            return;
        }
        self.break_at(from);
        self.break_at(to - 1);
        for cell in &mut self.cells[from..to] {
            *cell = Cell::Char(' ', None, 1, 0);
        }
    }

    /// Blank columns up to `col`, where the line is shorter.
    fn pad(&mut self, col: usize) {
        let blank = Cell::Char(' ', None, 1, self.styles.len() - 1);
        if self.cells.len() < col {
            self.cells.resize(col, blank);
        }
    }

    /// Where a character that takes more than one column is written over
    /// in part, it is gone whole: blanks in its columns.
    fn break_at(&mut self, at: usize) {
        let Some(head) = self.cells[..(at + 1).min(self.cells.len())]
            .iter()
            .rposition(|cell| matches!(cell, Cell::Char(..)))
        else {
            return;
        };
        let Cell::Char(_, _, width, style) = self.cells[head] else {
            return;
        };
        if width > 1 && at < head + width {
            let end = (head + width).min(self.cells.len());
            for cell in &mut self.cells[head..end] {
                *cell = Cell::Char(' ', None, 1, style);
            }
        }
    }

    /// Write `c` at `col`, over what is there; where the next one goes.
    fn put(&mut self, col: usize, c: char) -> usize {
        let style = self.styles.len() - 1;
        let width = match c {
            '\t' => (col / 8 + 1) * 8 - col,
            c if c.is_ascii() => 1,
            c => bazel::console::ansi::char_width(c),
        };
        if c == '\t' && col < self.cells.len() {
            // Over text a tab only moves.
            return (col + width).min(MAX_COLUMN.max(self.cells.len()));
        }
        if width == 0 {
            // Joins the character before it.
            let end = col.min(self.cells.len());
            if let Some(Cell::Char(_, marks, _, _)) = self.cells[..end]
                .iter_mut()
                .rev()
                .find(|cell| matches!(cell, Cell::Char(..)))
            {
                let mut joined = marks.take().map(String::from).unwrap_or_default();
                joined.push(c);
                *marks = Some(joined.into_boxed_str());
                return col;
            }
            // With nothing before it, it joins a blank.
            let next = self.put(col, ' ');
            return self.put(next, c);
        }
        if col == self.cells.len() && width == 1 {
            self.cells.push(Cell::Char(c, None, 1, style));
            return col + 1;
        }
        self.pad(col + width);
        self.break_at(col);
        self.break_at(col + width - 1);
        self.cells[col] = Cell::Char(c, None, width, style);
        for rest in &mut self.cells[col + 1..col + width] {
            *rest = Cell::Rest;
        }
        col + width
    }

    /// As a [`Line`]: its text, in runs of one colour.
    fn line(self) -> Line {
        let mut out = Line {
            visible: String::with_capacity(self.cells.len()),
            runs: vec![],
            styles: self.styles,
        };
        let mut run_style = None;
        for cell in &self.cells {
            let Cell::Char(c, marks, _, style) = cell else {
                continue;
            };
            if run_style != Some(*style) {
                if let Some(previous) = run_style {
                    out.runs.push((out.visible.len(), previous));
                }
                run_style = Some(*style);
            }
            out.visible.push(*c);
            if let Some(marks) = marks {
                out.visible.push_str(marks);
            }
        }
        if let Some(previous) = run_style {
            out.runs.push((out.visible.len(), previous));
        }
        out
    }
}

/// Where in `text` a secret is, as sorted byte ranges that do not overlap.
fn redactions(text: &str, secrets: &[String]) -> Vec<std::ops::Range<usize>> {
    let mut found: Vec<std::ops::Range<usize>> = secrets
        .iter()
        .flat_map(|s| text.match_indices(s.as_str()).map(|(i, m)| i..i + m.len()))
        .collect();
    found.sort_by_key(|r| r.start);
    let mut merged: Vec<std::ops::Range<usize>> = vec![];
    for range in found {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// AXL `print()` that survives a closed stderr.
///
/// Starlark's default handler is a bare `eprintln!`, which panics when the
/// write fails — so any `print()` from AXL takes the process down as soon as a
/// reader leaves (`aspect build … | head`). The panic aborts mid-task, so the
/// task never runs its terminal update and its status-surface entry is
/// stranded showing "running".
///
/// Install with `eval.set_print_handler(&TOLERANT_PRINT_HANDLER)`. Discarding
/// the write matches `outln!`/`errln!`: nobody is reading the output, and
/// finishing the task matters more than the line nobody sees.
///
/// Where [`terminal_links`] holds, as Bazel's console is, a line's URLs and
/// paths to files that exist are written as links
/// ([`bazel::console::links`]), a relative path from where the process runs.
pub struct TolerantPrintHandler;

/// Whether this process's stderr is a person's terminal that shows links:
/// a TTY with a `TERM` other than `dumb` (which writes an escape as text),
/// not a CI job's (a file's link names the runner's disk, and a log
/// viewer may show the escape), and no coding agent reading it (its
/// environment set, as the Console feature's `auto` profile checks), which
/// gets each path once, plain.
pub(crate) fn terminal_links() -> bool {
    use std::io::IsTerminal as _;
    std::io::stderr().is_terminal()
        && std::env::var_os("TERM").is_some_and(|t| !t.is_empty() && t != "dumb")
        && !crate::ci::on_recognized_ci()
        && !crate::agent::detected(|v| std::env::var_os(v))
}

/// What a link to a file opens, as the Console feature's `file_hyperlink_format`
/// says once a task resolves it ([`set_link_format`]); until then
/// `ASPECT_FILE_HYPERLINK_FORMAT`, else the `config.json` files' `editor`
/// ([`config_link_format`]); and for `auto`, the editor
/// `LinkFormat::auto` finds (its terminal's, `$VISUAL`/`$EDITOR`'s, one
/// installed, on `PATH` or as a macOS app), else none.
static LINK_FORMAT: std::sync::RwLock<Option<bazel::console::links::LinkFormat>> =
    std::sync::RwLock::new(None);

/// Make links to files in `format` from now on, for Bazel's console and the
/// CLI's own lines; `None` for `auto`.
pub(crate) fn set_link_format(format: Option<bazel::console::links::LinkFormat>) {
    if let Ok(mut slot) = LINK_FORMAT.write() {
        // `auto` is resolved now: an explicit `auto` is a choice made, not
        // one left to the environment.
        *slot = Some(format.unwrap_or_else(|| AUTO_LINK_FORMAT.clone()));
    }
}

/// What a link to a file opens now; see [`LINK_FORMAT`]. Before a task
/// resolves the Console, [`unconfigured_link_format`] of the `config.json`
/// files' `editor`.
pub(crate) fn link_format() -> bazel::console::links::LinkFormat {
    LINK_FORMAT
        .read()
        .ok()
        .and_then(|f| f.clone())
        .or_else(|| unconfigured_link_format(config_link_format()))
        .unwrap_or_else(|| AUTO_LINK_FORMAT.clone())
}

/// Make links to files in [`unconfigured_link_format`] of `config` from now
/// on: for a task whose run has no Console feature to resolve them.
pub(crate) fn set_unconfigured_link_format(
    config: Option<Option<bazel::console::links::LinkFormat>>,
) {
    set_link_format(unconfigured_link_format(config));
}

/// What a link to a file opens with no Console arg to say:
/// `ASPECT_FILE_HYPERLINK_FORMAT` when it is a format or `auto`, else
/// `config` (the `config.json` files' `editor`, [`config_link_format`]);
/// `None` for `auto`.
fn unconfigured_link_format(
    config: Option<Option<bazel::console::links::LinkFormat>>,
) -> Option<bazel::console::links::LinkFormat> {
    match env_link_format() {
        Some(Ok(format)) => format,
        _ => config.flatten(),
    }
}

/// The `editor` of the `config.json` files ([`layered_editor`] of the
/// user's `~/.aspect/config.json` and the repository's
/// `.aspect/config.json`), read once for the process. Each invalid one is a
/// warning, once. Unit tests read none, so they never depend on the real
/// `HOME`.
pub(crate) fn config_link_format() -> Option<Option<bazel::console::links::LinkFormat>> {
    static CONFIG: std::sync::LazyLock<Option<Option<bazel::console::links::LinkFormat>>> =
        std::sync::LazyLock::new(|| {
            if cfg!(test) {
                return None;
            }
            let user = crate::config_json::user_path().ok();
            let repo = crate::config_json::repo_path();
            layered_editor(user.as_deref(), repo.as_deref(), |w| {
                if crate::config_json::first_time(w) {
                    crate::diag::warn(w)
                }
            })
        });
    CONFIG.clone()
}

/// The `editor` of the `config.json` at `user`, else of the one at `repo`, a
/// team's default: `None` when neither sets one, else a format or `None` for
/// `auto`. The repository's takes an editor's name, `auto` or `none` only,
/// never a URL template, so a cloned repository's data cannot make a click
/// open any scheme it likes. An invalid one (for anyone, a `file://` format)
/// goes to `warn` and reads as unset, so the next decides.
pub(crate) fn layered_editor(
    user: Option<&std::path::Path>,
    repo: Option<&std::path::Path>,
    mut warn: impl FnMut(&str),
) -> Option<Option<bazel::console::links::LinkFormat>> {
    [(user, false), (repo, true)]
        .into_iter()
        .filter_map(|(path, named_only)| Some((path?, named_only)))
        .find_map(|(path, named_only)| {
            editor_link_format(path, named_only).unwrap_or_else(|warning| {
                warn(&warning);
                None
            })
        })
}

/// The `editor` in the `config.json` at `path`: `None` when the file or the
/// key is missing, else a format or `None` for `auto`, or a warning naming the
/// file when the file is unreadable or not a JSON object (worded as every
/// reader of the file words it, see `crate::config_json::object`) or the
/// value is not a format (`file` and every `file://` one are not), or, with
/// `named_only`, is a URL template.
pub(crate) fn editor_link_format(
    path: &std::path::Path,
    named_only: bool,
) -> Result<Option<Option<bazel::console::links::LinkFormat>>, String> {
    let ignored = |e: String| format!("{e}; ignoring it");
    let Some(value) = crate::config_json::editor(path).map_err(ignored)? else {
        return Ok(None);
    };
    if value == "auto" {
        return Ok(Some(None));
    }
    let format = bazel::console::links::LinkFormat::parse(&value)
        .map_err(|e| ignored(format!("{} \"editor\": {e}", path.display())))?;
    if named_only && !repository_may_choose(&value, &format) {
        return Err(ignored(format!(
            "{} \"editor\": a repository's config.json takes auto, none or an editor's name, \
             not {value:?}; a URL template belongs in your own ~/.aspect/config.json",
            path.display()
        )));
    }
    Ok(Some(Some(format)))
}

/// Whether a repository's `config.json` may choose `value` (parsed as
/// `format`): `none` or an editor's name, never a URL template, which may
/// name any scheme.
fn repository_may_choose(value: &str, format: &bazel::console::links::LinkFormat) -> bool {
    use bazel::console::links::LinkFormat;
    match format {
        LinkFormat::None => true,
        LinkFormat::Template(_) => LinkFormat::ALIASES.iter().any(|(name, _)| *name == value),
    }
}

/// Whether links' URLs are written beside them too: Terminal.app, which
/// shows no OSC 8 link but opens a URL written out by Cmd+double-click.
pub(crate) fn links_as_text() -> bool {
    std::env::var_os("TERM_PROGRAM").is_some_and(|t| t == "Apple_Terminal")
}

/// `auto`'s format for this process: its environment, and the editors
/// installed (on its `PATH`, or as macOS apps), looked up once.
static AUTO_LINK_FORMAT: std::sync::LazyLock<bazel::console::links::LinkFormat> =
    std::sync::LazyLock::new(|| {
        bazel::console::links::LinkFormat::auto(
            |v| std::env::var_os(v),
            bazel::console::links::installed,
        )
    });

/// `ASPECT_FILE_HYPERLINK_FORMAT`, when set and not empty: a format, `None` for
/// `auto`, or why it is neither.
pub(crate) fn env_link_format() -> Option<Result<Option<bazel::console::links::LinkFormat>, String>>
{
    let value = std::env::var("ASPECT_FILE_HYPERLINK_FORMAT")
        .ok()
        .filter(|v| !v.is_empty())?;
    Some(match value.as_str() {
        "auto" => Ok(None),
        value => bazel::console::links::LinkFormat::parse(value)
            .map(Some)
            .map_err(|e| format!("ASPECT_FILE_HYPERLINK_FORMAT: {e}")),
    })
}

/// Links what the CLI's own lines name, when [`terminal_links`] holds.
static PRINT_LINKER: std::sync::LazyLock<Option<std::sync::Mutex<bazel::console::links::Linker>>> =
    std::sync::LazyLock::new(|| {
        if !terminal_links() {
            return None;
        }
        let base = std::env::current_dir().ok()?;
        Some(std::sync::Mutex::new(
            bazel::console::links::Linker::new(vec![base]).with_files(
                bazel::console::links::FileLinks::none()
                    .written(links_as_text())
                    .styled(crate::color::console_supports_color()),
            ),
        ))
    });

/// `line` with its URLs and paths linked as AXL `print()`'s are, for a line
/// the runtime writes itself (the banner); off a person's terminal, as it is
/// but for `ansi.unlinked`'s markers ([`bazel::console::links::strip_unlinked`]).
pub(crate) fn linked(line: &str) -> String {
    match PRINT_LINKER.as_ref().and_then(|l| l.lock().ok()) {
        Some(mut linker) => {
            linker.set_format(link_format());
            String::from_utf8(linker.link_written(line.as_bytes()))
                .unwrap_or_else(|_| line.to_string())
        }
        None => String::from_utf8_lossy(&bazel::console::links::strip_unlinked(line.as_bytes()))
            .into_owned(),
    }
}

impl starlark::PrintHandler for TolerantPrintHandler {
    fn println(&self, text: &str) -> starlark::Result<()> {
        match PRINT_LINKER.as_ref().and_then(|l| l.lock().ok()) {
            Some(mut linker) => {
                use std::io::Write as _;
                linker.set_format(link_format());
                let mut line = linker.link_written(text.as_bytes());
                line.push(b'\n');
                let _ = std::io::stderr().write_all(&line);
                copy_output(&line);
            }
            None => {
                let text = bazel::console::links::strip_unlinked(text.as_bytes());
                errln!("{}", String::from_utf8_lossy(&text));
            }
        }
        Ok(())
    }
}

/// Shared instance: the handler holds no state of its own, only the
/// process-wide [`PRINT_LINKER`].
pub static TOLERANT_PRINT_HANDLER: TolerantPrintHandler = TolerantPrintHandler;

/// Source scan behind the `no_panicking_print_macros` tests.
///
/// Returns `path:line: text` for every `println!` / `eprintln!` / `print!` /
/// `eprint!` outside a test module. Each crate embeds its own sources and
/// asserts the result is empty, so a reintroduced macro fails at the source
/// rather than as a task stranded mid-run months later.
///
/// `#[doc(hidden)]`: it exists for those tests, not as API.
#[doc(hidden)]
pub fn panicking_print_macros(sources: &include_dir::Dir<'_>) -> Vec<String> {
    /// The macros that panic on a failed write. Matched with the trailing `(`
    /// so `outln!(` and `errln!(` do not collide with `println!(`/`eprintln!(`.
    const BANNED: [&str; 4] = ["println!(", "eprintln!(", "print!(", "eprint!("];

    let mut found = Vec::new();
    let mut dirs = vec![sources];
    while let Some(dir) = dirs.pop() {
        for entry in dir.entries() {
            match entry {
                include_dir::DirEntry::Dir(d) => dirs.push(d),
                include_dir::DirEntry::File(f) => {
                    let path = f.path().to_string_lossy().to_string();
                    // `out.rs` defines the replacements in terms of the real writers.
                    if !path.ends_with(".rs") || path.ends_with("out.rs") {
                        continue;
                    }
                    let Some(text) = f.contents_utf8() else {
                        continue;
                    };
                    for (n, line) in text.lines().enumerate() {
                        let trimmed = line.trim_start();
                        // A panic in a test is just a test failure.
                        if trimmed.starts_with("#[cfg(test)]") {
                            break;
                        }
                        if trimmed.starts_with("//") {
                            continue;
                        }
                        if BANNED.iter().any(|m| line.contains(m)) {
                            found.push(format!("{path}:{}: {trimmed}", n + 1));
                        }
                    }
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use bazel::console::links::LinkFormat;

    #[test]
    fn the_user_editor_is_a_format_auto_or_unset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(
            super::editor_link_format(&path, false),
            Ok(None),
            "missing file"
        );
        std::fs::write(&path, r#"{"deployments": []}"#).unwrap();
        assert_eq!(
            super::editor_link_format(&path, false),
            Ok(None),
            "missing key"
        );
        std::fs::write(&path, r#"{"editor": "auto"}"#).unwrap();
        assert_eq!(super::editor_link_format(&path, false), Ok(Some(None)));
        std::fs::write(&path, r#"{"editor": "zed"}"#).unwrap();
        assert_eq!(
            super::editor_link_format(&path, false),
            Ok(Some(Some(LinkFormat::parse("zed").unwrap())))
        );
        std::fs::write(&path, r#"{"editor": "myeditor://{path}"}"#).unwrap();
        assert_eq!(
            super::editor_link_format(&path, false),
            Ok(Some(Some(LinkFormat::Template("myeditor://{path}".into()))))
        );
    }

    #[test]
    fn an_invalid_user_editor_is_a_warning_naming_the_file_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let file = path.display().to_string();
        for content in [r#"{"editor": "nope"}"#, r#"{"editor": 1}"#, "{", "[]"] {
            std::fs::write(&path, content).unwrap();
            let warning = super::editor_link_format(&path, false).unwrap_err();
            assert!(warning.contains(&file), "{content}: {warning}");
        }
    }

    #[test]
    fn the_users_editor_overrides_the_repositorys_and_an_invalid_one_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.json");
        let repo = dir.path().join("repo.json");
        let zed = Some(Some(LinkFormat::parse("zed").unwrap()));
        let cursor = Some(Some(LinkFormat::parse("cursor").unwrap()));
        let layered = || {
            let mut warnings = Vec::new();
            let format =
                super::layered_editor(Some(&user), Some(&repo), |w| warnings.push(w.to_owned()));
            (format, warnings)
        };
        assert_eq!(layered(), (None, vec![]), "neither file");
        std::fs::write(&repo, r#"{"editor": "zed"}"#).unwrap();
        assert_eq!(layered(), (zed.clone(), vec![]), "the repository's default");
        std::fs::write(&user, r#"{"editor": "cursor"}"#).unwrap();
        assert_eq!(layered(), (cursor, vec![]), "the user's beats it");
        std::fs::write(&user, r#"{"editor": "nope"}"#).unwrap();
        let (format, warnings) = layered();
        assert_eq!(format, zed, "an invalid user's falls through");
        assert!(warnings[0].contains("user.json"), "{warnings:?}");
        std::fs::write(&repo, r#"{"editor": "nope"}"#).unwrap();
        let (format, warnings) = layered();
        assert_eq!(format, None);
        assert!(warnings[1].contains("repo.json"), "{warnings:?}");
        assert_eq!(super::layered_editor(None, None, |_| {}), None);
    }

    #[test]
    fn a_repositorys_editor_takes_a_name_but_no_template() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        for named in ["auto", "none", "zed", "cursor"] {
            std::fs::write(&path, format!(r#"{{"editor": "{named}"}}"#)).unwrap();
            assert!(super::editor_link_format(&path, true).is_ok(), "{named}");
        }
        for refused in ["file", "kitty", "file://{path}"] {
            std::fs::write(&path, format!(r#"{{"editor": "{refused}"}}"#)).unwrap();
            for named_only in [true, false] {
                assert!(
                    super::editor_link_format(&path, named_only).is_err(),
                    "{refused} is no one's"
                );
            }
        }
        std::fs::write(&path, r#"{"editor": "myeditor://{path}"}"#).unwrap();
        let warning = super::editor_link_format(&path, true).unwrap_err();
        assert!(warning.contains("not \"myeditor://{path}\""), "{warning}");
        assert!(warning.contains(&path.display().to_string()), "{warning}");
        assert!(
            super::editor_link_format(&path, false).is_ok(),
            "the user's may"
        );
        let user = dir.path().join("user.json");
        let mut warnings = Vec::new();
        let format =
            super::layered_editor(Some(&user), Some(&path), |w| warnings.push(w.to_owned()));
        assert_eq!(format, None, "the repository's template is refused");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn no_panicking_print_macros() {
        static SRC: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src");
        let found = super::panicking_print_macros(&SRC);
        assert!(
            found.is_empty(),
            "use outln!/errln!/out! instead (see CLAUDE.md):\n  {}",
            found.join("\n  ")
        );
    }
}

#[cfg(test)]
mod output_log_tests {
    use super::{Copies, Early, MAX_PENDING, OUTPUT_ANSI, OUTPUT_LOG, OutputLog};

    /// Its own log, not the process's, which other tests replace.
    fn open_log(secrets: &[&str]) -> (tempfile::TempDir, OutputLog) {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets.iter().map(|s| s.to_string()).collect();
        let log = OutputLog::open(dir.path(), secrets).unwrap();
        (dir, log)
    }

    fn files(dir: &tempfile::TempDir) -> (String, String) {
        let read = |name| std::fs::read_to_string(dir.path().join(name)).unwrap();
        (read(OUTPUT_LOG), read(OUTPUT_ANSI))
    }

    fn pair(plain: &str, ansi: &str) -> (String, String) {
        (plain.into(), ansi.into())
    }

    #[test]
    fn colour_is_kept_in_one_file_and_stripped_from_the_other() {
        let (dir, mut log) = open_log(&[]);
        log.push(b"\x1b[1;32mINFO:\x1b[0m built\nplain \x1b[1m\x1b[33mbold\x1b[0m\n");
        assert_eq!(
            files(&dir),
            pair(
                "INFO: built\nplain bold\n",
                "\x1b[1;32mINFO:\x1b[0m built\nplain \x1b[1;33mbold\x1b[0m\n"
            )
        );
    }

    /// Each attribute is set and cleared on its own, and the colour is
    /// written as one sequence of what is set.
    #[test]
    fn colour_is_kept_as_its_state() {
        let (dir, mut log) = open_log(&[]);
        log.push(
            b"\x1b[1;31mA\x1b[22mB\x1b[39mC\x1b[38;5;208;48:2::1:2:3mD\x1b[49;4mE\x1b[24;39mF\n",
        );
        assert_eq!(
            files(&dir).1,
            "\x1b[1;31mA\x1b[0;31mB\x1b[0mC\x1b[38;5;208;48;2;1;2;3mD\x1b[0;4;38;5;208mE\x1b[0mF\n"
        );
    }

    /// A writer that clears each attribute (`39`, `22`) rather than
    /// resetting keeps a state of one size: its lines cost the same however
    /// many came before.
    #[test]
    fn colour_that_is_never_reset_stays_cheap() {
        let (_dir, mut log) = open_log(&[]);
        let line = b"\x1b[31mred\x1b[39m \x1b[1mbold\x1b[22m \x1b[4;32mok\x1b[24;39m\n";
        let started = std::time::Instant::now();
        for _ in 0..50_000 {
            log.push(line);
        }
        assert_eq!(log.task.style, super::Style::default());
        // Generous: the state's size is what keeps it linear.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "took {:?}",
            started.elapsed()
        );
    }

    /// Bazel's usual lines, nothing in them moving back, take no cells.
    #[test]
    fn plain_lines_are_written_as_they_are() {
        let (dir, mut log) = open_log(&["s3cr3t"]);
        log.push(b"\x1b[32mINFO:\x1b[0m From Compiling a.cc:\ta\tb s3cr3t \xe6\x97\xa5\n");
        assert_eq!(
            files(&dir),
            pair(
                "INFO: From Compiling a.cc:\ta\tb <REDACTED> 日\n",
                "\x1b[32mINFO:\x1b[0m From Compiling a.cc:\ta\tb <REDACTED> 日\n"
            )
        );
    }

    /// A cursor movement goes no further than [`super::MAX_COLUMN`], however
    /// far it asks, and never overflows.
    #[test]
    fn a_cursor_movement_is_bounded() {
        let (dir, mut log) = open_log(&[]);
        log.push(b"a\x1b[18446744073709551615Cb\n");
        log.push(b"a\x1b[20000000Cb\x1b[9999Gc\x1b[99999999999999999999Dd\n");
        let plain = files(&dir).0;
        let lines: Vec<&str> = plain.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), super::MAX_COLUMN + 1);
        assert!(lines[0].starts_with('a') && lines[0].ends_with('b'));
        assert!(lines[1].starts_with('d') && lines[1].len() <= super::MAX_COLUMN + 2);
    }

    /// An SGR sequence that is not colour (a private one, `ESC [ > 4 ; 2 m`)
    /// leaves it, and the underline's colour (58) takes its parameters.
    #[test]
    fn sequences_that_are_not_colour_leave_it() {
        let (dir, mut log) = open_log(&[]);
        log.push(b"\x1b[>4;2ma\x1b[58;5;1mb\x1b[58;2;1;2;3mc\x1b[?1md\n");
        assert_eq!(files(&dir), pair("abcd\n", "abcd\n"));
    }

    /// A line settles column by column: a wide character takes two, one
    /// half written over is gone whole, a combining mark joins the one
    /// before it, and a tab spans to the next stop of 8, kept as a tab
    /// unless written over in part.
    #[test]
    fn a_line_settles_by_column() {
        let (dir, mut log) = open_log(&[]);
        log.push("日本\rab\n".as_bytes());
        log.push("日本\r\x1b[2Ca\n".as_bytes());
        log.push("cafe\u{301}\rC\n".as_bytes());
        log.push("a\tb\r\tc\n".as_bytes());
        log.push("\tx\rab\n".as_bytes());
        log.push(b"\x1b[1Kx\n");
        assert_eq!(
            files(&dir).0,
            "ab本\n日a \nCafe\u{301}\na\tc\nab      x\nx\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_files_are_readable_by_their_user_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let (dir, _log) = open_log(&[]);
        for name in [OUTPUT_LOG, OUTPUT_ANSI] {
            let mode = std::fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }

    #[test]
    fn every_other_escape_and_control_is_dropped() {
        let (dir, mut log) = open_log(&[]);
        log.push(
            "\x1b[1A\x1b]8;;file:///a\x1b\\\x1b[31ma\x1b[0m\x1b]8;;\x1b\\\x07\x01\x7f\u{85}\u{9b} \x1b(Bb\n"
                .as_bytes(),
        );
        assert_eq!(files(&dir), pair("a b\n", "\x1b[31ma\x1b[0m b\n"));
    }

    /// A line is kept as it settled: a carriage return or a backspace moves
    /// back and what follows overwrites, an erase in line erases, the
    /// colour set before it kept, even across writes.
    #[test]
    fn carriage_returns_overwrite_in_place() {
        let (dir, mut log) = open_log(&[]);
        for part in [
            "\x1b[33m10%\r",
            "99%\x1b[0m\n",
            "spin 2\rdone\n",
            "100%\r\x1b[K9%\n",
            "ab\x08c\n",
            "end\r\n",
        ] {
            log.push(part.as_bytes());
        }
        assert_eq!(
            files(&dir),
            pair(
                "99%\ndone 2\n9%\nac\nend\n",
                "\x1b[33m99%\x1b[0m\ndone 2\n9%\nac\nend\n"
            )
        );
    }

    /// Found in the plain text, so a secret split across writes or by a
    /// colour change is redacted too, in its first character's colour.
    #[test]
    fn secrets_are_redacted_in_both() {
        let (dir, mut log) = open_log(&["s3cr3t"]);
        log.push(b"token s3cr3t\n");
        for part in ["key s3", "c\x1b[31mr3t", " after\x1b[0m\n"] {
            log.push(part.as_bytes());
        }
        assert_eq!(
            files(&dir),
            pair(
                "token <REDACTED>\nkey <REDACTED> after\n",
                "token <REDACTED>\nkey <REDACTED>\x1b[31m after\x1b[0m\n"
            )
        );
    }

    /// As in the task's record: one shorter than `MIN_SCRUBBED` would match
    /// too much that is not it.
    #[test]
    fn a_short_secret_is_not_redacted_everywhere() {
        let (dir, mut log) = open_log(&["in", ""]);
        log.push(b"defined in bazel-bin/\n");
        assert_eq!(files(&dir).0, "defined in bazel-bin/\n");
    }

    /// An unended line past the most held back is written, but not a secret,
    /// an escape or a character the cut would split.
    #[test]
    fn a_forced_cut_splits_no_secret_escape_or_character() {
        let long = "x".repeat(MAX_PENDING);
        let (dir, mut log) = open_log(&["s3cr3t"]);
        log.push(format!("{long} s3cr").as_bytes());
        log.push(b"3t\n");
        assert_eq!(files(&dir).0, format!("{long} <REDACTED>\n"));
        let (dir, mut log) = open_log(&[]);
        log.push(format!("{long}\x1b[3").as_bytes());
        log.push(b"1mred\x1b[0m\n");
        log.push(&[long.as_bytes(), &"é".as_bytes()[..1]].concat());
        log.push(&"é".as_bytes()[1..]);
        log.push(b"\n");
        let (plain, ansi) = files(&dir);
        assert_eq!(plain, format!("{long}red\n{long}é\n"));
        assert!(
            ansi.starts_with(&format!("{long}\x1b[31mred\x1b[0m\n")),
            "{ansi}"
        );
    }

    /// The console's lines and the task's unended one are kept apart, so
    /// neither splits the other, nor a secret in it.
    #[test]
    fn the_console_does_not_split_the_tasks_line() {
        let (dir, mut log) = open_log(&["s3cr3t"]);
        log.push(b"task s3c");
        log.push_console(b"INFO: from bazel\n");
        log.push(b"r3t\n");
        assert_eq!(files(&dir).0, "INFO: from bazel\ntask <REDACTED>\n");
    }

    /// One chunk past the cap is cut at a line's end, then the note.
    #[test]
    fn each_file_stops_at_the_cap() {
        let (dir, mut log) = open_log(&[]);
        log.cap = 1 << 20;
        let line = format!("\x1b[32m{}\x1b[0m\n", "x".repeat(1000));
        log.push(line.repeat(2048).as_bytes());
        let (plain, ansi) = files(&dir);
        let text = format!("{}\n", "x".repeat(1000));
        let lines = |line: &str| (1usize << 20) / line.len();
        assert_eq!(
            plain,
            text.repeat(lines(&text)) + "\n... output.log stops here: 1 MiB\n"
        );
        assert_eq!(
            ansi,
            line.repeat(lines(&line)) + "\n... output.ansi stops here: 1 MiB\n"
        );
        log.push(b"more\n");
        assert_eq!(files(&dir).0, plain, "nothing after the note");
    }

    /// What was written before the files opened comes first, its newest
    /// bytes from a line's start; taken, the next task's starts empty.
    #[test]
    fn early_output_comes_first() {
        let mut early = Early(vec![]);
        early.push(&vec![b'o'; super::MAX_EARLY]);
        early.push(b"\n\x1b[1m-> Running build\x1b[0m\n");
        let (dir, mut log) = open_log(&[]);
        log.push(&early.take());
        log.push(b"after\n");
        assert_eq!(
            files(&dir),
            pair(
                "-> Running build\nafter\n",
                "\x1b[1m-> Running build\x1b[0m\nafter\n"
            )
        );
        assert!(early.take().is_empty());
    }

    /// A task's files keep what is printed after it ends, until the next
    /// task's header, which is the next task's.
    #[test]
    fn the_next_tasks_header_is_its_own() {
        let mut copies = Copies::default();
        copies.copy(b"banner\n", false);
        let (first, one) = open_log(&[]);
        copies.keep(one);
        copies.copy(b"first\n", false);
        copies.copy(b"conclusion\n", false);
        copies.begin_early();
        copies.copy(b"-> Running second\n", false);
        let (second, two) = open_log(&[]);
        copies.keep(two);
        copies.copy(b"second\n", false);
        assert_eq!(files(&first).0, "banner\nfirst\nconclusion\n");
        assert_eq!(files(&second).0, "-> Running second\nsecond\n");
    }

    /// What the task wrote before Bazel's console began, its unended line
    /// included, and none of the console; all of it with no console.
    #[test]
    fn the_output_before_the_console() {
        let (_dir, mut log) = open_log(&[]);
        let coloured: &[u8] = b"\x1b[32mone\x1b[0m\ntwo, unended";
        log.push(coloured);
        assert_eq!(log.before_console(1 << 20, false), b"one\ntwo, unended");
        assert_eq!(log.before_console(1 << 20, true), coloured);
        log.push_console(b"INFO: from bazel\n");
        log.push(b" and ended\n");
        assert_eq!(log.before_console(1 << 20, false), b"one\ntwo, unended");
        assert_eq!(log.before_console(1 << 20, true), coloured);
        assert_eq!(log.before_console(7, false), b"unended", "its newest bytes");
        assert_eq!(
            log.before_console(18, true),
            b"two, unended",
            "from a line's start"
        );
    }

    /// A cut at a line's start keeps that line; with no line's start, it
    /// splits no escape and no character.
    #[test]
    fn the_newest_output_is_cut_cleanly() {
        let (_dir, mut log) = open_log(&[]);
        log.push(b"aaaa\nbbbb\n");
        assert_eq!(log.before_console(5, false), b"bbbb\n");
        let (_dir, mut log) = open_log(&[]);
        log.push("\x1b[31mxyz\x1b[0mé".as_bytes());
        log.flush();
        assert_eq!(log.before_console(10, true), "xyz\x1b[0mé".as_bytes());
        assert_eq!(log.before_console(1, true), b"");
    }
}
