//! The thread between a captured console and wherever it is shown.
//!
//! One reader thread per invocation reads the capture, splits it into records,
//! runs each through an ordered [`LineProcessor`] chain and writes what
//! survives with a blocking write. There is no queue: a slow destination
//! back-pressures the pipe or PTY buffer and, through it, bazel, so memory
//! stays bounded.
//!
//! # Records
//!
//! A record is the bytes up to a `\r`, `\n` or `\r\n`. Bazel's curses UI
//! redraws in place with a bare `\r`, so splitting on `\n` alone would hold
//! the progress region back until the next real newline; a PTY ends every line
//! with `\r\n`, which is one boundary. Each record is written together
//! with its boundary in one write, so a line from another thread cannot land
//! inside it, and the destination is flushed once per read rather than per
//! record (the kernel already batches what one read returns). The exception
//! is a `\r\n` a read splits: the `\r` goes out with its record, and the
//! `\n` with the next read; but a record with no text waits, with its `\r`,
//! for the next read, which says whether it is a blank line.
//!
//! A record a stage hides still forwards its escape sequences
//! ([`super::ansi::controls`]): where curses is shown they are curses moving
//! the cursor over its progress area, and the terminal must end up where
//! bazel thinks it is. Ones that only set a colour and reset it
//! ([`super::ansi::is_neutral`]) are left out, as they would only clutter the
//! next line.
//!
//! # Rendered lines
//!
//! Given [`Shape::lines`], each record is classified, then fed with its
//! boundary to a [`Screen`], and the stages run on the rows it settles, each
//! with the classification of the record that wrote it, ended by `\n`, a
//! status line the screen lets through included.
//!
//! # The end of the console
//!
//! At end of stream the trailing partial record is forwarded, then each
//! stage's [`LineProcessor::finish`] (state it still holds, such as a pending
//! repeat count) and, once every stage has finished, each one's
//! [`LineProcessor::report`].
//!
//! # The raw log
//!
//! Given a [`RawLog`], the thread writes every byte it reads to it before
//! splitting, so the log is the console as bazel wrote it.
//!
//! # A destination that fails
//!
//! Once a write to the destination fails (a closed stderr, `aspect build |
//! head`), the rest of the console is read and discarded rather than ending
//! the thread. A write that would block is not a failure: it is retried.
//! Ending the thread would close the capture under bazel, which a write then
//! kills with `SIGPIPE`, and turn a console nobody reads into a failed build.

use std::io::{ErrorKind, Read, Write};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::ansi;
use super::feed::{Feed, Source};
use super::line::{Classifier, Line};
use super::log::RawLog;
use super::render::{Row, Screen};
use crate::stream::Activity;

const READ_BUF_SIZE: usize = 8 * 1024;

/// The longest incomplete record kept. A producer that never writes a boundary
/// has its bytes passed on as a record of this size, so memory stays bounded.
/// A held record with no text and its `\r` waits one read more past it, as
/// that read settles it.
const MAX_CARRY_BYTES: usize = 1024 * 1024;

/// One stage of the chain, run on the reader thread for every record. Must
/// not block for long, or forwarding stalls; [`super::processors::Interceptor`]
/// is the one stage that blocks, bounded by its timeout.
pub trait LineProcessor: Send {
    /// `record` excludes its boundary byte. `Some(bytes)` forwards those bytes
    /// followed by the original boundary; they may differ from the input and
    /// contain newlines. `None` drops the record and its boundary, though its
    /// escape sequences are still forwarded (see the module docs); return
    /// `Some(vec![])` to keep a blank line instead.
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>>;

    /// Called with what the record is, just before [`LineProcessor::process`]:
    /// classified once for the whole chain, from the record as bazel wrote
    /// it, so every stage agrees on it whatever the stages before hid.
    fn see(&mut self, _line: &Line) {}

    /// Called instead of [`LineProcessor::process`] for a record a stage
    /// before this one hid: what it was, so state that follows the stream
    /// (the body of a message hidden earlier) knows a record went by.
    fn skipped(&mut self, _line: &Line) {}

    /// Called once at end of stream; the bytes are written as is. For a stage
    /// holding state, such as a pending repeat count.
    fn finish(&mut self) -> Option<Vec<u8>> {
        None
    }

    /// Called once after every stage's [`LineProcessor::finish`]; the bytes
    /// are written as is. For what a stage says about the whole console.
    fn report(&mut self) -> Option<Vec<u8>> {
        None
    }
}

/// The running reader thread. Joined after the child has exited.
#[derive(Debug)]
pub struct Forwarder {
    handle: Option<JoinHandle<std::io::Result<()>>>,
    /// Whether the thread is waiting for bazel, busy, or done.
    activity: Activity,
}

impl Forwarder {
    /// Start forwarding `reader` to `forward` through `processors` (an empty
    /// chain forwards every record verbatim). The thread ends when `reader`
    /// returns `Ok(0)`, which for a [`crate::stream::fd::Fd`] is once the
    /// stream is over by its rules, or fails to read.
    pub fn spawn(
        reader: Box<dyn Read + Send>,
        forward: Box<dyn Write + Send>,
        processors: Vec<Box<dyn LineProcessor>>,
    ) -> std::io::Result<Forwarder> {
        Self::spawn_logged(reader, forward, processors, None, Shape::default())
    }

    /// [`Forwarder::spawn`], also writing what is read to `log`, and shaping
    /// what is forwarded past the stages as `shape` says.
    pub fn spawn_logged(
        reader: Box<dyn Read + Send>,
        forward: Box<dyn Write + Send>,
        processors: Vec<Box<dyn LineProcessor>>,
        log: Option<RawLog>,
        shape: Shape,
    ) -> std::io::Result<Forwarder> {
        Self::spawn_reported(reader, forward, processors, log, shape, None)
    }

