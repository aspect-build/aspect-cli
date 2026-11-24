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

/// What a secret's value is written as.
const REDACTED: &[u8] = b"<REDACTED>";

/// One of the task's output files.
struct LogFile {
    name: &'static str,
    path: std::path::PathBuf,
    file: std::fs::File,
    written: u64,
    /// Where Bazel's console's first bytes begin, once it has written any
    /// ([`copy_console_output`]).
    console_at: Option<u64>,
}

impl LogFile {
    fn open(dir: &std::path::Path, name: &'static str) -> std::io::Result<LogFile> {
        let path = dir.join(name);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(LogFile {
            name,
            path,
            file,
            written: 0,
            console_at: None,
        })
    }

    fn write(&mut self, text: &[u8], cap: u64) {
        use std::io::Write as _;
        if self.written >= cap || text.is_empty() {
            return;
        }
        self.written += text.len() as u64;
        let _ = self.file.write_all(text);
        if self.written >= cap {
            let note = format!("\n... {} stops here: {} MiB\n", self.name, cap >> 20);
            let _ = self.file.write_all(note.as_bytes());
        }
    }

    /// The newest `max` bytes of what was written before the console began
    /// (all of it, and `tail` after it, when no console has), from the
    /// start of a line when the cut leaves one.
    fn before_console(&self, max: u64, tail: &[u8]) -> Vec<u8> {
        use std::io::{Read as _, Seek as _};
        let (end, tail) = match self.console_at {
            Some(at) => (at, &[][..]),
            None => (self.written, tail),
        };
        let start = end.saturating_sub(max.saturating_sub(tail.len() as u64));
        let mut text = vec![];
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return vec![];
        };
        if file.seek(std::io::SeekFrom::Start(start)).is_err()
            || file.take(end - start).read_to_end(&mut text).is_err()
        {
            return vec![];
        }
        text.extend_from_slice(tail);
        let over = text.len().saturating_sub(max as usize);
        text.drain(..over);
        if start > 0 || over > 0 {
            // An escape or a character the cut split would show as text.
            if let Some(i) = text.iter().position(|&b| b == b'\n') {
                text.drain(..=i);
            }
        }
        text
    }
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
    /// The line written so far without its end, kept until it ends so a
    /// carriage return, an escape or a secret split across writes is
    /// handled whole.
    pending: Vec<u8>,
}

static OUTPUT: std::sync::Mutex<Option<OutputLog>> = std::sync::Mutex::new(None);

/// What was written before [`keep_output`], for its files.
static EARLY: std::sync::Mutex<Early> = std::sync::Mutex::new(Early(vec![]));

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
/// forwards it, but not what a child writes to a stream it inherited. What
/// was written before this call (the banner, the task's header) comes
/// first, its newest [`MAX_EARLY`] bytes. Returns which this is, for
/// [`flush_output`].
///
/// Both files keep each line as it settled (what a carriage return wrote
/// over dropped), `secrets` (the task's secret flag values) redacted, and
/// stop at [`MAX_OUTPUT_LOG`] each. [`OUTPUT_LOG`] is plain text, every
/// escape and control character but tab and newline dropped. [`OUTPUT_ANSI`]
/// is the same text with its colour (SGR, `ESC [ ... m`) kept. A secret is
/// found in the plain text, so one split by a colour change is still
/// redacted; the colour changes inside it move to after its `<REDACTED>`.
pub fn keep_output(dir: &std::path::Path, secrets: Vec<String>) -> std::io::Result<u64> {
    let mut log = OutputLog::open(dir, secrets)?;
    let generation = log.generation;
    if let Ok(mut slot) = OUTPUT.lock() {
        // Under the lock `copy_output` takes, so nothing written now is lost.
        if let Ok(mut early) = EARLY.lock() {
            log.push(&early.take());
        }
        *slot = Some(log);
    }
    axl_types::stream::copy_output_to(copy_output);
    Ok(generation)
}