    /// [`Forwarder::spawn_logged`] with a reader that reports its own waits
    /// to `activity` ([`crate::stream::fd::Fd::reporting`]): the thread then
    /// only marks itself busy and done, so it is never seen waiting while a
    /// read is taking bytes. Without one, it marks itself waiting around each
    /// read.
    pub fn spawn_reported(
        mut reader: Box<dyn Read + Send>,
        forward: Box<dyn Write + Send>,
        processors: Vec<Box<dyn LineProcessor>>,
        mut log: Option<RawLog>,
        shape: Shape,
        activity: Option<Activity>,
    ) -> std::io::Result<Forwarder> {
        let reports = activity.is_some();
        let activity = activity.unwrap_or_default();
        let state = activity.clone();
        let handle = thread::Builder::new()
            .name("bazel-console".into())
            .spawn(move || {
                let screen = shape.lines.map(|every| match &shape.feed {
                    Some(feed) => Screen::new(every).with_feed(feed.clone()),
                    None => Screen::new(every),
                });
                let mut splitter = Splitter::new(
                    processors,
                    Destination::new(forward, shape.strip_colour)
                        .feeding(shape.feed.clone(), shape.hidden)
                        .copying(shape.copy)
                        .holding(shape.hold)
                        .gating(shape.gate.clone()),
                    screen,
                )
                .with_notes(shape.notes.clone());
                let mut buf = [0u8; READ_BUF_SIZE];
                let result = loop {
                    if reports {
                        state.busy();
                    } else {
                        state.waiting();
                    }
                    let read = reader.read(&mut buf);
                    state.busy();
                    let n = match read {
                        Ok(0) => break Ok(()),
                        Ok(n) => n,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => break Err(e),
                    };
                    if let Some(log) = log.as_mut() {
                        log.write(&buf[..n]);
                    }
                    splitter.feed(&buf[..n]);
                };
                splitter.finish();
                if let Some(feed) = &shape.feed {
                    feed.finish(Source::Stderr);
                }
                state.done();
                result
            })?;
        Ok(Forwarder {
            handle: Some(handle),
            activity,
        })
    }

    /// Whether the thread is done: the console has ended and everything it
    /// held is written.
    pub fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    /// Millis the thread has been waiting for bazel to write, 0 while it is
    /// processing, holding or writing what it read (bazel is then blocked on
    /// us, not silent). Polled alongside the child's liveness, a stalled
    /// invocation shows as a live child with a growing idle time.
    pub fn idle_ms(&self) -> u64 {
        self.activity.idle_ms()
    }

    /// `inner`, made to wait before each write until this forwarder has
    /// forwarded what was waiting for it on `fd`, a duplicate of the read end
    /// it follows (only polled, never read). For the client's stdout when it
    /// is a file of its own: bazel writes a test's output there right after
    /// the stderr line above it, which this forwarder takes a detour to
    /// deliver.
    ///
    /// It needs a forwarder started with [`Forwarder::spawn_reported`] on a
    /// reporting [`crate::stream::fd::Fd`], or it is seen waiting during the
    /// read that takes bytes. Only stdout waits: a stderr line bazel wrote
    /// after the stdout held back (a burst, or stdout that fills its pipe)
    /// can still go first, so with separate destinations the order is close,
    /// not exact.
    pub fn ordered_after(&self, inner: Box<dyn Write + Send>, fd: std::os::fd::OwnedFd) -> After {
        After {
            inner,
            gate: self.gate(fd),
        }
    }

    /// [`Forwarder::ordered_after`]'s wait on its own, for a forwarder to run
    /// before it takes what it holds across a write ([`Shape::gate`]).
    pub fn gate(&self, fd: std::os::fd::OwnedFd) -> Gate {
        Gate {
            other: self.activity.clone(),
            fd,
            skip_until: std::sync::Mutex::new(None),
        }
    }

    /// Wait for the thread to end. A second call returns `Ok(())`.
    pub fn join(&mut self) -> std::io::Result<()> {
        match self.handle.take() {
            Some(handle) => handle
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("console forwarder panicked"))),
            None => Ok(()),
        }
    }
}

/// How long an [`After`] stops waiting once a wait ran out: the other
/// forwarder is holding a line (an `intercept` stage) or stalled, and every
/// record of this stream must not pay the full wait.
const AFTER_BACKOFF: Duration = Duration::from_secs(1);

/// The longest an [`After`] holds a write back.
const AFTER_WAIT: Duration = Duration::from_millis(200);

/// See [`Forwarder::ordered_after`].
pub struct After {
    inner: Box<dyn Write + Send>,
    gate: Gate,
}

/// See [`Forwarder::gate`].
pub struct Gate {
    other: Activity,
    fd: std::os::fd::OwnedFd,
    /// No waiting until then: see [`AFTER_BACKOFF`].
    skip_until: std::sync::Mutex<Option<Instant>>,
}

impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gate").finish_non_exhaustive()
    }
}

impl Gate {
    /// Until the other forwarder is waiting with nothing left to read, or has
    /// ended, for at most [`AFTER_WAIT`]: a stalled one must not stall this
    /// stream. A wait that runs out pauses waiting for [`AFTER_BACKOFF`].
    pub fn wait(&self) {
        let mut skip_until = self.skip_until.lock().unwrap_or_else(|p| p.into_inner());
        if skip_until.is_some_and(|t| Instant::now() < t) {
            return;
        }
        let deadline = Instant::now() + AFTER_WAIT;
        loop {
            // Readable first: the other reader marks itself busy before the
            // read that empties the pipe, so seeing it empty and then not
            // busy means its bytes have been forwarded. The other order can
            // see "waiting", then an empty pipe its read just emptied.
            let pending = readable(&self.fd);
            if self.other.is_done() || (!pending && !self.other.is_busy()) {
                return;
            }
            if Instant::now() >= deadline {
                *skip_until = Some(Instant::now() + AFTER_BACKOFF);
                return;
            }
            thread::sleep(AFTER_POLL);
        }
    }
}

/// Whether `fd` has bytes waiting, without reading them.
fn readable(fd: &impl std::os::fd::AsRawFd) -> bool {
    crate::stream::poll_in(fd, Duration::ZERO) & libc::POLLIN != 0
}

impl Write for After {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.gate.wait();
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// How long a destination that would block waits before the next try.
const RETRY_POLL: Duration = Duration::from_millis(10);

/// How often a write waiting on the other forwarder looks again.
const AFTER_POLL: Duration = Duration::from_millis(1);

/// Splits what is read into records and forwards each through the chain.
///
/// `\r\n` is one boundary, not a record ending in `\r` followed by an empty
/// one: a PTY writes every newline that way, and the record goes out with
/// both in one write. When a read ends between the two, the `\r` is forwarded
/// with its record straight away, since a bare one is how curses redraws, and
/// the `\n` arriving next is forwarded only if that record was; but a record
/// with no text (nothing, an escape, spaces) waits with its `\r` for the
/// next read, which says whether it is a blank line or a bare `\r`: a stage
/// tells the two apart ([`Line::returns`]), and there is nothing to show
/// meanwhile. A hidden record forwards neither, only its escape sequences.
struct Splitter {
    carry: Vec<u8>,
    lines: Classifier,
    processors: Vec<Box<dyn LineProcessor>>,
    out: Destination,
    /// Set after a record ending in `\r`: whether that record was dropped.
    after_cr: Option<bool>,
    /// The carry is a record with no text and its `\r`, held for the next
    /// read.
    held_cr: bool,
    /// Rendering curses output to plain lines ([`Screen`]) ahead of the
    /// stages, which then run on the rows it settles.
    screen: Option<Screen>,
    /// Classifies a settled row no record was classified for.
    row_lines: Classifier,
    /// Lines to write between Bazel's ([`Notes`]).
    notes: Option<Notes>,
    /// Whether the last record could be in Bazel's progress area
    /// ([`super::line::Line::drawn`]): no note is written after one.
    last_drawn: bool,
}

impl Splitter {
    fn new(
        processors: Vec<Box<dyn LineProcessor>>,
        out: Destination,
        screen: Option<Screen>,
    ) -> Self {
        Self {
            carry: Vec::with_capacity(READ_BUF_SIZE),
            lines: Classifier::default(),
            processors,
            out,
            after_cr: None,
            held_cr: false,
            screen,
            row_lines: Classifier::default(),
            notes: None,
            last_drawn: true,
        }
    }

    /// Also write the lines `notes` has, between Bazel's.
    fn with_notes(mut self, notes: Option<Notes>) -> Self {
        self.notes = notes;
        self
    }

    /// Write the queued notes, each as a line the stages run on.
    fn write_notes(&mut self) {
        let Some(notes) = self.notes.as_ref().map(Notes::take) else {
            return;
        };
        for note in notes {
            // Its own classifier: Bazel's knows where its progress area is.
            let line = Classifier::default().classify(&note);
            if self.screen.is_some() {
                let mut bytes = note;
                bytes.push(b'\n');
                self.render(&bytes, Some(&line));
            } else {
                self.chain(&note, b"\n", line);
            }
        }
    }

    /// Forward every complete record in `bytes`, keeping the trailing
    /// incomplete one (up to [`MAX_CARRY_BYTES`]), then flush.
    fn feed(&mut self, bytes: &[u8]) {
        let mut carry = std::mem::take(&mut self.carry);
        // The carry holds no boundary but a held record's `\r`, so only it and
        // the new bytes need scanning.
        let scanned = if std::mem::take(&mut self.held_cr) {
            0
        } else {
            carry.len()
        };
        carry.extend_from_slice(bytes);
        let mut start = 0;
        let mut i = scanned;
        while i < carry.len() {
            let b = carry[i];
            if b == b'\n' && start == i && self.after_cr.is_some() {
                if self.after_cr.take() == Some(false) {
                    if self.screen.is_some() {
                        self.render(b"\n", None);
                    } else {
                        self.out.write(b"\n");
                    }
                }
                start = i + 1;
            } else if b == b'\r' && carry.get(i + 1) == Some(&b'\n') {
                self.emit(&carry[start..i], b"\r\n");
                self.after_cr = None;
                i += 1;
                start = i + 1;
            } else if b == b'\r'
                && i + 1 == carry.len()
                && super::line::message_of(&carry[start..i]).trim().is_empty()
            {
                self.held_cr = true;
                break;
            } else if b == b'\n' || b == b'\r' {
                let kept = self.emit(&carry[start..i], &[b]);
                self.after_cr = (b == b'\r').then_some(!kept);
                start = i + 1;
            }
            i += 1;
        }
        carry.drain(..start);
        self.carry = carry;
        if !self.held_cr && self.carry.len() > MAX_CARRY_BYTES {
            self.flush_carry();
        }
        self.out.flush();
    }

    /// End of stream: the trailing partial record, then, when rendering, the
    /// rows the screen still held, then the notes still queued, then each stage's
    /// [`LineProcessor::finish`], then each one's [`LineProcessor::report`].
    fn finish(&mut self) {
        self.flush_carry();
        if let Some(mut screen) = self.screen.take() {
            let rows = screen.finish();
            self.run_rows(rows);
        }
        if self.notes.as_ref().is_some_and(|n| !n.is_empty()) {
            if !self.out.at_line_start {
                self.out.write(b"\n");
            }
            self.write_notes();
        }
        for p in self.processors.iter_mut() {
            if let Some(bytes) = p.finish() {
                self.out.write(&bytes);
            }
        }
        for p in self.processors.iter_mut() {
            if let Some(bytes) = p.report() {
                // On a line of its own, even after a console that ended
                // mid-line.
                if !self.out.at_line_start {
                    self.out.write(b"\n");
                }
                self.out.write(&bytes);
            }
        }
        self.out.flush();
    }

    /// Forward the incomplete record, with no boundary.
    fn flush_carry(&mut self) {
        if std::mem::take(&mut self.held_cr) {
            let record = std::mem::take(&mut self.carry);
            self.emit(&record[..record.len() - 1], b"\r");
        } else if !self.carry.is_empty() {
            let record = std::mem::take(&mut self.carry);
            self.emit(&record, b"");
        }
        self.after_cr = None;
    }