/// Copy `bytes`, as written to stdout or stderr, to the task's output
/// files, a line at a time (an unended one held back, up to
/// [`MAX_PENDING`], until it ends or [`flush_output`]); before
/// [`keep_output`], to what it writes first.
pub fn copy_output(bytes: &[u8]) {
    let Ok(mut slot) = OUTPUT.lock() else { return };
    match slot.as_mut() {
        Some(log) => log.push(bytes),
        None => {
            if let Ok(mut early) = EARLY.lock() {
                early.push(bytes);
            }
        }
    }
}

/// [`copy_output`] for Bazel's console as the console forwards it: where
/// the first of it begins is kept, for [`output_before_console`].
pub fn copy_console_output(bytes: &[u8]) {
    let Ok(mut slot) = OUTPUT.lock() else { return };
    match slot.as_mut() {
        Some(log) => log.push_console(bytes),
        None => {
            if let Ok(mut early) = EARLY.lock() {
                early.push(bytes);
            }
        }
    }
}

/// What the task wrote before Bazel's console began (all it has written,
/// when no console has), as `output.log` keeps it, its newest `max` bytes
/// from a line's start: for a screen taken once the console is under way to
/// show the task's output from its start without Bazel's lines twice.
pub fn output_before_console(max: u64) -> Vec<u8> {
    before_console(max, false)
}

/// [`output_before_console`] as `output.ansi` keeps it, with its colour.
pub fn output_before_console_ansi(max: u64) -> Vec<u8> {
    before_console(max, true)
}

fn before_console(max: u64, colour: bool) -> Vec<u8> {
    let Ok(slot) = OUTPUT.lock() else {
        return vec![];
    };
    slot.as_ref()
        .map_or_else(Vec::new, |log| log.before_console(max, colour))
}

/// Write what the output files hold back of an unended line: the task that
/// [`keep_output`] returned `generation` for is over. A later task's files
/// are left alone.
pub fn flush_output(generation: u64) {
    let Ok(mut slot) = OUTPUT.lock() else { return };
    if let Some(log) = slot.as_mut().filter(|log| log.generation == generation) {
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
            secrets: secrets.into_iter().filter(|s| !s.is_empty()).collect(),
            pending: vec![],
        })
    }

    /// [`copy_console_output`]'s copy: where the console begins marked
    /// first, an unended line of the task's written before it.
    fn push_console(&mut self, bytes: &[u8]) {
        if self.plain.console_at.is_none() {
            self.flush();
            self.plain.console_at = Some(self.plain.written);
            self.ansi.console_at = Some(self.ansi.written);
        }
        self.push(bytes);
    }

    /// [`output_before_console`]'s bytes, or with `colour`
    /// [`output_before_console_ansi`]'s.
    fn before_console(&self, max: u64, colour: bool) -> Vec<u8> {
        let (plain, ansi) = render(&self.pending, &self.secrets);
        match colour {
            false => self.plain.before_console(max, &plain),
            true => self.ansi.before_console(max, &ansi),
        }
    }

    /// `bytes` as written: whole lines kept, an unended one held back.
    fn push(&mut self, bytes: &[u8]) {
        if self.full() {
            return;
        }
        self.pending.extend_from_slice(bytes);
        let end = match self.pending.iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None if self.pending.len() > MAX_PENDING => self.pending.len(),
            None => return,
        };
        let lines: Vec<u8> = self.pending.drain(..end).collect();
        self.write(&lines);
    }

    fn full(&self) -> bool {
        self.plain.written >= self.cap && self.ansi.written >= self.cap
    }

    /// What is held back of an unended line, kept.
    fn flush(&mut self) {
        let rest = std::mem::take(&mut self.pending);
        self.write(&rest);
    }

    /// `bytes`, whole lines, written to both files as [`render`] has them.
    fn write(&mut self, bytes: &[u8]) {
        let (plain, ansi) = render(bytes, &self.secrets);
        self.plain.write(&plain, self.cap);
        self.ansi.write(&ansi, self.cap);
    }
}