    /// Run one record through the chain and write the result with its
    /// boundary; whether it survived. A record a stage hides leaves its escape
    /// sequences, without the boundary. The chain runs even once the
    /// destination has failed, so observers still see every line.
    fn emit(&mut self, record: &[u8], boundary: &[u8]) -> bool {
        let mut line = self.lines.classify(record);
        // Between a message Bazel ended and its progress area, or the next
        // output's header: a note there is where nothing redraws.
        if !self.last_drawn && (line.drawn || line.opens_output) {
            self.write_notes();
        }
        self.last_drawn = line.drawn || boundary != b"\n" && boundary != b"\r\n";
        if self.screen.is_some() {
            let mut bytes = record.to_vec();
            bytes.extend_from_slice(boundary);
            self.render(&bytes, Some(&line));
            // Kept: a `\n` that follows a `\r` across reads goes to the screen.
            return true;
        }
        line.returns = boundary == b"\r";
        self.chain(record, boundary, line)
    }

    /// Feed `bytes` to the screen, and run the rows it settles through the
    /// chain.
    fn render(&mut self, bytes: &[u8], line: Option<&Line>) {
        let rows = self.screen.as_mut().expect("rendering").feed(bytes, line);
        self.run_rows(rows);
    }

    /// Run settled rows through the chain, as lines, a status line the
    /// screen let through included: a `rewrite` that redacts reaches it too.
    fn run_rows(&mut self, rows: Vec<Row>) {
        for row in rows {
            let line = match row.line {
                Some(line) => line,
                None => self.row_lines.classify(&row.bytes),
            };
            self.chain(&row.bytes, b"\n", line);
        }
    }

    /// Run `record`, classified as `line`, through the chain and write the
    /// result with its boundary; whether it survived. A record a stage hides
    /// leaves its escape sequences: curses' cursor movement on a terminal, a
    /// rendered row's colour changes, which later lines would otherwise
    /// inherit or lose.
    fn chain(&mut self, record: &[u8], boundary: &[u8], line: Line) -> bool {
        let mut current = record.to_vec();
        for i in 0..self.processors.len() {
            let p = &mut self.processors[i];
            p.see(&line);
            match p.process(&current) {
                Some(bytes) => current = bytes,
                None => {
                    for later in &mut self.processors[i + 1..] {
                        later.skipped(&line);
                    }
                    let kept = ansi::controls(&current);
                    if !ansi::is_neutral(&kept) {
                        self.out.write(&kept);
                    }
                    return false;
                }
            }
        }
        current.extend_from_slice(boundary);
        // What curses redraws (the progress area, a spinner's `\r`) is no
        // line of the copy.
        self.out.settled = !line.drawn && !line.returns;
        self.out.write(&current);
        self.out.settled = true;
        true
    }
}

/// What the forwarder does to what the stages pass, before it is written.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    /// Leave colour out: for a destination that renders none, or a user who
    /// asked for none.
    pub strip_colour: bool,
    /// Render Bazel's curses output as plain lines ([`Screen`]), with its
    /// status line at most every so often when `Some(Some(_))`.
    pub lines: Option<Option<std::time::Duration>>,
    /// Also hand what is written, and each progress area the rendering
    /// erases, to this [`Feed`].
    pub feed: Option<Arc<Feed>>,
    /// Write nothing to the destination: what is shown goes to `feed` only.
    pub hidden: bool,
    /// Lines another thread has for the console ([`Notes`]), written as
    /// Bazel's own are, where Bazel is not drawing its progress area.
    pub notes: Option<Notes>,
    /// Also given every settled line written, shown or hidden: the task
    /// history's copy of its output.
    pub copy: Option<fn(&[u8])>,
    /// Held across each write's copy and the write itself (the CLI's
    /// terminal write lock), so nothing comes between the two.
    pub hold: Option<fn() -> Box<dyn std::any::Any>>,
    /// Waited on before each write, ahead of `hold` ([`Forwarder::gate`]):
    /// the wait for the other forwarder holds nothing.
    pub gate: Option<Arc<Gate>>,
}

/// The failed actions' logs the history kept, in the order it kept them:
/// each action's label and, per stream it kept (`stderr`, `stdout`), where.
/// The recap names them under their errors.
#[derive(Debug, Clone, Default)]
pub struct KeptLogs(Arc<std::sync::Mutex<Vec<(String, Vec<(String, String)>)>>>);

impl KeptLogs {
    /// Note `label`'s kept `streams`, as `(stream, path)`.
    pub fn push(&self, label: String, streams: Vec<(String, String)>) {
        self.0.lock().unwrap().push((label, streams));
    }

    /// Each kept action so far.
    pub fn all(&self) -> Vec<(String, Vec<(String, String)>)> {
        self.0.lock().unwrap().clone()
    }
}

/// Lines for the console from another thread (the history's note of where
/// it kept a failed action's log). The forwarder writes each, as a line the
/// stages run on, once Bazel has ended a message and before it draws its
/// progress area again, so a line never lands inside what curses redraws;
/// the rest when the console ends.
#[derive(Debug, Clone, Default)]
pub struct Notes {
    queue: Arc<std::sync::Mutex<std::collections::VecDeque<Vec<u8>>>>,
    /// The `INFO` label's style, the CLI's own (`report::Style::info`):
    /// a note is the CLI's line, so it looks as `diag::info`'s do. Empty
    /// leaves it plain.
    info: String,
}

impl Notes {
    /// Notes whose `INFO` label is drawn in `info` ([`aspect_theme::label_line`]).
    pub fn styled(info: String) -> Notes {
        Notes {
            info,
            ..Notes::default()
        }
    }

    /// Queue `line`, without its newline.
    pub fn push(&self, line: Vec<u8>) {
        self.queue.lock().unwrap().push_back(line);
    }

    fn is_empty(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }

    /// The queued notes, each as the CLI writes its own lines.
    pub fn take(&self) -> Vec<Vec<u8>> {
        let notes: Vec<Vec<u8>> = self.queue.lock().unwrap().drain(..).collect();
        notes.into_iter().map(|note| self.style(note)).collect()
    }

    /// `note` with its `INFO` label styled as the CLI's own are.
    fn style(&self, note: Vec<u8>) -> Vec<u8> {
        match note.strip_prefix(b"INFO:") {
            Some(rest) if !self.info.is_empty() => {
                aspect_theme::label_line(&self.info, "INFO", &String::from_utf8_lossy(rest))
                    .into_bytes()
            }
            _ => note,
        }
    }
}

/// The destination. A write that would block (stderr left nonblocking by
/// another process) is retried; any other failure discards everything after
/// it.
struct Destination {
    inner: Box<dyn Write + Send>,
    copy: Option<fn(&[u8])>,
    /// Whether what is being written is a settled line, for `copy`.
    settled: bool,
    /// Held across a write's copy and the write ([`Shape::hold`]).
    hold: Option<fn() -> Box<dyn std::any::Any>>,
    /// Waited on before a write, ahead of `hold` ([`Shape::gate`]).
    gate: Option<Arc<Gate>>,
    strip_colour: bool,
    feed: Option<Arc<Feed>>,
    hidden: bool,
    /// Whether the last text written (escapes aside) ended a line.
    at_line_start: bool,
    failed: bool,
}

impl Destination {
    fn new(inner: Box<dyn Write + Send>, strip_colour: bool) -> Self {
        Self {
            inner,
            copy: None,
            settled: true,
            hold: None,
            gate: None,
            strip_colour,
            feed: None,
            hidden: false,
            at_line_start: true,
            failed: false,
        }
    }

    /// Also give every settled line written to `copy`, shown or hidden.
    fn copying(mut self, copy: Option<fn(&[u8])>) -> Self {
        self.copy = copy;
        self
    }

    /// Hold `hold`'s guard across each write's copy and the write.
    fn holding(mut self, hold: Option<fn() -> Box<dyn std::any::Any>>) -> Self {
        self.hold = hold;
        self
    }

    /// Wait on `gate` before each write, ahead of the hold.
    fn gating(mut self, gate: Option<Arc<Gate>>) -> Self {
        self.gate = gate;
        self
    }

    /// Also write to `feed`, and with `hidden` only there.
    fn feeding(mut self, feed: Option<Arc<Feed>>, hidden: bool) -> Self {
        self.feed = feed;
        self.hidden = hidden;
        self
    }

    fn write(&mut self, bytes: &[u8]) {
        // Only a write that reaches the destination waits its turn behind the
        // other forwarder: a hidden one is fed and copied, never shown.
        if let Some(gate) = self.gate.as_ref().filter(|_| !self.hidden) {
            gate.wait();
        }
        let _held = self.hold.map(|hold| hold());
        if let Some(&last) = ansi::strip(bytes).last() {
            self.at_line_start = last == b'\n' || last == b'\r';
        }
        if let Some(feed) = &self.feed {
            feed.write(Source::Stderr, bytes);
        }
        if let (Some(copy), true) = (self.copy, self.settled) {
            copy(bytes);
        }
        if self.hidden {
            return;
        }
        let stripped = ansi::without(bytes, self.strip_colour, false);
        let mut bytes = &stripped[..];
        while !self.failed && !bytes.is_empty() {
            match self.inner.write(bytes) {
                Ok(0) => self.fail(ErrorKind::WriteZero.into()),
                Ok(n) => bytes = &bytes[n..],
                Err(e) => self.retry_or_fail(e),
            }
        }
    }

    fn flush(&mut self) {
        // Held as a write is: what the flush writes is a write.
        let _held = self.hold.map(|hold| hold());
        while !self.failed {
            match self.inner.flush() {
                Ok(()) => return,
                Err(e) => self.retry_or_fail(e),
            }
        }
    }

    fn retry_or_fail(&mut self, e: std::io::Error) {
        match e.kind() {
            ErrorKind::Interrupted => {}
            ErrorKind::WouldBlock => thread::sleep(RETRY_POLL),
            _ => self.fail(e),
        }
    }

    fn fail(&mut self, e: std::io::Error) {
        tracing::debug!("console destination failed ({e}); discarding the rest");
        self.failed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::SharedSink;
    use std::sync::atomic::Ordering;

    fn run_with(input: &[u8], processors: Vec<Box<dyn LineProcessor>>) -> Vec<u8> {
        let sink = SharedSink::default();
        let mut forwarder = Forwarder::spawn(
            Box::new(std::io::Cursor::new(input.to_vec())),
            Box::new(sink.clone()),
            processors,
        )
        .unwrap();
        forwarder.join().unwrap();
        sink.bytes()
    }

    fn run(input: &[u8]) -> Vec<u8> {
        run_with(input, vec![])
    }

    struct Upcase;
    impl LineProcessor for Upcase {
        fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
            Some(record.to_ascii_uppercase())
        }
    }