/// `bytes` as [`OUTPUT_LOG`] and [`OUTPUT_ANSI`] keep them (see
/// [`keep_output`]): the same visible text, the second with its colour.
fn render(bytes: &[u8], secrets: &[String]) -> (Vec<u8>, Vec<u8>) {
    let text = String::from_utf8_lossy(bytes);
    let mut plain = Vec::with_capacity(bytes.len());
    let mut ansi = Vec::with_capacity(bytes.len());
    for line in text.as_bytes().split_inclusive(|&b| b == b'\n') {
        let (body, end) = match line.strip_suffix(b"\n") {
            Some(body) => (body, &b"\n"[..]),
            None => (line, &b""[..]),
        };
        let (visible, colours) = settle(body);
        let hidden = redactions(&visible, secrets);
        write_redacted(&mut plain, &visible, &[], &hidden);
        write_redacted(&mut ansi, &visible, &colours, &hidden);
        plain.extend_from_slice(end);
        ansi.extend_from_slice(end);
    }
    (plain, ansi)
}

/// A line's visible text as it settled, the text a carriage return was
/// followed by replacing what was before it, and its colour changes, each
/// with where in the text it falls. Every other escape and control
/// character but tab is dropped.
fn settle(line: &[u8]) -> (Vec<u8>, Vec<(usize, &[u8])>) {
    use bazel::console::ansi;
    let mut visible = vec![];
    let mut colours: Vec<(usize, &[u8])> = vec![];
    let mut returned = false;
    for (is_escape, seg) in ansi::segments(line) {
        if is_escape {
            if ansi::is_sgr(seg) {
                colours.push((visible.len(), seg));
            }
            continue;
        }
        for &b in seg {
            if b == b'\r' {
                returned = true;
            } else if b == b'\t' || (b >= 0x20 && b != 0x7f) {
                if returned {
                    // The colour set before it still holds.
                    visible.clear();
                    colours.iter_mut().for_each(|(at, _)| *at = 0);
                    returned = false;
                }
                visible.push(b);
            }
        }
    }
    (visible, colours)
}

/// Where in `text` a secret is, as sorted ranges that do not overlap.
fn redactions(text: &[u8], secrets: &[String]) -> Vec<std::ops::Range<usize>> {
    let mut found = vec![];
    for secret in secrets.iter().map(|s| s.as_bytes()) {
        let mut at = 0;
        while let Some(i) = text[at..]
            .windows(secret.len())
            .position(|window| window == secret)
        {
            found.push(at + i..at + i + secret.len());
            at += i + secret.len();
        }
    }
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

/// `text` with each of `hidden` written as [`REDACTED`], and `colours` at
/// their places; one inside a redaction goes after it.
fn write_redacted(
    out: &mut Vec<u8>,
    text: &[u8],
    colours: &[(usize, &[u8])],
    hidden: &[std::ops::Range<usize>],
) {
    let mut colours = colours.iter().peekable();
    let mut hidden = hidden.iter().peekable();
    let mut at = 0;
    loop {
        let next_colour = colours.peek().map(|(i, _)| *i);
        let next_hidden = hidden.peek().map(|r| r.start);
        match (next_colour, next_hidden) {
            (None, None) => break,
            (Some(c), h) if h.is_none_or(|h| c <= h) => {
                out.extend_from_slice(&text[at..c]);
                at = c;
                out.extend_from_slice(colours.next().unwrap().1);
            }
            _ => {
                let range = hidden.next().unwrap();
                out.extend_from_slice(&text[at..range.start]);
                out.extend_from_slice(REDACTED);
                at = range.end;
                while let Some((_, seq)) = colours.next_if(|(i, _)| *i < range.end) {
                    out.extend_from_slice(seq);
                }
            }
        }
    }
    out.extend_from_slice(&text[at..]);
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
/// not a CI job's (a `file://` link names the runner's disk, and a log
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
/// `ASPECT_FILE_HYPERLINK_FORMAT`, else the user's `editor`
/// ([`user_link_format`]); and for `auto`, the editor
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
/// resolves the Console, [`unconfigured_link_format`] of the user's `editor`.
pub(crate) fn link_format() -> bazel::console::links::LinkFormat {
    LINK_FORMAT
        .read()
        .ok()
        .and_then(|f| f.clone())
        .or_else(|| unconfigured_link_format(user_link_format()))
        .unwrap_or_else(|| AUTO_LINK_FORMAT.clone())
}

/// Make links to files in [`unconfigured_link_format`] of `user` from now
/// on: for a task whose run has no Console feature to resolve them.
pub(crate) fn set_unconfigured_link_format(
    user: Option<Option<bazel::console::links::LinkFormat>>,
) {
    set_link_format(unconfigured_link_format(user));
}

/// What a link to a file opens with no Console arg to say:
/// `ASPECT_FILE_HYPERLINK_FORMAT` when it is a format or `auto`, else `user`
/// (the `editor` in the user's `~/.aspect/config.json`); `None` for `auto`.
fn unconfigured_link_format(
    user: Option<Option<bazel::console::links::LinkFormat>>,
) -> Option<bazel::console::links::LinkFormat> {
    match env_link_format() {
        Some(Ok(format)) => format,
        _ => user.flatten(),
    }
}

/// The `editor` in the user's `~/.aspect/config.json`, read once for the
/// process: `None` when it sets none, else a format or `None` for `auto`. An
/// invalid one is a warning, once, and reads as unset. Unit tests read none,
/// so they never depend on the real `HOME`.
pub(crate) fn user_link_format() -> Option<Option<bazel::console::links::LinkFormat>> {
    static USER: std::sync::LazyLock<Option<Option<bazel::console::links::LinkFormat>>> =
        std::sync::LazyLock::new(|| {
            if cfg!(test) {
                return None;
            }
            let path = crate::config_json::user_path().ok()?;
            editor_link_format(&path).unwrap_or_else(|warning| {
                crate::diag::warn(&warning);
                None
            })
        });
    USER.clone()
}

/// The `editor` in the `config.json` at `path`: `None` when the file or the
/// key is missing, else a format or `None` for `auto`, or a warning naming the
/// file and key when the file is unreadable or not a JSON object or the value
/// is not a format.
pub(crate) fn editor_link_format(
    path: &std::path::Path,
) -> Result<Option<Option<bazel::console::links::LinkFormat>>, String> {
    let ignored = |e: String| format!("{e}; ignoring it");
    let Some(value) = crate::config_json::editor(path).map_err(ignored)? else {
        return Ok(None);
    };
    match value.as_str() {
        "auto" => Ok(Some(None)),
        value => bazel::console::links::LinkFormat::parse(value)
            .map(|format| Some(Some(format)))
            .map_err(|e| ignored(format!("{} \"editor\": {e}", path.display()))),
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
                bazel::console::links::FileLinks::file()
                    .written(links_as_text())
                    .styled(crate::color::console_supports_color()),
            ),
        ))
    });