    /// Drops a record equal to the one before it.
    #[derive(Default)]
    struct ConsecutiveDedup {
        prev: Option<Vec<u8>>,
    }
    impl LineProcessor for ConsecutiveDedup {
        fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
            if self.prev.as_deref() == Some(record) {
                return None;
            }
            self.prev = Some(record.to_vec());
            Some(record.to_vec())
        }
    }

    #[test]
    fn an_empty_chain_forwards_bytes_verbatim() {
        for input in [
            &b"line one\nline two\n"[..],
            b"[1 / 9] Building\r[2 / 9] Building\r",
            b"progress\rprogress\rdone\n",
            b"crlf\r\nline\r\n",
            b"no trailing newline",
            b"",
        ] {
            assert_eq!(run(input), input, "{:?}", String::from_utf8_lossy(input));
        }
    }

    #[test]
    fn a_record_past_the_carry_cap_is_forwarded_through_the_chain() {
        let big = vec![b'x'; MAX_CARRY_BYTES + 4096];
        assert_eq!(run(&big), big);
        assert_eq!(
            run_with(&big, vec![Box::new(Upcase)]),
            big.to_ascii_uppercase()
        );
    }

    #[test]
    fn a_dropped_record_takes_its_boundary_with_it() {
        let out = run_with(b"a\na\nb\n", vec![Box::new(ConsecutiveDedup::default())]);
        assert_eq!(out, b"a\nb\n");
    }

    #[test]
    fn an_empty_result_keeps_a_blank_line() {
        struct Blank;
        impl LineProcessor for Blank {
            fn process(&mut self, _record: &[u8]) -> Option<Vec<u8>> {
                Some(vec![])
            }
        }
        assert_eq!(run_with(b"a\rb\n", vec![Box::new(Blank)]), b"\r\n");
    }

    #[test]
    fn stages_run_in_order_on_each_others_output() {
        let out = run_with(
            b"a\nA\nb\n",
            vec![Box::new(Upcase), Box::new(ConsecutiveDedup::default())],
        );
        assert_eq!(out, b"A\nB\n");
    }

    /// Under a PTY a message carries curses' cursor movement; hiding the
    /// message must not lose it.
    #[test]
    fn a_dropped_record_leaves_its_escape_sequences() {
        let out = run_with(
            b"\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mx\r\n\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mx\r\nnext\r\n",
            vec![Box::new(ConsecutiveDedup::default())],
        );
        assert_eq!(
            out,
            b"\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mx\r\n\x1b[1A\x1b[K\x1b[35m\x1b[0mnext\r\n"
        );
    }

    /// A console that ends mid-line still gets its report on a line of its
    /// own; one that ends with a newline and a colour reset gets no blank.
    #[test]
    fn a_report_starts_on_a_line_of_its_own() {
        struct Report;
        impl LineProcessor for Report {
            fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                Some(record.to_vec())
            }
            fn report(&mut self) -> Option<Vec<u8>> {
                Some(b"report\n".to_vec())
            }
        }
        assert_eq!(
            run_with(b"partial", vec![Box::new(Report)]),
            b"partial\nreport\n"
        );
        assert_eq!(
            run_with(b"done\r\n\x1b[0m", vec![Box::new(Report)]),
            b"done\r\n\x1b[0mreport\n"
        );
    }

    #[test]
    fn reports_follow_every_finish() {
        struct Both(&'static str);
        impl LineProcessor for Both {
            fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                Some(record.to_vec())
            }
            fn finish(&mut self) -> Option<Vec<u8>> {
                Some(format!("finish {}\n", self.0).into_bytes())
            }
            fn report(&mut self) -> Option<Vec<u8>> {
                Some(format!("report {}\n", self.0).into_bytes())
            }
        }
        let out = run_with(b"x\n", vec![Box::new(Both("a")), Box::new(Both("b"))]);
        assert_eq!(out, b"x\nfinish a\nfinish b\nreport a\nreport b\n");
    }

    #[test]
    fn a_plain_destination_gets_no_escape_sequences() {
        let sink = SharedSink::default();
        let mut forwarder = Forwarder::spawn_logged(
            Box::new(std::io::Cursor::new(
                b"\x1b[35mWARNING: \x1b[0mx\n\x1b[35mWARNING: \x1b[0mx\ny\n".to_vec(),
            )),
            Box::new(sink.clone()),
            vec![Box::new(ConsecutiveDedup::default())],
            None,
            Shape {
                strip_colour: true,
                ..Shape::default()
            },
        )
        .unwrap();
        forwarder.join().unwrap();
        assert_eq!(sink.bytes(), b"WARNING: x\ny\n");
    }

    /// A note queued before the console ends is written as a line, after
    /// Bazel's; one queued while Bazel writes a message lands after it,
    /// before the progress area.
    #[test]
    fn notes_are_written_between_bazels_lines() {
        let sink = SharedSink::default();
        let notes = Notes::default();
        notes.push(b"INFO: //a:b Genrule stderr: /h/x.stderr".to_vec());
        let mut forwarder = Forwarder::spawn_logged(
            Box::new(std::io::Cursor::new(b"ERROR: x failed\n  why\n".to_vec())),
            Box::new(sink.clone()),
            vec![],
            None,
            Shape {
                notes: Some(notes.clone()),
                ..Shape::default()
            },
        )
        .unwrap();
        forwarder.join().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&sink.bytes()),
            "ERROR: x failed\n  why\nINFO: //a:b Genrule stderr: /h/x.stderr\n"
        );
    }

    /// Hidden, nothing reaches the destination and the feed gets the lines.
    #[test]
    fn a_hidden_destination_gets_nothing_and_the_feed_every_line() {
        let sink = SharedSink::default();
        let feed = Arc::new(Feed::default());
        let mut forwarder = Forwarder::spawn_logged(
            Box::new(std::io::Cursor::new(
                b"\x1b[35mWARNING: \x1b[0mx\ny\n".to_vec(),
            )),
            Box::new(sink.clone()),
            vec![],
            None,
            Shape {
                lines: Some(None),
                feed: Some(feed.clone()),
                hidden: true,
                notes: None,
                ..Shape::default()
            },
        )
        .unwrap();
        forwarder.join().unwrap();
        assert_eq!(sink.bytes(), b"");
        assert_eq!(feed.drain().lines, vec!["WARNING: x", "y"]);
    }

    /// A PTY's `\r\n` goes out with its record in one write, so a line
    /// another thread writes cannot land between the two.
    #[test]
    fn crlf_is_written_with_its_record() {
        #[derive(Clone, Default)]
        struct Writes(std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>);
        impl Write for Writes {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().push(buf.to_vec());
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let writes = Writes::default();
        let mut forwarder = Forwarder::spawn(
            Box::new(std::io::Cursor::new(b"one\r\ntwo\r\n".to_vec())),
            Box::new(writes.clone()),
            vec![],
        )
        .unwrap();
        forwarder.join().unwrap();
        assert_eq!(
            *writes.0.lock().unwrap(),
            [b"one\r\n".to_vec(), b"two\r\n".to_vec()]
        );
    }

    #[test]
    fn the_raw_log_holds_the_console_as_read() {
        let dir = tempfile::tempdir().unwrap();
        let log =
            RawLog::create(&crate::console::log::Place::In(dir.path().to_path_buf())).unwrap();
        let path = log.path().to_path_buf();
        let sink = SharedSink::default();
        let input = b"a\na\nb\r\n".to_vec();
        let mut forwarder = Forwarder::spawn_logged(
            Box::new(std::io::Cursor::new(input.clone())),
            Box::new(sink.clone()),
            vec![Box::new(ConsecutiveDedup::default())],
            Some(log),
            Shape::default(),
        )
        .unwrap();
        forwarder.join().unwrap();
        assert_eq!(sink.bytes(), b"a\nb\r\n");
        assert_eq!(std::fs::read(path).unwrap(), input);
    }

    #[test]
    fn finish_runs_at_end_of_stream() {
        #[derive(Default)]
        struct CountOnly(usize);
        impl LineProcessor for CountOnly {
            fn process(&mut self, _record: &[u8]) -> Option<Vec<u8>> {
                self.0 += 1;
                None
            }
            fn finish(&mut self) -> Option<Vec<u8>> {
                Some(format!("{} lines\n", self.0).into_bytes())
            }
        }
        let out = run_with(b"a\nb\nc", vec![Box::new(CountOnly::default())]);
        assert_eq!(out, b"3 lines\n");
    }

    #[test]
    fn a_read_error_ends_the_thread_with_that_error() {
        struct Failing(bool);
        impl Read for Failing {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if std::mem::replace(&mut self.0, true) {
                    return Err(std::io::Error::other("boom"));
                }
                buf[..7].copy_from_slice(b"partial");
                Ok(7)
            }
        }
        let sink = SharedSink::default();
        let mut forwarder =
            Forwarder::spawn(Box::new(Failing(false)), Box::new(sink.clone()), vec![]).unwrap();
        assert_eq!(forwarder.join().unwrap_err().to_string(), "boom");
        assert_eq!(
            sink.bytes(),
            b"partial",
            "the pending record is flushed first"
        );
        assert!(
            forwarder.join().is_ok(),
            "a second join finds nothing to join"
        );
    }

    /// A closed stderr must not end the thread: bazel would lose its reader,
    /// and the build its status. The reader is drained to the end and the
    /// chain still sees every record.
    #[test]
    fn a_failing_destination_is_discarded_and_the_input_drained() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        #[derive(Clone, Default)]
        struct Count(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl LineProcessor for Count {
            fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Some(record.to_vec())
            }
        }
        let count = Count::default();
        let input: Vec<u8> = (0..10_000)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();
        let mut forwarder = Forwarder::spawn(
            Box::new(std::io::Cursor::new(input)),
            Box::new(Closed),
            vec![Box::new(count.clone())],
        )
        .unwrap();
        forwarder
            .join()
            .expect("a failed destination is not an error");
        assert_eq!(count.0.load(Ordering::SeqCst), 10_000);
    }

    /// Records `(record, boundary)` pairs as the chain sees them.
    #[derive(Clone, Default)]
    struct Seen(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl LineProcessor for Seen {
        fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
            let text = String::from_utf8_lossy(record).into_owned();
            self.0.lock().unwrap().push(text);
            (record != b"secret").then(|| record.to_vec())
        }
    }

    /// Feed `chunks` as separate reads.
    fn run_chunks(chunks: &[&[u8]], processors: Vec<Box<dyn LineProcessor>>) -> Vec<u8> {
        struct Chunks(Vec<Vec<u8>>);
        impl Read for Chunks {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let chunk = self.0.remove(0);
                buf[..chunk.len()].copy_from_slice(&chunk);
                Ok(chunk.len())
            }
        }
        let sink = SharedSink::default();
        let reader = Chunks(chunks.iter().map(|c| c.to_vec()).collect());
        let mut forwarder =
            Forwarder::spawn(Box::new(reader), Box::new(sink.clone()), processors).unwrap();
        forwarder.join().unwrap();
        sink.bytes()
    }

    /// A PTY's `\r\n` is one boundary: the chain sees no empty record, and a
    /// dropped line leaves no blank one, even when a read splits the pair.
    #[test]
    fn crlf_is_one_boundary() {
        for chunks in [
            &[&b"keep\r\nsecret\r\nlast\r\n"[..]][..],
            &[b"keep\r", b"\nsecret\r", b"\nlast\r\n"],
        ] {
            let seen = Seen::default();
            let out = run_chunks(chunks, vec![Box::new(seen.clone())]);
            assert_eq!(out, b"keep\r\nlast\r\n");
            assert_eq!(*seen.0.lock().unwrap(), ["keep", "secret", "last"]);
        }
    }

    /// A `\r` with nothing before it at the end of a read waits for the next:
    /// half of a blank line is not a tool's bare `\r`, so a run of repeats
    /// before it is counted before the blank line, not after.
    #[test]
    fn a_blank_lines_half_waits_for_the_next_read() {
        let repeats = super::super::processors::CollapseRepeats::default().under_pty(true);
        let out = run_chunks(
            &[b"wait\r\nwait\r\nwait\r\n\r", b"\nwait\r\n"],
            vec![Box::new(repeats)],
        );
        assert_eq!(
            String::from_utf8_lossy(&out),
            "wait\r\n(last line repeated 2 more times)\n\r\nwait\r\n"
        );
    }

    /// So does one past the carry cap: it waits one read more, not the cap.
    #[test]
    fn a_held_blank_line_past_the_carry_cap_waits_too() {
        let blank = b"\x1b[K".repeat(MAX_CARRY_BYTES / 3 + 10);
        let mut first = b"wait\r\nwait\r\nwait\r\n".to_vec();
        first.extend(&blank);
        first.push(b'\r');
        let mut chunks: Vec<&[u8]> = first.chunks(8190).collect();
        chunks.push(b"\nwait\r\n");
        let repeats = super::super::processors::CollapseRepeats::default().under_pty(true);
        let out = run_chunks(&chunks, vec![Box::new(repeats)]);
        let mut want = b"wait\r\n(last line repeated 2 more times)\n".to_vec();
        want.extend(&blank);
        want.extend(b"\r\nwait\r\n");
        assert!(
            out == want,
            "the count before the blank line, and no extra one"
        );
    }

    /// So does one whose line has only an escape or spaces on it.
    #[test]
    fn a_blank_lines_half_with_an_escape_waits_too() {
        for blank in ["\x1b[0m", "\x1b[K", "  "] {
            let repeats = super::super::processors::CollapseRepeats::default().under_pty(true);
            let first = format!("wait\r\nwait\r\nwait\r\n{blank}\r");
            let out = run_chunks(&[first.as_bytes(), b"\nwait\r\n"], vec![Box::new(repeats)]);
            assert_eq!(
                String::from_utf8_lossy(&out),
                format!("wait\r\n(last line repeated 2 more times)\n{blank}\r\nwait\r\n"),
                "{blank:?}"
            );
        }
    }

    #[test]
    fn a_bare_cr_is_forwarded_with_its_read() {
        let seen = Seen::default();
        let out = run_chunks(
            &[b"[1 / 9]\r", b"[2 / 9]\r\r"],
            vec![Box::new(seen.clone())],
        );
        assert_eq!(out, b"[1 / 9]\r[2 / 9]\r\r");
        assert_eq!(*seen.0.lock().unwrap(), ["[1 / 9]", "[2 / 9]", ""]);
    }

    /// A destination left nonblocking by someone else says `WouldBlock`; that
    /// is a retry, not a reason to discard the console.
    #[test]
    fn a_destination_that_would_block_is_retried() {
        #[derive(Clone, Default)]
        struct Flaky(SharedSink, std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl Write for Flaky {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                if self.1.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) {
                    return Err(std::io::ErrorKind::WouldBlock.into());
                }
                self.0.write(&buf[..1])
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let flaky = Flaky::default();
        let mut forwarder = Forwarder::spawn(
            Box::new(std::io::Cursor::new(b"one\ntwo\n".to_vec())),
            Box::new(flaky.clone()),
            vec![],
        )
        .unwrap();
        forwarder.join().unwrap();
        assert_eq!(flaky.0.bytes(), b"one\ntwo\n");
    }

    /// A stage holding a line is the forwarder stalling bazel, not bazel
    /// going quiet.
    #[test]
    fn idle_time_is_zero_while_a_stage_holds_a_line() {
        struct Hold(std::sync::mpsc::Receiver<()>);
        impl LineProcessor for Hold {
            fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                let _ = self.0.recv();
                Some(record.to_vec())
            }
        }
        let (release, held) = std::sync::mpsc::channel();
        let (r, mut w) = std::io::pipe().unwrap();
        let mut forwarder = Forwarder::spawn(
            Box::new(r),
            Box::new(std::io::sink()),
            vec![Box::new(Hold(held))],
        )
        .unwrap();
        w.write_all(b"held\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(forwarder.idle_ms(), 0);
        release.send(()).unwrap();
        drop(w);
        forwarder.join().unwrap();
    }

    /// A write to an `After` destination waits until the other forwarder has
    /// passed on what was waiting for it.
    #[test]
    fn an_after_destination_waits_for_the_other_stream() {
        struct Slow;
        impl LineProcessor for Slow {
            fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                std::thread::sleep(std::time::Duration::from_millis(50));
                Some(record.to_vec())
            }
        }
        let order = SharedSink::default();
        let (err_r, mut err_w) = std::io::pipe().unwrap();
        let probe = std::os::fd::OwnedFd::from(err_r.try_clone().unwrap());
        let mut err = Forwarder::spawn(
            Box::new(err_r),
            Box::new(order.clone()),
            vec![Box::new(Slow)],
        )
        .unwrap();
        let (out_r, mut out_w) = std::io::pipe().unwrap();
        let mut out = Forwarder::spawn(
            Box::new(out_r),
            Box::new(err.ordered_after(Box::new(order.clone()), probe)),
            vec![],
        )
        .unwrap();
        err_w.write_all(b"header\n").unwrap();
        out_w.write_all(b"output\n").unwrap();
        drop((err_w, out_w));
        err.join().unwrap();
        out.join().unwrap();
        assert_eq!(order.text(), "header\noutput\n");
    }

    /// Once the other forwarder has ended, or while it holds a line, writes
    /// go through without paying the wait each time.
    #[test]
    fn an_after_destination_does_not_wait_on_an_ended_or_stuck_stream() {
        for hold in [false, true] {
            struct Hold(std::sync::mpsc::Receiver<()>);
            impl LineProcessor for Hold {
                fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                    let _ = self.0.recv();
                    Some(record.to_vec())
                }
            }
            let (release, held) = std::sync::mpsc::channel();
            let (err_r, mut err_w) = std::io::pipe().unwrap();
            let probe = std::os::fd::OwnedFd::from(err_r.try_clone().unwrap());
            let mut err = Forwarder::spawn(
                Box::new(err_r),
                Box::new(std::io::sink()),
                vec![Box::new(Hold(held))],
            )
            .unwrap();
            if hold {
                err_w.write_all(b"held\n").unwrap();
            } else {
                release.send(()).unwrap();
                drop(err_w);
                err.join().unwrap();
                err_w = std::io::pipe().unwrap().1;
            }
            let (out_r, mut out_w) = std::io::pipe().unwrap();
            let sink = SharedSink::default();
            let mut out = Forwarder::spawn(
                Box::new(out_r),
                Box::new(err.ordered_after(Box::new(sink.clone()), probe)),
                vec![],
            )
            .unwrap();
            let start = Instant::now();
            for i in 0..50 {
                out_w.write_all(format!("line {i}\n").as_bytes()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            drop(out_w);
            out.join().unwrap();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(3),
                "hold: {hold}: {:?}",
                start.elapsed()
            );
            assert_eq!(sink.text().lines().count(), 50);
            drop(release);
            drop(err_w);
            err.join().unwrap();
        }
    }

    #[test]
    fn idle_time_grows_while_nothing_arrives() {
        let (r, _w) = std::io::pipe().unwrap();
        let forwarder = Forwarder::spawn(Box::new(r), Box::new(std::io::sink()), vec![]).unwrap();
        // Measured from when the thread first waits, which on a loaded
        // machine may be a while after the spawn.
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while forwarder.idle_ms() < 50 {
            assert!(Instant::now() < deadline, "idle time never grew");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