/// `line` with its URLs and paths linked as AXL `print()`'s are, for a line
/// the runtime writes itself (the banner); as it is off a person's terminal.
pub(crate) fn linked(line: &str) -> String {
    match PRINT_LINKER.as_ref().and_then(|l| l.lock().ok()) {
        Some(mut linker) => {
            linker.set_format(link_format());
            String::from_utf8(linker.link_written(line.as_bytes()))
                .unwrap_or_else(|_| line.to_string())
        }
        None => line.to_string(),
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
            None => errln!("{text}"),
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
        assert_eq!(super::editor_link_format(&path), Ok(None), "missing file");
        std::fs::write(&path, r#"{"deployments": []}"#).unwrap();
        assert_eq!(super::editor_link_format(&path), Ok(None), "missing key");
        std::fs::write(&path, r#"{"editor": "auto"}"#).unwrap();
        assert_eq!(super::editor_link_format(&path), Ok(Some(None)));
        std::fs::write(&path, r#"{"editor": "zed"}"#).unwrap();
        assert_eq!(
            super::editor_link_format(&path),
            Ok(Some(Some(LinkFormat::parse("zed").unwrap())))
        );
        std::fs::write(&path, r#"{"editor": "myeditor://{path}"}"#).unwrap();
        assert_eq!(
            super::editor_link_format(&path),
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
            let warning = super::editor_link_format(&path).unwrap_err();
            assert!(warning.contains(&file), "{content}: {warning}");
            assert!(warning.contains("\"editor\""), "{content}: {warning}");
        }
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
    use super::{Early, OUTPUT_ANSI, OUTPUT_LOG, OutputLog};

    /// Its own log, not the process's, which other tests replace.
    fn log(secrets: &[&str]) -> (tempfile::TempDir, OutputLog) {
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
        let (dir, mut log) = log(&[]);
        log.push(b"\x1b[1;32mINFO:\x1b[0m built\n\tplain\n");
        assert_eq!(
            files(&dir),
            pair(
                "INFO: built\n\tplain\n",
                "\x1b[1;32mINFO:\x1b[0m built\n\tplain\n"
            )
        );
    }

    #[test]
    fn every_other_escape_and_control_is_dropped() {
        let (dir, mut log) = log(&[]);
        log.push(
            b"\x1b[1A\x1b[K\x1b]8;;file:///a\x1b\\\x1b[31ma\x1b[0m\x1b]8;;\x1b\\\x07\x08 \x1b(Bb\n",
        );
        assert_eq!(files(&dir), pair("a b\n", "\x1b[31ma\x1b[0m b\n"));
    }

    /// A line is kept as it settled: what a carriage return wrote over is
    /// gone, the colour set before it kept, even across writes.
    #[test]
    fn carriage_returns_are_settled() {
        let (dir, mut log) = log(&[]);
        for part in ["\x1b[33m10%\r", "99%\x1b[0m\n", "end\r\n", "x\r\r\n"] {
            log.push(part.as_bytes());
        }
        assert_eq!(
            files(&dir),
            pair("99%\nend\nx\n", "\x1b[33m99%\x1b[0m\nend\nx\n")
        );
    }

    /// Found in the plain text, so a secret split across writes or by a
    /// colour change is redacted too, the colour moved after it.
    #[test]
    fn secrets_are_redacted_in_both() {
        let (dir, mut log) = log(&["s3cr3t", ""]);
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

    #[test]
    fn an_unended_line_waits_for_its_end_or_a_flush() {
        let (dir, mut log) = log(&[]);
        log.push(b"one\ntail");
        assert_eq!(files(&dir), pair("one\n", "one\n"));
        log.flush();
        assert_eq!(files(&dir), pair("one\ntail", "one\ntail"));
    }

    /// Each file stops on its own once it holds the cap, with a note.
    #[test]
    fn each_file_stops_at_the_cap() {
        let (dir, mut log) = log(&[]);
        log.cap = 1 << 20;
        let text = format!("{}\n", "x".repeat(1023));
        let coloured = format!("\x1b[32m{}\x1b[0m\n", "x".repeat(1023));
        for _ in 0..2048 {
            log.push(coloured.as_bytes());
        }
        let lines = |line: &str| (1usize << 20).div_ceil(line.len());
        let (plain, ansi) = files(&dir);
        assert_eq!(
            plain,
            text.repeat(lines(&text)) + "\n... output.log stops here: 1 MiB\n"
        );
        assert_eq!(
            ansi,
            coloured.repeat(lines(&coloured)) + "\n... output.ansi stops here: 1 MiB\n"
        );
    }

    /// What was written before the files opened comes first, its newest
    /// bytes from a line's start; taken, the next task's starts empty.
    #[test]
    fn early_output_comes_first() {
        let mut early = Early(vec![]);
        early.push(&vec![b'o'; super::MAX_EARLY]);
        early.push(b"\n\x1b[1m-> Running build\x1b[0m\n");
        let (dir, mut log) = log(&[]);
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

    /// What the task wrote before Bazel's console began, its unended line
    /// included, and none of the console; all of it with no console.
    #[test]
    fn the_output_before_the_console() {
        let (_dir, mut log) = log(&[]);
        let coloured: &[u8] = b"\x1b[32mone\x1b[0m\ntwo, unended";
        log.push(coloured);
        assert_eq!(log.before_console(1 << 20, false), b"one\ntwo, unended");
        assert_eq!(log.before_console(1 << 20, true), coloured);
        log.push_console(b"INFO: from bazel\n");
        log.push(b"after\n");
        assert_eq!(log.before_console(1 << 20, false), b"one\ntwo, unended");
        assert_eq!(log.before_console(1 << 20, true), coloured);
        assert_eq!(log.before_console(7, false), b"unended", "its newest bytes");
        assert_eq!(
            log.before_console(14, true),
            b"two, unended",
            "from a line's start, no escape cut"
        );
    }
}
