//! `std.term`: the terminal, for a feature that draws the whole screen.
//!
//! `ctx.std.term.take()` hands out the controlling terminal (`/dev/tty`) as a
//! [`Screen`]: the alternate screen, the cursor hidden, lines not wrapped (a
//! row too wide is cut at the edge, rather than scrolling the screen), keys
//! read one at a time without echo. Ctrl+C still interrupts, so the CLI's cancel path stays
//! the way out of a stuck screen, unless the feature reads it as a key
//! (`screen.interrupts(False)`). The mouse is the terminal's (selection and
//! links its own) unless the feature takes it (`screen.mouse(True)`: the
//! wheel, and the left button's presses, drags and releases, as keys),
//! or asks for the wheel as arrow keys with the mouse still the terminal's
//! (`screen.wheel_keys(True)`). While it is held, nothing is drawn over
//! the screen: the CLI's own output (its lines, `print()`, diagnostics, the
//! AXL streams, the build console) is routed to it ([`route`]), and what
//! else reaches fds 1 and 2 (a stray writer, a child that inherited them)
//! is captured. `screen.output()` hands both to the feature, and
//! `release()` puts the terminal back and writes them to the scrollback
//! (the newest 8 MiB of each stream, or the task's `output.ansi` dumped),
//! so what was printed while the screen was up is not lost.
//!
//! One screen at a time, process-wide. [`release_all`] restores a screen
//! still held when the CLI's run returns and on its forced exit; a panic
//! that ends the process releases it too (`install_panic_hook`), as does
//! a crash (`restore_terminal_after_crash`), and a build with frame hooks
//! whose hook or wait fails;
//! a process killed outright leaves the terminal as the screen had it
//! (`reset` fixes it). The replay writes stdout's capture, then stderr's, so
//! the order between the two is lost.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::Duration;

use allocative::Allocative;
use bazel::console::feed::{Feed, Source};
use derive_more::Display;
use nix::sys::termios::{self, InputFlags, LocalFlags, SetArg, SpecialCharacterIndices, Termios};
use starlark::StarlarkResultExt;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{self, NoSerialize, ProvidesStaticType, ValueLike, starlark_value};

/// Captured output kept for the replay; past it the oldest bytes go.
const MAX_CAPTURED_BYTES: usize = 8 << 20;

/// The screen held now, if any.
static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// The generation the next `take()` gets.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

struct Held {
    /// Which `take()` this is: a [`Screen`] acts only on its own.
    generation: u64,
    /// Whether `take()` entered the alternate screen, to leave it on release.
    alt_screen: bool,
    tty: File,
    saved_termios: Termios,
    /// The modes the screen reads keys in, for a resume to set again.
    cbreak: Termios,
    /// The process's stdout and stderr, captured; None while suspended.
    stdout: Option<Redirect>,
    stderr: Option<Redirect>,
    /// The last frame written, drawn again on a resume at the size it was
    /// drawn at (`frame_size`).
    last_frame: Vec<u8>,
    frame_size: Option<(i32, i32)>,
    /// Whether `mouse(True)` took the mouse, for a resume to take it again.
    mouse_on: bool,
    /// Given back for a prompt or an interactive child ([`suspend`]).
    suspended: bool,
    /// How many [`suspend`]s are outstanding: the last [`resume`] takes the
    /// screen back, so two interactive children at once share one suspend.
    suspends: usize,
    capture: Arc<Feed>,
    /// What the task wrote before Bazel's console began, for `earlier()`.
    earlier: Vec<u8>,
    keys: Keys,
}

/// One of the process's descriptors sent into a pipe a thread reads.
struct Redirect {
    fd: RawFd,
    /// The descriptor as it was, put back by [`Redirect::restore`].
    saved: OwnedFd,
    /// Hands back the pipe's read end when stopped with it still open.
    reader: Option<JoinHandle<Option<File>>>,
    stop: Arc<AtomicBool>,
    /// What was written to it, for the replay.
    written: Arc<Mutex<Vec<u8>>>,
    /// The pipe's read end a [`Redirect::drain`] got back, still open.
    left: Option<File>,
}

impl Redirect {
    /// Send `fd` into a pipe whose reader keeps what is written, for the
    /// replay, and hands it to `capture` as lines.
    /// With `copies` (a screen of its own's), what it reads is copied to
    /// the task's output files, all of it, as it is drained on release too.
    fn new(
        fd: RawFd,
        capture: Arc<Feed>,
        source: Source,
        copies: bool,
    ) -> std::io::Result<Redirect> {
        let saved = dup(fd)?;
        // Closed on exec as it is made (`pipe2` on Linux; macOS sets it just
        // after): only `fd` is for children (`dup2` leaves it inheritable).
        let (read, write) = std::io::pipe()?;
        let (read, write) = (OwnedFd::from(read), OwnedFd::from(write));
        dup2(write.as_raw_fd(), fd)?;
        drop(write);
        let stop = Arc::new(AtomicBool::new(false));
        let written = Arc::new(Mutex::new(Vec::new()));
        let spawned = {
            let (stop, written) = (stop.clone(), written.clone());
            let mut file = File::from(read);
            std::thread::Builder::new()
                .name("term-capture".into())
                .spawn(move || {
                    let mut buf = [0u8; 8192];
                    // Once stopped, what is left is read for this long at
                    // most: a child writing without pause cannot hold up the
                    // release, and gets the rest copied through after it.
                    let mut drain_until = None;
                    loop {
                        let stopping = stop.load(Ordering::SeqCst);
                        if stopping {
                            let until = *drain_until
                                .get_or_insert_with(|| std::time::Instant::now() + DRAIN_BUDGET);
                            if std::time::Instant::now() >= until {
                                return Some(file);
                            }
                        }
                        let wait = if stopping {
                            Duration::ZERO
                        } else {
                            Duration::from_millis(20)
                        };
                        if !pipe_readable(file.as_raw_fd(), wait) {
                            if stopping {
                                // A child may still hold the write end:
                                // `restore` keeps the pipe open for it.
                                return Some(file);
                            }
                            continue;
                        }
                        match file.read(&mut buf) {
                            Ok(0) => return None,
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                            // Read again after a pause: stopping reading would
                            // fill the pipe and block every writer to it.
                            Err(_) => std::thread::sleep(Duration::from_millis(10)),
                            Ok(n) => {
                                keep(&mut written.lock().unwrap(), &buf[..n]);
                                if copies {
                                    crate::out::copy_only(&buf[..n]);
                                }
                                capture.write(source, &buf[..n]);
                            }
                        }
                    }
                })
        };
        match spawned {
            Ok(reader) => Ok(Redirect {
                fd,
                saved,
                reader: Some(reader),
                stop,
                written,
                left: None,
            }),
            Err(e) => {
                let _ = dup2(saved.as_raw_fd(), fd);
                Err(e)
            }
        }
    }

    /// Put the descriptor back, and what was written to it while captured:
    /// stdout's to stdout, stderr's to stderr.
    ///
    /// A child spawned while captured may still hold the pipe: what it
    /// writes from here on is copied through to the descriptor by a detached
    /// thread until it closes, so it neither blocks nor dies of SIGPIPE.
    fn restore(self) {
        self.put_back();
        self.finish(true);
    }

    /// Point the descriptor at what it was again: quick, so every descriptor
    /// is back before any replay waits on a slow destination.
    fn put_back(&self) {
        let _ = dup2(self.saved.as_raw_fd(), self.fd);
    }

    /// Stop the reader once it has read (and, copying, copied) what the pipe
    /// holds; after [`Redirect::put_back`]. [`Redirect::finish`] does it too.
    fn drain(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(reader) = self.reader.take() {
            self.left = reader.join().ok().flatten();
        }
    }

    /// Stop capturing, replay what was written, and pass on what a child
    /// still holding the pipe writes, whole lines at a time, to `fd` as it is
    /// then (a capture again once a screen is taken back); after
    /// [`Redirect::put_back`].
    fn finish(mut self, replay: bool) {
        self.drain();
        let open = self.left.take();
        let written = std::mem::take(&mut *self.written.lock().unwrap());
        let fd = self.fd;
        let mut out = File::from(self.saved);
        if replay {
            let _turn = stream_write_bounded(fd);
            let replayed = newest(&written);
            write_patiently(&mut out, replayed);
            axl_types::terminal::wrote(fd, replayed);
        }
        if let Some(mut pipe) = open {
            let _ = std::thread::Builder::new()
                .name("term-capture-after".into())
                .spawn(move || {
                    // Whole lines, each run of them one unit on the target;
                    // an unended tail waits for its line's end or the pipe's.
                    // To fd 1 or 2 as it is now ([`write_std_unit`]): the
                    // terminal while the screen is given back, a capture again
                    // once it is taken back, which then copies them; else
                    // copied here. Any other descriptor (a test's) gets them
                    // where it was.
                    let mut buf = [0u8; 8192];
                    let mut pending = Vec::new();
                    let std_stream = fd == 1 || fd == 2;
                    let mut put = |bytes: &[u8]| {
                        if !std_stream {
                            let _turn = stream_write_bounded(fd);
                            write_patiently(&mut out, bytes);
                            return;
                        }
                        if let Ok(false) = write_std_unit(fd == 2, bytes) {
                            crate::out::copy_only(bytes);
                        }
                    };
                    loop {
                        match pipe.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                pending.extend_from_slice(&buf[..n]);
                                if let Some(end) = pending.iter().rposition(|&b| b == b'\n') {
                                    put(&pending[..=end]);
                                    pending.drain(..=end);
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(_) => break,
                        }
                    }
                    if !pending.is_empty() {
                        put(&pending);
                    }
                });
        }
    }
}

/// How long a stopped capture reads what is left before handing the pipe on.
const DRAIN_BUDGET: Duration = Duration::from_millis(100);

/// How long moving fds 1 and 2 in or out of a capture waits for a writer to
/// finish its unit: one writing does in far less, and one stalled on a
/// destination nobody reads is not waited on.
const SWITCH_WAIT: Duration = Duration::from_millis(100);

/// How long [`write_patiently`] waits on a destination that takes nothing.
const STALL_LIMIT: Duration = Duration::from_secs(2);

/// The most [`write_patiently`] writes at once: what a pipe takes whole once
/// it is writable, so a blocking descriptor cannot stall in a write.
const WRITE_CHUNK: usize = 512;

/// Write all of `bytes` to `out`, a chunk at a time once it is writable;
/// an error, or no progress for [`STALL_LIMIT`] (a pipe nobody reads, a
/// terminal stopped by Ctrl+S), drops the rest. Bounded either way, blocking
/// descriptor or not.
fn write_patiently(out: &mut File, mut bytes: &[u8]) {
    let mut stalled_since = std::time::Instant::now();
    while !bytes.is_empty() {
        let left = STALL_LIMIT.saturating_sub(stalled_since.elapsed());
        if left.is_zero() || !writable(out.as_raw_fd(), left) {
            return;
        }
        let chunk = &bytes[..bytes.len().min(WRITE_CHUNK)];
        match out.write(chunk) {
            Ok(0) => return,
            Ok(n) => {
                bytes = &bytes[n..];
                stalled_since = std::time::Instant::now();
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return,
        }
    }
}

/// Append `bytes` to `buf`, keeping at least the newest
/// [`MAX_CAPTURED_BYTES`] and at most twice that, so trimming is rare.
fn keep(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(bytes);
    if buf.len() > 2 * MAX_CAPTURED_BYTES {
        let cut = buf.len() - MAX_CAPTURED_BYTES;
        buf.drain(..cut);
    }
}

/// The newest [`MAX_CAPTURED_BYTES`] of `buf`, for the replay.
fn newest(buf: &[u8]) -> &[u8] {
    &buf[buf.len().saturating_sub(MAX_CAPTURED_BYTES)..]
}

/// The size a held screen reports when its terminal does not say.
const FALLBACK_SIZE: (i32, i32) = (80, 24);

/// The controlling terminal's size, held or not; None without one, or,
/// unheld, while this process is in the background.
fn terminal_size() -> Option<(i32, i32)> {
    if let Some(h) = HELD.lock().unwrap().as_ref() {
        return size_of(h.tty.as_raw_fd());
    }
    File::open("/dev/tty")
        .ok()
        .filter(|tty| in_foreground(tty.as_raw_fd()))
        .and_then(|tty| size_of(tty.as_raw_fd()))
}

/// The terminal's size as `(columns, rows)`, from `fd`.
fn size_of(fd: RawFd) -> Option<(i32, i32)> {
    bazel::console::capture::window_size(fd)
        .filter(|ws| ws.ws_col > 0 && ws.ws_row > 0)
        .map(|ws| (ws.ws_col as i32, ws.ws_row as i32))
}

/// Whether the pipe `fd` has bytes to read within `timeout`.
pub(crate) fn pipe_readable(fd: RawFd, timeout: Duration) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: one pollfd, owned on the stack.
    let n = unsafe { libc::poll(&mut pfd, 1, ms) };
    n > 0 && pfd.revents & (libc::POLLIN | libc::POLLHUP) != 0
}

/// Whether a terminal read or write failed because the terminal is gone
/// (closed: a hangup): the screen's `key()` then reads as no key, and its
/// `write()` and mode changes (`mouse()`, `wheel_keys()`, `interrupts()`)
/// are dropped, so a task finds out through its cancellation (the hangup's
/// SIGHUP) rather than an error mid-frame.
fn hung_up(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EIO) | Some(libc::ENXIO))
}

/// A screen method's terminal write or mode change, done; one a terminal
/// gone refused ([`hung_up`]) as well.
fn gone_is_fine(done: std::io::Result<()>) -> anyhow::Result<NoneType> {
    match done {
        Err(e) if !hung_up(&e) => Err(e.into()),
        _ => Ok(NoneType),
    }
}

/// Whether the terminal `fd` has bytes to read within `timeout`.
fn tty_readable(fd: RawFd, timeout: Duration) -> bool {
    select_one(fd, timeout, false).unwrap_or(false)
}

/// `fd`'s target ([`axl_types::terminal`]) for a whole dump or replay,
/// given up after [`STALL_LIMIT`]; None for any other descriptor, or given up.
fn stream_write_bounded(fd: RawFd) -> Option<axl_types::terminal::Turn> {
    axl_types::terminal::turn_within(fd, STALL_LIMIT)
}

/// Write `bytes` to stdout (or, with `stderr`, stderr) as one unit, for a
/// child's output passed on: a forwarder's, or what a child still writes into
/// a stopped capture. Whether it went into a screen's capture, which copies
/// it, so the caller copies it only when not.
///
/// The target is held for the whole, waited on for as long as it takes (a
/// capture's replay holds it, and gives up on its own after
/// [`STALL_LIMIT`] without progress): never written alongside another's.
/// What std's stdout buffers goes first. Then `bytes` go to the stream's
/// file as it is now, through a descriptor of their own, never inside a
/// blocking write ([`write_when_writable`]): a screen taken or given back
/// meanwhile moves fds 1 and 2 without waiting on this write or failing it,
/// and a `select` cut short by that move backs off and goes on. Whether a
/// capture copies is read under the target, where no move happens.
pub(crate) fn write_std_unit(stderr: bool, bytes: &[u8]) -> std::io::Result<bool> {
    let fd = if stderr { 2 } else { 1 };
    let _turn = axl_types::terminal::turn(fd);
    if !stderr {
        std::io::stdout().lock().flush()?;
    }
    let into_capture = crate::out::capture_copies();
    // SAFETY: F_DUPFD_CLOEXEC on fd 1 or 2 returns a new descriptor this
    // function owns, closed when `own` drops.
    let own = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if own < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let own = unsafe { OwnedFd::from_raw_fd(own) };
    write_when_writable(own.as_raw_fd(), bytes)?;
    axl_types::terminal::wrote(fd, bytes);
    Ok(into_capture)
}

/// Write all of `bytes` to `fd`, waiting as long as it takes, but never
/// inside a `write` that blocks: each chunk of at most [`WRITE_CHUNK`] once
/// `fd` is writable. The caller holds `fd`'s target for the whole. A
/// take's `dup2` over `fd` waits out a write in flight on it on macOS, so a
/// forwarder to a pipe nobody reads, blocked in one, would hold up the take,
/// and every stream fed to the screen with it. A `select` that fails rather
/// than waits (not a descriptor it can watch) gets the write anyway, after a
/// backoff that grows to 100ms, which says why.
pub(crate) fn write_when_writable(fd: RawFd, mut bytes: &[u8]) -> std::io::Result<()> {
    let wait = Duration::from_millis(100);
    let mut failed_fast = 0u32;
    while !bytes.is_empty() {
        let asked = std::time::Instant::now();
        if !writable(fd, wait) {
            if asked.elapsed() >= wait {
                continue;
            }
            // `select` failed rather than waited: back off, then the write
            // says why (or goes through).
            failed_fast += 1;
            std::thread::sleep(Duration::from_millis(u64::from(failed_fast.min(10)) * 10));
        }
        let chunk = &bytes[..bytes.len().min(WRITE_CHUNK)];
        // SAFETY: a write from a live slice to a descriptor the caller names.
        let n = unsafe { libc::write(fd, chunk.as_ptr().cast(), chunk.len()) };
        match n {
            0 => return Err(std::io::ErrorKind::WriteZero.into()),
            n if n > 0 => bytes = &bytes[n as usize..],
            _ => {
                let e = std::io::Error::last_os_error();
                if !matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

/// Whether `fd` takes a write within `timeout`; a descriptor `select` cannot
/// watch is written to anyway.
fn writable(fd: RawFd, timeout: Duration) -> bool {
    select_one(fd, timeout, true).unwrap_or(true)
}

/// Whether `fd` is ready for reading (or, with `write`, writing) within
/// `timeout`; None for a descriptor past `FD_SETSIZE`. `select`, not `poll`:
/// macOS's `poll` does not support terminal devices.
fn select_one(fd: RawFd, timeout: Duration, write: bool) -> Option<bool> {
    // SAFETY: an fd_set on the stack, zeroed by FD_ZERO, holding one open
    // descriptor below FD_SETSIZE (checked).
    unsafe {
        if fd < 0 || fd as usize >= libc::FD_SETSIZE {
            return None;
        }
        let mut set: libc::fd_set = std::mem::zeroed();
        libc::FD_ZERO(&mut set);
        libc::FD_SET(fd, &mut set);
        let mut tv = libc::timeval {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_usec: timeout.subsec_micros() as libc::suseconds_t,
        };
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let mut ready = set;
            let (read_set, write_set): (*mut libc::fd_set, *mut libc::fd_set) = if write {
                (std::ptr::null_mut(), &mut ready)
            } else {
                (&mut ready, std::ptr::null_mut())
            };
            let n = libc::select(fd + 1, read_set, write_set, std::ptr::null_mut(), &mut tv);
            // A signal handler ran: wait out what is left, as a stall it is not.
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                tv = libc::timeval {
                    tv_sec: left.as_secs() as libc::time_t,
                    tv_usec: left.subsec_micros() as libc::suseconds_t,
                };
                continue;
            }
            return Some(n > 0 && libc::FD_ISSET(fd, &ready));
        }
    }
}

/// A copy of `fd`, closed on exec: children never get it.
fn dup(fd: RawFd) -> std::io::Result<OwnedFd> {
    // SAFETY: F_DUPFD_CLOEXEC returns a new descriptor this process owns.
    let new = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if new < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(new) })
}

/// Mark `fd` closed on exec.
fn cloexec(fd: RawFd) -> std::io::Result<()> {
    // SAFETY: plain descriptor flag updates on an open descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// `to` made a copy of `from`, keeping whether `to` is closed on exec: a
/// `dup2` would make it inheritable, so a child spawned meanwhile on another
/// thread would hold it.
fn dup2(from: RawFd, to: RawFd) -> std::io::Result<()> {
    // SAFETY: plain descriptor flag reads and updates, and `to` replaced;
    // both are open descriptors.
    let closed_on_exec = unsafe { libc::fcntl(to, libc::F_GETFD) } & libc::FD_CLOEXEC != 0;
    if unsafe { libc::dup2(from, to) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if closed_on_exec {
        cloexec(to)?;
    }
    Ok(())
}

/// Whether this process's group is the terminal `fd`'s foreground one.
fn in_foreground(fd: RawFd) -> bool {
    // SAFETY: tcgetpgrp and getpgrp take no pointers.
    let (fg, own) = unsafe { (libc::tcgetpgrp(fd), libc::getpgrp()) };
    fg == own
}

/// How long the screen waits for the terminal to say its background.
const BACKGROUND_WAIT: Duration = Duration::from_millis(150);

/// The terminal's background as it answers an OSC 11 query (`Some(true)`
/// when dark), with what else arrived meanwhile, for the keys. A DA1
/// request follows the query: every terminal answers it, so one that
/// ignores OSC 11 ends the wait at once rather than at `wait`.
fn query_background(tty: &mut File, wait: Duration) -> (Option<bool>, Vec<u8>) {
    if tty
        .write_all(b"\x1b]11;?\x1b\\\x1b[c")
        .and_then(|()| tty.flush())
        .is_err()
    {
        return (None, vec![]);
    }
    let deadline = std::time::Instant::now() + wait;
    let mut got = vec![];
    let mut buf = [0u8; 256];
    ANSWER_DUE.store(true, Ordering::SeqCst);
    loop {
        if da1_reply(&got).is_some() {
            ANSWER_DUE.store(false, Ordering::SeqCst);
            break;
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || select_one(tty.as_raw_fd(), left, false) != Some(true) {
            break;
        }
        match std::io::Read::read(tty, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
    let mut rest = got.clone();
    let dark = osc11_reply(&got).and_then(|(range, dark)| {
        rest.drain(range);
        dark
    });
    if let Some(range) = da1_reply(&rest) {
        rest.drain(range);
    }
    (dark, rest)
}

/// Whether the background query's DA1 answer is still to come: asked, and
/// not answered within its wait (a slow link). The keys clear it when it
/// arrives late; a hand-off to a child reads it out first
/// ([`drop_answers_due`]).
static ANSWER_DUE: AtomicBool = AtomicBool::new(false);

/// Before the terminal goes to a child or a prompt: read and drop the
/// answers to the screen's own queries still on their way (the background
/// query's DA1, the wheel query's), for at most [`BACKGROUND_WAIT`], so the
/// child never gets them as typed input; then drop what else is unread,
/// keys typed for the screen.
fn drop_answers_due(tty: &mut File) {
    let deadline = std::time::Instant::now() + BACKGROUND_WAIT;
    let mut got = vec![];
    let mut buf = [0u8; 256];
    let due = |got: &[u8]| {
        (ANSWER_DUE.load(Ordering::SeqCst) && da1_reply(got).is_none())
            || (WHEEL.load(Ordering::SeqCst) == WHEEL_ASKED && find(got, b"$y").is_none())
    };
    while due(&got) {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || select_one(tty.as_raw_fd(), left, false) != Some(true) {
            break;
        }
        match std::io::Read::read(tty, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
    if da1_reply(&got).is_some() {
        ANSWER_DUE.store(false, Ordering::SeqCst);
    }
    // A wheel answer read here still says what release puts back.
    if let Some(at) = find(&got, b"\x1b[?1007;") {
        let _ = late_wheel_answer(&got[at..]);
    }
    let _ = termios::tcflush(&*tty, termios::FlushArg::TCIFLUSH);
}

/// Where a DA1 answer (`ESC [ ? ... c`) is in `bytes`.
fn da1_reply(bytes: &[u8]) -> Option<std::ops::Range<usize>> {
    let start = bytes.windows(3).position(|w| w == b"\x1b[?")?;
    let end = bytes[start + 3..]
        .iter()
        .position(|&b| !(b.is_ascii_digit() || b == b';'))?;
    (bytes[start + 3 + end] == b'c').then(|| start..start + 3 + end + 1)
}

/// Where an OSC 11 answer (`ESC ] 11 ; rgb:R/G/B`, ended by BEL or ST) is
/// in `bytes`, and whether its colour is dark (None for one unread).
fn osc11_reply(bytes: &[u8]) -> Option<(std::ops::Range<usize>, Option<bool>)> {
    let start = bytes.windows(5).position(|w| w == b"\x1b]11;")?;
    let body = &bytes[start + 5..];
    let (len, end) = body.iter().enumerate().find_map(|(i, &b)| match b {
        0x07 => Some((i, 1)),
        0x1b if body.get(i + 1) == Some(&b'\\') => Some((i, 2)),
        _ => None,
    })?;
    let text = std::str::from_utf8(&body[..len]).ok();
    let dark = text.and_then(|t| t.strip_prefix("rgb:")).and_then(|rgb| {
        let channel = |h: &str| {
            let v = u32::from_str_radix(h, 16).ok()?;
            (!h.is_empty() && h.len() <= 4).then(|| v as f64 / ((1u32 << (4 * h.len())) - 1) as f64)
        };
        let mut parts = rgb.split('/').map(channel);
        let (r, g, b) = (parts.next()??, parts.next()??, parts.next()??);
        Some(0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5)
    });
    Some((start..start + 5 + len + end, dark))
}

/// Take the terminal; see the module docs. Its generation. Refused while
/// this process runs in the background (another job has the terminal).
fn take(alt_screen: bool) -> anyhow::Result<u64> {
    // Stdout is not flushed: a writer waiting on a stdout nobody reads may
    // hold its lock, and the take would wait on it. The CLI's own output
    // goes to the screen from here on ([`route`]), not through std's
    // buffer. A partial line std still buffers is flushed by the dump where
    // stdout is the terminal; to a stdout that isn't, it goes out with the
    // next write through std, after the release.
    axl_types::terminal::settle();
    let mut held = HELD.lock().unwrap();
    if held.is_some() {
        anyhow::bail!("the terminal is already taken");
    }
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|e| anyhow::anyhow!("no terminal to take: {e}"))?;
    // From a background job, changing the terminal's modes would stop this
    // process (SIGTTOU): the terminal is the foreground job's.
    if !in_foreground(tty.as_raw_fd()) {
        anyhow::bail!("the terminal belongs to another job: this one runs in the background");
    }
    let saved_termios = termios::tcgetattr(&tty)?;
    let mut cbreak = saved_termios.clone();
    // Keys as they are typed, unechoed; ISIG stays, so Ctrl+C interrupts.
    // No flow control: Ctrl+S would stop the screen's writes.
    cbreak.input_flags.remove(InputFlags::IXON);
    // No IEXTEN: Ctrl+O would discard the screen's output, Ctrl+V eat keys.
    cbreak
        .local_flags
        .remove(LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::IEXTEN);
    cbreak.control_chars[SpecialCharacterIndices::VMIN as usize] = 0;
    cbreak.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
    termios::tcsetattr(&tty, SetArg::TCSANOW, &cbreak)?;
    let undo_termios = |e: std::io::Error| {
        let _ = termios::tcsetattr(&tty, SetArg::TCSANOW, &saved_termios);
        e
    };

    {
        use std::io::IsTerminal;
        mark_captured(Some((
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal(),
        )));
    }
    let capture = Arc::new(Feed::default());
    // A screen of its own shows the task's output from its start: what it
    // wrote before Bazel's console began, handed over apart (`earlier()`).
    let earlier = if alt_screen {
        crate::out::output_before_console_ansi(MAX_CAPTURED_BYTES as u64)
    } else {
        vec![]
    };
    let forget_tty = || {
        mark_captured(None);
        crate::out::set_capture_copies(false);
        set_sink(None);
    };
    // No write half to the terminal and half to the capture.
    let moving = axl_types::terminal::every_turn_within(SWITCH_WAIT);
    let stdout = Redirect::new(1, capture.clone(), Source::Stdout, alt_screen)
        .inspect_err(|_| forget_tty())
        .map_err(undo_termios)?;
    let stderr = match Redirect::new(2, capture.clone(), Source::Stderr, alt_screen) {
        Ok(r) => r,
        Err(e) => {
            stdout.restore();
            forget_tty();
            return Err(undo_termios(e).into());
        }
    };
    // Once both are captured, the capture copies them, and the CLI's own
    // output goes to the screen instead ([`route`]). A screen of its own is
    // dumped on leaving, from here on.
    if alt_screen {
        crate::out::set_capture_copies(true);
        set_sink(Some((&capture, &stdout, &stderr)));
        set_dump_mark();
    }
    drop(moving);

    let mut tty = tty;
    let enter: &[u8] = if alt_screen {
        b"\x1b[?1049h\x1b[?7l\x1b[?25l\x1b[?2004h\x1b[2J\x1b[H"
    } else {
        b"\x1b[?25l\x1b[?2004h"
    };
    if let Err(e) = tty.write_all(enter).and_then(|()| tty.flush()) {
        stderr.restore();
        stdout.restore();
        forget_tty();
        let _ = termios::tcsetattr(&tty, SetArg::TCSANOW, &saved_termios);
        return Err(e.into());
    }
    // Asked once the modes are set, so the answer is read, not echoed.
    let (dark, typed) = if alt_screen {
        query_background(&mut tty, BACKGROUND_WAIT)
    } else {
        (None, vec![])
    };
    if let Some(dark) = dark {
        crate::theme::learn_background(if dark {
            aspect_theme::Appearance::Dark
        } else {
            aspect_theme::Appearance::Light
        });
    }
    install_panic_hook();
    remember_for_crash(
        tty.as_raw_fd(),
        &saved_termios,
        alt_screen,
        stdout.saved.as_raw_fd(),
        stderr.saved.as_raw_fd(),
    );
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::SeqCst);
    axl_types::stream::set_terminal_input(suspend, resume);
    if alt_screen {
        stop_with_the_screen();
    }
    *held = Some(Held {
        generation,
        alt_screen,
        tty,
        saved_termios,
        cbreak,
        stdout: Some(stdout),
        stderr: Some(stderr),
        last_frame: vec![],
        frame_size: None,
        mouse_on: false,
        suspended: false,
        suspends: 0,
        capture,
        earlier,
        keys: Keys {
            pending: typed,
            ..Keys::default()
        },
    });
    Ok(generation)
}

/// What a crash handler needs to give the terminal back, without allocating
/// or locking: set while a screen is held. One for the process, its three
/// descriptors its own copies, never closed and renewed in place by each
/// `take()`, so a handler that read the pointer just before a release
/// cleared it still writes to open descriptors.
struct CrashState {
    tty: RawFd,
    termios: libc::termios,
    alt_screen: bool,
    stdout: RawFd,
    stderr: RawFd,
    /// `output: <output.ansi>\n`, written to stderr after the restore: the
    /// screen's dump cannot be made from a signal handler.
    note: [u8; CRASH_NOTE],
    note_len: usize,
}

/// The longest crash note kept, a path's line.
const CRASH_NOTE: usize = 1024;

/// The crash note for the task's `output.ansi`, cut to fit.
fn crash_note() -> ([u8; CRASH_NOTE], usize) {
    let mut note = [0u8; CRASH_NOTE];
    let Some(path) = crate::out::output_ansi_path() else {
        return (note, 0);
    };
    let line = format!("output: {}\n", path.display());
    let n = line.len().min(CRASH_NOTE);
    note[..n].copy_from_slice(&line.as_bytes()[..n]);
    (note, n)
}

static CRASH: std::sync::atomic::AtomicPtr<CrashState> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

fn remember_for_crash(
    tty: RawFd,
    termios: &Termios,
    alt_screen: bool,
    stdout: RawFd,
    stderr: RawFd,
) {
    /// The one state every take reuses, its descriptors kept at the same
    /// numbers, so a handler that loaded it never writes to a closed one.
    static SLOT: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
    // SAFETY: plain descriptor duplicates; -1 on failure, which the
    // handler's calls then fail on harmlessly. `dup2` onto a kept number
    // clears close-on-exec, so it is set again.
    let keep = |fd: RawFd| unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    let renew = |at: RawFd, fd: RawFd| unsafe {
        if at < 0 || libc::dup2(fd, at) < 0 {
            return keep(fd);
        }
        libc::fcntl(at, libc::F_SETFD, libc::FD_CLOEXEC);
        at
    };
    let mut slot = SLOT.lock().unwrap();
    CRASH.store(std::ptr::null_mut(), Ordering::SeqCst);
    let (note, note_len) = if alt_screen {
        crash_note()
    } else {
        ([0u8; CRASH_NOTE], 0)
    };
    let state = if *slot == 0 {
        let state = Box::into_raw(Box::new(CrashState {
            tty: keep(tty),
            termios: termios.clone().into(),
            alt_screen,
            stdout: keep(stdout),
            stderr: keep(stderr),
            note,
            note_len,
        }));
        *slot = state as usize;
        state
    } else {
        let state = *slot as *mut CrashState;
        // SAFETY: the leaked Box above, written only here, under SLOT.
        unsafe {
            let s = &mut *state;
            s.tty = renew(s.tty, tty);
            s.termios = termios.clone().into();
            s.alt_screen = alt_screen;
            s.stdout = renew(s.stdout, stdout);
            s.stderr = renew(s.stderr, stderr);
            s.note = note;
            s.note_len = note_len;
        }
        state
    };
    CRASH.store(state, Ordering::SeqCst);
}

/// For a crash handler: give back the terminal a held screen has, so the
/// crash report reaches it and the shell is usable after. Async-signal-safe:
/// only `tcsetattr`, `write` and `dup2` on descriptors set aside when the
/// screen was taken. What was captured is not replayed.
pub fn restore_terminal_after_crash() {
    let state = CRASH.load(Ordering::SeqCst);
    if state.is_null() {
        return;
    }
    // SAFETY: set from the one leaked Box `take` reuses, never freed, whose
    // descriptors are never closed, only renewed in place.
    unsafe {
        let s = &*state;
        libc::tcsetattr(s.tty, libc::TCSANOW, &s.termios);
        let leave = leave_sequence(s.alt_screen);
        libc::write(s.tty, leave.as_ptr().cast(), leave.len());
        let wheel = wheel_restore();
        libc::write(s.tty, wheel.as_ptr().cast(), wheel.len());
        libc::dup2(s.stdout, 1);
        libc::dup2(s.stderr, 2);
        if s.note_len > 0 {
            libc::write(2, s.note.as_ptr().cast(), s.note_len);
        }
    }
}

/// Put back a screen still held: the terminal's modes, the main screen, the
/// process's stdout and stderr, and then what was captured from each,
/// written to it. A no-op when none is held. Whether one was.
///
/// Bounded unless the terminal itself stops taking output (the leave sequence
/// and the mode restore wait on it), so the forced exit ends: the lock is
/// waited on for 500ms,
/// and the replay, run without it, gives up on a destination that takes
/// nothing for `STALL_LIMIT`. A frame still holding the lock after that (stuck
/// in a write) gets the crash path's restore instead, with nothing replayed.
pub fn release_all() -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    // No forwarder writes into the replay (see [`OUTPUT_ORDER`]), unless one
    // holds a lock past the deadline.
    let mut order = Order {
        _out: lock_until(&OUTPUT_ORDER[0], deadline),
        err: lock_until(&OUTPUT_ORDER[1], deadline),
    };
    loop {
        match HELD.try_lock() {
            // The replay runs with the lock let go.
            Ok(mut held) => {
                let mine = held.take();
                drop(held);
                return release_dumping(mine, &mut order);
            }
            Err(std::sync::TryLockError::Poisoned(p)) => {
                return release_dumping(p.into_inner().take(), &mut order);
            }
            Err(std::sync::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                restore_terminal_after_crash();
                return true;
            }
        }
    }
}

/// Give a held screen of its own back for a while, for something that needs
/// the real terminal (a prompt reading stdin, an interactive child): the
/// task's output so far dumped to it ([`dump_output`]) and stdout's captured
/// lines to a stdout that is not the terminal, the main screen (the screen's
/// own cleared first), its modes and stdout and stderr back, the capture's
/// copies stopped, and no answer to its own queries or key typed for it left
/// for the child to read ([`drop_answers_due`]). Whether
/// one was given back, for [`resume`], which each true needs once: a screen
/// already suspended counts another, and comes back with the last resume.
/// One not of its own, or none: false.
pub(crate) fn suspend() -> bool {
    let finishing;
    let moving;
    {
        // Bounded, as a release's: one replaying to a stalled stdout holds
        // these, and Ctrl+Z must not wait it out.
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        let _order = Order {
            _out: lock_until(&OUTPUT_ORDER[0], deadline),
            err: lock_until(&OUTPUT_ORDER[1], deadline),
        };
        let mut held = HELD.lock().unwrap();
        let Some(h) = held.as_mut().filter(|h| h.alt_screen) else {
            return false;
        };
        if h.suspended {
            h.suspends += 1;
            return true;
        }
        h.suspends = 1;
        moving = give_back_descriptors([h.stdout.as_mut(), h.stderr.as_mut()]);
        // The child gets a quiet terminal: no answer meant for the screen
        // in its input, no frame left on the screen it leaves.
        drop_answers_due(&mut h.tty);
        let _ = h.tty.write_all(b"\x1b[2J\x1b[H");
        let _ = h.tty.write_all(leave_sequence(true));
        let _ = h.tty.write_all(wheel_restore());
        let _ = h.tty.flush();
        let _ = termios::tcsetattr(&h.tty, SetArg::TCSANOW, &h.saved_termios);
        CRASH.store(std::ptr::null_mut(), Ordering::SeqCst);
        mark_captured(None);
        // What the capture read before now stays in its feed, for the screen.
        finishing = (h.stdout.take(), h.stderr.take());
        h.suspended = true;
    }
    let held = moving.holds(2);
    if dump_output_turned(DUMP_LINES, "… {n} older lines: aspect history show", held) {
        DUMPED.store(true, Ordering::SeqCst);
    }
    drop(moving);
    // After the dump: what a child still writes into a capture comes after
    // it. Stdout's lines go to a stdout that is not the terminal, as on a
    // release: the dump showed them on the terminal only.
    let (stdout, stderr) = finishing;
    if let Some(r) = stdout {
        // SAFETY: isatty on a descriptor the redirect owns.
        let tty = unsafe { libc::isatty(r.saved.as_raw_fd()) } == 1;
        r.finish(!tty);
    }
    if let Some(r) = stderr {
        r.finish(false);
    }
    true
}

/// Stdout's and stderr's: each held by a release from before the screen is
/// taken out of `HELD` until that stream's capture is replayed, and by a
/// forwarder to that stream while it decides a chunk's route and feeds a
/// screen ([`forwarding`]), never across a write to the terminal: so a
/// child's output never lands inside the replay, nor goes through the
/// capture's pipe as the screen goes, where it would be copied twice, and a
/// stream nobody reads stalls no one else (stderr's is let go before
/// stdout's replay, unless the two are one file). A line a forwarder feeds
/// the screen is copied to the output files under it too, so a release's
/// dump has it.
static OUTPUT_ORDER: [Mutex<()>; 2] = [Mutex::new(()), Mutex::new(())];

/// [`OUTPUT_ORDER`] as a release holds it, a lock it gave up on `None`.
struct Order {
    _out: Option<std::sync::MutexGuard<'static, ()>>,
    err: Option<std::sync::MutexGuard<'static, ()>>,
}

impl Order {
    /// Both, stdout's first, as every release takes them.
    fn lock() -> Order {
        let lock = |m: &'static Mutex<()>| Some(m.lock().unwrap_or_else(|p| p.into_inner()));
        Order {
            _out: lock(&OUTPUT_ORDER[0]),
            err: lock(&OUTPUT_ORDER[1]),
        }
    }

    /// Neither: for a release nothing else can be writing beside.
    fn none() -> Order {
        Order {
            _out: None,
            err: None,
        }
    }
}

/// `m` locked, or None when it is still held at `deadline`.
fn lock_until(
    m: &'static Mutex<()>,
    deadline: std::time::Instant,
) -> Option<std::sync::MutexGuard<'static, ()>> {
    loop {
        match m.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(p)) => return Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::sync::TryLockError::WouldBlock) => return None,
        }
    }
}

/// Run `f`, a forwarder's handling of one chunk to stderr (or stdout),
/// between that stream's releases.
pub(crate) fn forwarding<T>(stderr: bool, f: impl FnOnce() -> T) -> T {
    let _order = OUTPUT_ORDER[stderr as usize]
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    f()
}

/// Hand `bytes` (whole lines) a child wrote to the screen holding the
/// terminal, as its stderr (or stdout) capture would: a forwarder's lines
/// go to the screen's console here, and are kept for its replay as the
/// capture's are, not through the process's own stderr, where the capture
/// would copy them a second time and splice them with the CLI's. Whether a
/// screen took them (one of its own, not given back); `bytes` empty asks
/// only that. Its first `shown` bytes, a line's start written before the
/// screen was taken, are on the main screen already: not kept to replay.
pub(crate) fn feed_screen(stderr: bool, bytes: &[u8], shown: usize) -> bool {
    let Ok(held) = HELD.lock() else {
        return false;
    };
    let Some(h) = held.as_ref().filter(|h| h.alt_screen && !h.suspended) else {
        return false;
    };
    if !bytes.is_empty() {
        h.capture.write(
            if stderr {
                Source::Stderr
            } else {
                Source::Stdout
            },
            bytes,
        );
        if let Some(r) = if stderr { &h.stderr } else { &h.stdout } {
            keep(
                &mut r.written.lock().unwrap(),
                &bytes[shown.min(bytes.len())..],
            );
        }
    }
    true
}

/// `bytes` (output.ansi's, which keeps colour, not links) with their links
/// made again as the CLI's own lines make them, as OSC 8 links only: a
/// screen opens a link itself, so no URL is written out beside it, even
/// where the terminal shows them as text.
fn relinked(bytes: &[u8]) -> Vec<u8> {
    if !crate::out::terminal_links() {
        return bytes.to_vec();
    }
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        match line.strip_suffix('\n') {
            Some(body) => {
                out.push_str(&crate::out::linked_for_screen(body));
                out.push('\n');
            }
            None => out.push_str(&crate::out::linked_for_screen(line)),
        }
    }
    out.into_bytes()
}

/// Ctrl+Z (SIGTSTP) while a screen holds the terminal: the screen is given
/// back first ([`suspend`]: the main screen, the shell's modes), then the
/// process stops (SIGSTOP); once continued (`fg`) the screen is taken again,
/// redrawn ([`resume`]). With no screen held it just stops, as by default.
/// Installed once, with the first screen of its own; the signal handler only
/// writes a byte to a pipe a thread reads.
fn stop_with_the_screen() {
    static ONCE: Once = Once::new();
    static WAKE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
    extern "C" fn on_tstp(_: libc::c_int) {
        let fd = WAKE.load(Ordering::SeqCst);
        if fd >= 0 {
            // SAFETY: write(2) is async-signal-safe; a full pipe drops it.
            unsafe { libc::write(fd, [1u8].as_ptr().cast(), 1) };
        }
    }
    ONCE.call_once(|| {
        let Ok((read, write)) = std::io::pipe() else {
            return;
        };
        WAKE.store(
            std::os::fd::IntoRawFd::into_raw_fd(OwnedFd::from(write)),
            Ordering::SeqCst,
        );
        let spawned = std::thread::Builder::new()
            .name("term-tstp".into())
            .spawn(move || {
                let mut read = read;
                let mut byte = [0u8; 1];
                while std::io::Read::read(&mut read, &mut byte).is_ok_and(|n| n == 1) {
                    let suspended = suspend();
                    // SAFETY: stopping this process, as Ctrl+Z asked.
                    unsafe { libc::raise(libc::SIGSTOP) };
                    if suspended {
                        resume();
                    }
                }
            });
        if spawned.is_ok() {
            // SAFETY: a handler that only calls write(2).
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_tstp as *const () as usize;
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(libc::SIGTSTP, &action, std::ptr::null_mut());
            }
        }
    });
}

/// Whether a full screen of its own holds the terminal now, not given back:
/// a child spawned meanwhile has no terminal of its own.
pub(crate) fn screen_holds_terminal() -> bool {
    HELD.lock()
        .ok()
        .is_some_and(|h| h.as_ref().is_some_and(|h| h.alt_screen && !h.suspended))
}

/// Take a screen [`suspend`] gave back again: its modes, the alternate
/// screen, the mouse when it had it, stdout and stderr captured again, its
/// last frame drawn when the terminal is the size it was drawn for (else
/// left clear for the next frame, which reads the size anew), and a dump on
/// leaving from here on. A no-op unless one
/// is suspended.
pub(crate) fn resume() {
    let mut held = HELD.lock().unwrap();
    let Some(h) = held.as_mut().filter(|h| h.suspended) else {
        return;
    };
    h.suspends = h.suspends.saturating_sub(1);
    if h.suspends > 0 {
        return;
    }
    let moving = axl_types::terminal::every_turn_within(SWITCH_WAIT);
    let redirected = (
        Redirect::new(1, h.capture.clone(), Source::Stdout, true),
        Redirect::new(2, h.capture.clone(), Source::Stderr, true),
    );
    if let (Ok(out), Ok(err)) = &redirected {
        // Once both are captured, as at the take: the next dump starts here.
        crate::out::set_capture_copies(true);
        set_sink(Some((&h.capture, out, err)));
        set_dump_mark();
    }
    drop(moving);
    match redirected {
        (Ok(out), Ok(err)) => {
            h.stdout = Some(out);
            h.stderr = Some(err);
        }
        (out, err) => {
            // Without its captures the screen would be drawn over: it stays
            // given back, the task's lines going to the terminal.
            for r in [out.ok(), err.ok()].into_iter().flatten() {
                r.restore();
            }
            return;
        }
    }
    // What the prompt or child left unread (a newline after its answer) is
    // not the screen's first key.
    let _ = termios::tcflush(&h.tty, termios::FlushArg::TCIFLUSH);
    let _ = termios::tcsetattr(&h.tty, SetArg::TCSANOW, &h.cbreak);
    let mut enter = b"\x1b[?1049h\x1b[?7l\x1b[?25l\x1b[?2004h\x1b[2J\x1b[H".to_vec();
    if h.mouse_on {
        enter.extend_from_slice(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
    }
    // The last frame only at the size it was drawn for; else the screen
    // stays clear until its next frame, which reads the size anew.
    if h.frame_size.is_some() && h.frame_size == size_of(h.tty.as_raw_fd()) {
        enter.extend_from_slice(&synchronized(&h.last_frame));
    }
    let _ = h.tty.write_all(&enter).and_then(|()| h.tty.flush());
    {
        use std::io::IsTerminal;
        mark_captured(Some((
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal(),
        )));
    }
    if let (Some(out), Some(err)) = (&h.stdout, &h.stderr) {
        remember_for_crash(
            h.tty.as_raw_fd(),
            &h.saved_termios,
            true,
            out.saved.as_raw_fd(),
            err.saved.as_raw_fd(),
        );
    }
    h.suspended = false;
}

/// [`release_all`] for `held`: the task's output since the screen was
/// taken written from `output.ansi` ([`dump_output`]) when there is one to
/// write, else what was captured replayed. Whether one was held.
fn release_dumping(held: Option<Held>, order: &mut Order) -> bool {
    let was = held.is_some();
    let dumping = was && dump_ready();
    let _ = release(
        held,
        !dumping,
        dumping.then_some((DUMP_LINES, "… {n} older lines: aspect history show")),
        order,
    );
    was
}

/// Where `output.ansi` stood when the screen was taken (or came back from a
/// suspend): what a dump on leaving writes from, the main screen showing
/// what came before. None once dumped, or with no screen of its own.
static DUMP_MARK: Mutex<Option<u64>> = Mutex::new(None);

/// Whether the screen given back last had its dump written, or is having it
/// written (set as the mark is taken): by its release ([`release`] with a
/// dump), by a suspend still in force, or by a dump of its own. A later
/// [`dump_output`] says so rather than write it again. Cleared when a screen
/// of its own (the alternate screen) is taken or comes back from a suspend.
static DUMPED: AtomicBool = AtomicBool::new(false);

/// The dump's mark from here on (at a take, or back from a suspend): what
/// follows is dumped on leaving, and nothing of it is dumped yet.
fn set_dump_mark() {
    *DUMP_MARK.lock().unwrap() = crate::out::output_ansi_mark();
    DUMPED.store(false, Ordering::SeqCst);
}

/// The most lines a dump writes, the older counted on a line before them.
pub(crate) const DUMP_LINES: usize = 20000;

/// Whether a dump would write anything: a mark, and the task's files.
fn dump_ready() -> bool {
    DUMP_MARK.lock().unwrap().is_some() && crate::out::output_ansi_mark().is_some()
}

/// Write the task's output since the mark ([`DUMP_MARK`]) to stderr, as
/// `output.ansi` keeps it, its links made again as the CLI's own
/// lines make them (the file keeps colour, not links; Bazel's lines are kept
/// before their URLs are written), but for a line holding a URL already
/// where the terminal shows them as text (the CLI's own lines write
/// theirs). Its newest `max_lines`,
/// after a line `older` gives, `{n}` the count of those it leaves out.
/// An unended last line (the task's, the console's, or a stray writer's the
/// capture copied) is written as far as it goes, its end following on the
/// terminal. Where the terminal is left mid-line, the dump starts on a line
/// of its own: no row is erased, so a line the CLI left unended shows its
/// start again in the dump. Not copied to the output
/// files, which hold it; what stderr takes nothing of for `STALL_LIMIT` is
/// dropped. Whether the screen's dump is written: by this call, or already
/// by the release or suspend that gave the screen back ([`DUMPED`]); never
/// twice, nor without a mark or the task's files.
pub(crate) fn dump_output(max_lines: usize, older: &str) -> bool {
    if DUMPED.load(Ordering::SeqCst) {
        return true;
    }
    dump_output_turned(max_lines, older, false)
}

/// A pause between a dump reading `output.ansi` and writing it, from
/// `ASPECT_TEST_DUMP_DELAY_MS` (read once, at most 5s): for tests, to widen
/// the window a writer not held off would land in.
fn dump_test_delay() {
    static DELAY: std::sync::OnceLock<Option<Duration>> = std::sync::OnceLock::new();
    let delay = DELAY.get_or_init(|| {
        std::env::var("ASPECT_TEST_DUMP_DELAY_MS")
            .ok()
            .and_then(|ms| ms.parse::<u64>().ok())
            .map(|ms| Duration::from_millis(ms.min(5000)))
    });
    if let Some(delay) = delay {
        std::thread::sleep(*delay);
    }
}

/// [`dump_output`], with stderr's target already held by the caller
/// (`held`): for a release or a suspend, which holds the targets from giving
/// the descriptors back to the end of the dump, so no line printed meanwhile
/// shows both live and in it. Where giving the descriptors back gave up on
/// stderr's (a writer stalled on it), `held` is false and the dump takes
/// stderr's turn itself, as a dump of its own does; one given up on stdout's
/// alone leaves `held` true.
fn dump_output_turned(max_lines: usize, older: &str, held: bool) -> bool {
    // Said to be dumped as the mark is taken, under its lock: a release or
    // suspend racing this one sees either the mark or that it is dumped.
    let mark = {
        let mut mark = DUMP_MARK.lock().unwrap();
        let taken = mark.take();
        if taken.is_some() {
            DUMPED.store(true, Ordering::SeqCst);
        }
        taken
    };
    let Some(mark) = mark else {
        return false;
    };
    let Some((text, _)) = crate::out::output_ansi_since(mark) else {
        DUMPED.store(false, Ordering::SeqCst);
        return false;
    };
    dump_test_delay();
    let relink = crate::out::terminal_links();
    let as_text = crate::out::links_as_text();
    let before = crate::out::links_written();
    let out = dump_text(&String::from_utf8_lossy(&text), max_lines, older, |line| {
        if relink && !(as_text && line.contains("://")) {
            crate::out::linked(line)
        } else {
            line.to_string()
        }
    });
    DUMP_WRITTEN.fetch_add(crate::out::links_written() - before, Ordering::SeqCst);
    // Not through `std::io::stderr()`, whose lock a writer stalled on a
    // stderr nobody reads may hold; the target's lock keeps every other
    // writer out of the dump instead, stdout's too where they share it.
    // Bounded as the replay is.
    // SAFETY: fd 2 stays open; ManuallyDrop never closes it.
    let mut err = std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(2) });
    let turn = (!held)
        .then(|| axl_types::terminal::turn_at_line_end(2, STALL_LIMIT, Duration::from_millis(200)));
    let turned = held || turn.as_ref().is_some_and(|t| t.is_some());
    // What std's stdout holds of an unended line goes out first where it
    // shares the terminal, so it is not left to surface after everything.
    // Only with the target held (stderr's is stdout's here), else a writer
    // stalled on it holds std's lock too; and only while the terminal takes
    // writes, so one stopped (Ctrl+S) never blocks it with every target held.
    // SAFETY: isatty on fd 1.
    if turned
        && unsafe { libc::isatty(1) } == 1
        && axl_types::terminal::one_target(1, 2)
        && writable(1, Duration::from_millis(200))
    {
        use std::io::Write as _;
        let _ = std::io::stdout().lock().flush();
    }
    // The terminal left mid-line (a line the CLI, a child or a stray
    // writer left unended): the dump starts a line of its own. That line's
    // start shows again in the dump where it is the CLI's.
    if axl_types::terminal::mid_line(2) {
        write_patiently(&mut err, b"\n");
    }
    write_patiently(&mut err, out.as_bytes());
    axl_types::terminal::wrote(2, out.as_bytes());
    true
}

/// The URLs the dumps have written out beside their links ([`dump_output`]):
/// a build's console writes none out while a screen holds the terminal, so
/// these are the ones a terminal that shows them as text has in its
/// scrollback.
static DUMP_WRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What [`dump_output`] writes of `text`: its newest `max_lines` lines,
/// each through `link` without its end, after a line `older` gives (`{n}`
/// the count left out) when any were.
fn dump_text(text: &str, max_lines: usize, older: &str, link: impl Fn(&str) -> String) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let skip = lines.len().saturating_sub(max_lines);
    let mut out = String::with_capacity(text.len() + 64);
    if skip > 0 && !older.is_empty() {
        out.push_str(&older.replace("{n}", &skip.to_string()));
        out.push('\n');
    }
    for line in &lines[skip..] {
        match line.strip_suffix('\n') {
            Some(body) => {
                out.push_str(&link(body));
                out.push('\n');
            }
            None => out.push_str(&link(line)),
        }
    }
    out
}

/// [`release_all`] for `held`, stderr's capture written to it only with
/// `replay_stderr`. Without it, stdout's is written only where stdout is
/// not the terminal (a screen that shows what it captured has shown that
/// too), and what the captures still held, read as they stopped, is
/// returned as lines for the screen to write itself. Stderr's capture goes
/// first, and its lock in `order` is let go before stdout's replay when the
/// two are different files: a stdout nobody reads holds up stdout alone.
///
/// With `dump` (its newest lines, and the line saying how many older ones
/// it left out), the task's output since the screen was taken is dumped
/// ([`dump_output`]) right after the main screen comes back, before any
/// replay, with `order` and every target giving the descriptors back took
/// still held: no line written meanwhile shows both live and in the dump,
/// nor before it. A target given up on there (a writer stalled on it) is
/// not held; the dump takes stderr's turn itself if it is that one.
fn release(
    held: Option<Held>,
    replay_stderr: bool,
    dump: Option<(usize, &str)>,
    order: &mut Order,
) -> Vec<String> {
    let Some(mut held) = held else {
        return vec![];
    };
    // SAFETY: isatty on a descriptor the redirect owns.
    let stdout_tty = held
        .stdout
        .as_ref()
        .is_none_or(|r| unsafe { libc::isatty(r.saved.as_raw_fd()) } == 1);
    // Both descriptors back first, so a crash during the replays below
    // reports to the terminal.
    let moving = give_back_descriptors([held.stdout.as_mut(), held.stderr.as_mut()]);
    let leave = leave_sequence(held.alt_screen);
    let _ = held.tty.write_all(leave);
    let _ = held.tty.write_all(wheel_restore());
    let _ = held.tty.flush();
    WHEEL.store(WHEEL_UNTOUCHED, Ordering::SeqCst);
    // Keys typed but never read go too: the shell would run them.
    let _ = termios::tcsetattr(&held.tty, SetArg::TCSAFLUSH, &held.saved_termios);
    CRASH.store(std::ptr::null_mut(), Ordering::SeqCst);
    mark_captured(None);
    let all_held = moving.holds(2);
    if let Some((max_lines, older)) = dump
        && dump_output_turned(max_lines, older, all_held)
    {
        DUMPED.store(true, Ordering::SeqCst);
    }
    drop(moving);
    let one_file = match (&held.stdout, &held.stderr) {
        (Some(o), Some(e)) => same_file(o.saved.as_raw_fd(), e.saved.as_raw_fd()),
        _ => false,
    };
    let stdout = held.stdout.take();
    if one_file && let Some(r) = stdout {
        r.finish(replay_stderr || !stdout_tty);
        if let Some(r) = held.stderr.take() {
            r.finish(replay_stderr);
        }
    } else {
        if let Some(r) = held.stderr.take() {
            r.finish(replay_stderr);
        }
        order.err = None;
        if let Some(r) = stdout {
            r.finish(replay_stderr || !stdout_tty);
        }
    }
    if replay_stderr {
        return vec![];
    }
    held.capture.drain().colours
}

/// Where the CLI's own output goes while a screen of its own holds the
/// terminal ([`route`]): its console's feed, and what its captures keep for
/// the replay. Set and cleared with every target held, where the capture's
/// copies go on and off, so a write is routed exactly when fds 1 and 2 are
/// captured.
struct Sink {
    capture: Arc<Feed>,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// Route the CLI's writes to `capture` and the replays `out` and `err`
/// keep, or (None) to fds 1 and 2 again.
fn set_sink(sink: Option<(&Arc<Feed>, &Redirect, &Redirect)>) {
    *SINK.lock().unwrap_or_else(|p| p.into_inner()) = sink.map(|(capture, out, err)| Sink {
        capture: capture.clone(),
        stdout: out.written.clone(),
        stderr: err.written.clone(),
    });
}

/// The route the CLI's own writes are offered ([`axl_types::terminal`]):
/// while a screen of its own holds the terminal, `bytes` for fd 1 or 2 go to
/// its console and are kept for its replay, as its capture would have them,
/// without passing through the captured descriptors; the writer copies them
/// to the output files itself. Whether it took them.
fn route(fd: i32, bytes: &[u8]) -> bool {
    let sink = SINK.lock().unwrap_or_else(|p| p.into_inner());
    let Some(s) = sink.as_ref() else {
        return false;
    };
    let (source, kept) = if fd == 2 {
        (Source::Stderr, &s.stderr)
    } else {
        (Source::Stdout, &s.stdout)
    };
    s.capture.write(source, bytes);
    keep(&mut kept.lock().unwrap_or_else(|p| p.into_inner()), bytes);
    true
}

/// Install [`route`] for the CLI's own output. Once, at start-up.
pub fn route_cli_output() {
    axl_types::terminal::route_to(route);
}

/// Put fds 1 and 2 back from a screen's captures, and the output files'
/// copies back to the print paths, with every target held the while (each
/// waited on for [`SWITCH_WAIT`]; returned, for a caller that dumps before
/// it lets them go): no write lands half in a capture and half on the terminal, and
/// none is copied between the capture's last chunks (its readers drained
/// first), or goes uncopied.
fn give_back_descriptors(redirects: [Option<&mut Redirect>; 2]) -> axl_types::terminal::Turns {
    let moving = axl_types::terminal::every_turn_within(SWITCH_WAIT);
    let mut redirects: Vec<&mut Redirect> = redirects.into_iter().flatten().collect();
    for r in &redirects {
        r.put_back();
    }
    for r in &mut redirects {
        r.drain();
    }
    crate::out::set_capture_copies(false);
    set_sink(None);
    moving
}

/// Whether descriptors `a` and `b` are one target ([`axl_types::terminal::one_target`]).
fn same_file(a: RawFd, b: RawFd) -> bool {
    axl_types::terminal::one_target(a, b)
}

/// `frame` as one synchronized update (DEC mode 2026): the terminal holds
/// what it draws until the end, so a frame never shows half drawn.
fn synchronized(frame: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() + 16);
    out.extend_from_slice(b"\x1b[?2026h");
    out.extend_from_slice(frame);
    out.extend_from_slice(b"\x1b[?2026l");
    out
}

/// What gives the terminal back its cursor, the mouse (`mouse(True)`), a
/// synchronized update a frame cut short left open (`?2026`), and,
/// for a screen that took it, the main screen and wrapping; its wheel
/// (`wheel_keys`) is put back as it was apart ([`wheel_restore`]).
fn leave_sequence(alt_screen: bool) -> &'static [u8] {
    if alt_screen {
        b"\x1b[?2026l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?2004l\x1b[?7h\x1b[?25h\x1b[?1049l"
    } else {
        b"\x1b[?2026l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?2004l\x1b[?25h"
    }
}

/// What a screen's `wheel_keys` did to the terminal's alternate scroll
/// (`?1007`): nothing, or changed it from on or from off (or from a state
/// the terminal did not say). One atomic, for the crash path's restore.
static WHEEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(WHEEL_UNTOUCHED);
const WHEEL_UNTOUCHED: u8 = 0;
const WHEEL_WAS_ON: u8 = 1;
const WHEEL_WAS_OFF: u8 = 2;
/// Asked, and not answered in time: taken as off, unless the answer comes
/// later among the keys ([`late_wheel_answer`]).
const WHEEL_ASKED: u8 = 3;

/// What puts the terminal's alternate scroll back as it was before
/// `wheel_keys` changed it: nothing when untouched or it was on (a VTE
/// terminal's default), off otherwise, as it is in most terminals by
/// default and in one that did not answer the query.
fn wheel_restore() -> &'static [u8] {
    match WHEEL.load(Ordering::SeqCst) {
        WHEEL_WAS_ON => b"\x1b[?1007h",
        WHEEL_WAS_OFF | WHEEL_ASKED => b"\x1b[?1007l",
        _ => b"",
    }
}

/// The terminal's alternate scroll mode now, asked with DECRQM: Some(true)
/// or Some(false) as it answers within [`WHEEL_QUERY_WAIT`], None when it
/// does not. What arrives meanwhile is kept for `key()`, the answer among
/// it (an unknown sequence there, skipped).
fn alternate_scroll(tty: &mut File, keys: &mut Keys) -> Option<bool> {
    tty.write_all(b"\x1b[?1007$p").ok()?;
    tty.flush().ok()?;
    let deadline = std::time::Instant::now() + WHEEL_QUERY_WAIT;
    let from = keys.pending.len();
    loop {
        let reply = b"\x1b[?1007;";
        if let Some(at) = find(&keys.pending[from..], reply) {
            let rest = &keys.pending[from + at + reply.len()..];
            if let Some(end) = find(rest, b"$y") {
                return match &rest[..end] {
                    b"1" | b"3" => Some(true),
                    b"2" | b"4" => Some(false),
                    _ => None,
                };
            }
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || !tty_readable(tty.as_raw_fd(), left) {
            return None;
        }
        let mut buf = [0u8; 256];
        match tty.read(&mut buf) {
            Ok(n) if n > 0 => keys.pending.extend_from_slice(&buf[..n]),
            _ => return None,
        }
    }
}

/// How long `wheel_keys` waits for the terminal to say its alternate scroll.
const WHEEL_QUERY_WAIT: Duration = Duration::from_millis(150);

/// Remember the terminal state from before a screen captured stdout and
/// stderr (`Some`), or forget it (`None`): for AXL's `is_tty`, the console's
/// display and colour, and the size of the PTY Bazel draws into.
fn mark_captured(state: Option<(bool, bool)>) {
    axl_types::stream::remember_tty(state);
    bazel::console::capture::set_stderr_captured(state.is_some());
}

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Only where a panic ends the process: an unwinding one may not
            // (a task's thread, a tokio task), and the build goes on. The
            // panicking thread may hold the lock, so it is not waited on.
            if cfg!(panic = "abort") {
                match HELD.try_lock() {
                    Ok(mut held) => {
                        let _ = release(held.take(), true, None, &mut Order::none());
                    }
                    Err(_) => restore_terminal_after_crash(),
                }
            }
            previous(info);
        }));
    });
}

/// Run `f` on the screen `this` took, when it still holds the terminal.
fn with_held<T>(
    this: values::Value,
    f: impl FnOnce(&mut Held) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let generation = generation_of(this)?;
    let mut held = HELD.lock().unwrap();
    match held.as_mut() {
        Some(h) if h.generation == generation => f(h),
        _ => anyhow::bail!("the screen was released"),
    }
}

fn generation_of(this: values::Value) -> anyhow::Result<u64> {
    Ok(this
        .downcast_ref_err::<Screen>()
        .into_anyhow_result()?
        .generation)
}

/// What has been read from the terminal and not yet handed out as keys.
#[derive(Debug, Default)]
struct Keys {
    pending: Vec<u8>,
    /// Inside a paste whose end marker has not come yet: what arrives is
    /// more of it ([`paste_chunk`]), never keys.
    in_paste: bool,
    /// Since when, inside a paste, a look at the terminal has found nothing
    /// waiting (cleared by anything read): to end a paste whose end marker
    /// never comes ([`PASTE_IDLE`]). Time unpolled (a frame late) is not
    /// quiet, since what came meanwhile is still waiting.
    quiet_since: Option<std::time::Instant>,
    /// Since when a background answer has been arriving in pieces (a slow
    /// link): held whole, never read as keys ([`ANSWER_WAIT`] at most).
    answer_since: Option<std::time::Instant>,
    /// The terminal is gone (a hangup): a read failed so ([`hung_up`]), or
    /// found nothing where a wait said something was. From then on there is
    /// no key to wait for.
    gone: bool,
}

/// How long a paste may go without a byte before what follows is keys
/// again: a stream that lost its end marker would otherwise take every key
/// after it as pasted text, the finished screen's leave keys among them.
const PASTE_IDLE: Duration = Duration::from_secs(1);

impl Keys {
    /// The next key `tty` has, without waiting: one it cannot name is
    /// skipped, not taken for none. None when nothing has come, or, inside a
    /// paste, when its rest has not yet, which the caller waits for under its
    /// own deadline.
    fn take(&mut self, tty: &mut File) -> std::io::Result<Option<String>> {
        loop {
            match self.next(tty, Duration::ZERO)? {
                Some(name) => return Ok(Some(name)),
                None if self.pending.is_empty() || self.in_paste => return Ok(None),
                None => {}
            }
        }
    }

    /// The next key typed within `timeout`, named as the Starlark `key()`
    /// says; None for one it cannot name, or nothing.
    fn next(&mut self, tty: &mut File, timeout: Duration) -> std::io::Result<Option<String>> {
        let mut buf = [0u8; 4096];
        // Gone quiet inside a paste: its end is lost, and a fragment held back
        // for it (part of the marker, half a character) is dropped with it.
        if self.in_paste {
            if tty_readable(tty.as_raw_fd(), Duration::ZERO) {
                self.quiet_since = None;
            } else if self
                .quiet_since
                .get_or_insert_with(std::time::Instant::now)
                .elapsed()
                >= PASTE_IDLE
            {
                self.in_paste = false;
                self.pending.clear();
                self.quiet_since = None;
            }
        }
        if self.pending.is_empty() {
            if !tty_readable(tty.as_raw_fd(), timeout) {
                return Ok(None);
            }
            let n = match tty.read(&mut buf) {
                Err(e) if hung_up(&e) => 0,
                read => read?,
            };
            if n == 0 {
                self.gone = true;
                return Ok(None);
            }
            self.pending.extend_from_slice(&buf[..n]);
            self.quiet_since = None;
        }
        // A key split across reads (an escape sequence on a slow link, a
        // UTF-8 character or a paste cut by a full read): wait briefly for
        // the rest, `KEY_SLICE` at most in all, so a paste trickling in
        // never holds the screen's lock long; its rest comes as more pieces.
        let until = std::time::Instant::now() + KEY_SLICE;
        while unfinished(&self.pending) {
            let left = until.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() || !tty_readable(tty.as_raw_fd(), left.min(ESCAPE_WAIT)) {
                break;
            }
            let n = match tty.read(&mut buf) {
                Err(e) if hung_up(&e) => break,
                read => read?,
            };
            if n == 0 {
                break;
            }
            self.pending.extend_from_slice(&buf[..n]);
            self.quiet_since = None;
        }
        if self.in_paste {
            return Ok(paste_chunk(&mut self.pending, &mut self.in_paste));
        }
        // A paste still coming after the wait (a terminal that sends a long
        // one in pieces): what came now, the rest as more of it.
        if self.pending.starts_with(PASTE_START) && find(&self.pending, PASTE_END).is_none() {
            self.pending.drain(..PASTE_START.len());
            self.in_paste = true;
            return Ok(paste_chunk(&mut self.pending, &mut self.in_paste));
        }
        // A late background answer still coming: held until it is whole.
        if self.pending.starts_with(b"\x1b]11;") && unfinished(&self.pending) {
            let since = *self
                .answer_since
                .get_or_insert_with(std::time::Instant::now);
            if since.elapsed() < ANSWER_WAIT {
                return Ok(None);
            }
        }
        self.answer_since = None;
        if let Some(used) = late_wheel_answer(&self.pending).or_else(|| {
            late_background_answer(&self.pending).inspect(|_| {
                if da1_reply(&self.pending).is_some_and(|r| r.start == 0) {
                    ANSWER_DUE.store(false, Ordering::SeqCst);
                }
            })
        }) {
            self.pending.drain(..used);
            return Ok(None);
        }
        let (name, used) = parse_key(&self.pending);
        self.pending.drain(..used);
        Ok(name)
    }
}

/// An answer to the background query (`take`'s OSC 11, then DA1) at the
/// start of `pending`, come after its wait (a slow link): never keys. Its
/// length, or None when `pending` does not start with one.
fn late_background_answer(pending: &[u8]) -> Option<usize> {
    osc11_reply(pending)
        .map(|(range, _)| range)
        .or_else(|| da1_reply(pending))
        .filter(|range| range.start == 0)
        .map(|range| range.end)
}

/// The answer to the alternate scroll query (`ESC[?1007;<n>$y`) at the
/// start of `pending`, come after its wait (a slow link): what release puts
/// back is then what the terminal had (1 or 3 set), not off. Its length,
/// or None when `pending` does not start with one.
fn late_wheel_answer(pending: &[u8]) -> Option<usize> {
    let rest = pending.strip_prefix(b"\x1b[?1007;")?;
    let end = find(rest, b"$y")?;
    let n = std::str::from_utf8(&rest[..end]).ok()?.parse::<u8>().ok()?;
    let _ = WHEEL.compare_exchange(
        WHEEL_ASKED,
        if matches!(n, 1 | 3) {
            WHEEL_WAS_ON
        } else {
            WHEEL_WAS_OFF
        },
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
    Some(b"\x1b[?1007;".len() + end + 2)
}

/// The next piece of a paste being read, as `paste:<text>`: up to its end
/// marker, which ends it, or what has come, less a tail that may be the
/// start of the end marker or of a UTF-8 character. None when that leaves
/// nothing yet.
fn paste_chunk(pending: &mut Vec<u8>, in_paste: &mut bool) -> Option<String> {
    let (text_len, used) = match find(pending, PASTE_END) {
        Some(end) => {
            *in_paste = false;
            (end, end + PASTE_END.len())
        }
        None => {
            let keep = held_back(pending);
            (pending.len() - keep, pending.len() - keep)
        }
    };
    let text = String::from_utf8_lossy(&pending[..text_len]).into_owned();
    pending.drain(..used);
    (!text.is_empty() || !*in_paste).then(|| format!("paste:{text}"))
}

/// How many bytes at the end of `bytes` to keep for the next read: a
/// proper prefix of the paste's end marker, or an unfinished UTF-8
/// character.
fn held_back(bytes: &[u8]) -> usize {
    let marker = (1..PASTE_END.len())
        .rev()
        .find(|&n| bytes.ends_with(&PASTE_END[..n]))
        .unwrap_or(0);
    let utf8 = (1..=3.min(bytes.len()))
        .find(|&n| {
            let start = bytes.len() - n;
            utf8_len(bytes[start]).is_some_and(|len| len > n)
        })
        .unwrap_or(0);
    marker.max(utf8)
}

/// The longest a key wait goes without looking at the screen again (and
/// failing once it was released).
const KEY_SLICE: Duration = Duration::from_millis(50);

/// How long a background answer arriving in pieces is held for its rest.
const ANSWER_WAIT: Duration = Duration::from_secs(1);

/// How long [`Keys::next`] waits for the rest of a key between reads.
const ESCAPE_WAIT: Duration = Duration::from_millis(30);

/// Bracketed paste's markers around pasted text (`take()` turns it on).
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// The most of a paste waited for at once; what came is handed out, and
/// the rest follows as more of it.
const MAX_PASTE: usize = 64 * 1024;

/// Whether `bytes` start a key that has not all arrived: `ESC` alone, `ESC
/// O`, `ESC [` without its final byte, a UTF-8 character short of its
/// continuation bytes, or a paste without its end marker.
fn unfinished(bytes: &[u8]) -> bool {
    if bytes.starts_with(PASTE_START) {
        return find(bytes, PASTE_END).is_none() && bytes.len() < MAX_PASTE;
    }
    // The background query's answer (`take`), whole before it is dropped.
    if bytes.starts_with(b"\x1b]11;") {
        return !bytes.contains(&0x07) && find(bytes, b"\x1b\\").is_none() && bytes.len() < 64;
    }
    match bytes {
        [0x1b] | [0x1b, b'O'] | [0x1b, b'O', b'2'] => true,
        [0x1b, b']'] | [0x1b, b']', b'1'] | [0x1b, b']', b'1', b'1'] => true,
        // A legacy mouse report: three raw bytes after `ESC [ M`.
        [0x1b, b'[', b'M', rest @ ..] => rest.len() < 3,
        [0x1b, b'[', rest @ ..] => {
            !rest.iter().any(|b| (0x40..=0x7e).contains(b)) || PASTE_START.starts_with(bytes)
        }
        [lead, ..] => utf8_len(*lead).is_some_and(|n| bytes.len() < n),
        [] => false,
    }
}

/// A mouse report's button as a wheel key; None for any other button. Shift,
/// Alt and Ctrl held (bits 4, 8 and 16) do not change it.
fn wheel(button: u32) -> Option<&'static str> {
    match button & !0b11100 {
        64 => Some("wheel-up"),
        65 => Some("wheel-down"),
        _ => None,
    }
}

/// A mouse report as a key: the wheel's, or the left button's
/// `mouse-press`, `mouse-drag` (moved with it held) and `mouse-release`, and
/// a move with no button held (`mouse-move`), at
/// `:<column>:<row>` from 0 (the report's `x` and `y` count from 1). Shift,
/// Alt and Ctrl held do not change it. None for another button. `release`
/// is SGR's own (`m`); a legacy report's release is button 3, which names
/// no button.
fn mouse_key(button: u32, x: u32, y: u32, release: bool) -> Option<String> {
    if let Some(name) = wheel(button) {
        return (!release).then(|| name.to_string());
    }
    let b = button & !0b11100;
    let at = |kind: &str| {
        format!(
            "mouse-{kind}:{}:{}",
            x.saturating_sub(1),
            y.saturating_sub(1)
        )
    };
    match b {
        _ if release && b & 0b11 == 0 => Some(at("release")),
        3 => Some(at("release")),
        0 => Some(at("press")),
        32 => Some(at("drag")),
        35 => Some(at("move")),
        _ => None,
    }
}

/// An arrow's direction from its final byte, `A` to `D`.
fn arrow(d: u8) -> &'static str {
    match d {
        b'A' => "up",
        b'B' => "down",
        b'C' => "right",
        _ => "left",
    }
}

/// How many bytes the UTF-8 character `lead` starts takes; None for a byte
/// that starts none (a stray continuation byte).
fn utf8_len(lead: u8) -> Option<usize> {
    match lead {
        0x00..=0x7f => Some(1),
        0xc0..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf7 => Some(4),
        _ => None,
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The first key in `bytes` and how many bytes it took.
fn parse_key(bytes: &[u8]) -> (Option<String>, usize) {
    let named = |s: &str, n: usize| (Some(s.to_string()), n);
    match bytes {
        [] => (None, 0),
        // A bracketed paste, whole: `paste:<text>`, so a newline in it is
        // never read as Enter. (`key()` hands out one not yet ended in
        // pieces, before this.)
        _ if bytes.starts_with(PASTE_START) => {
            let body = &bytes[PASTE_START.len()..];
            let (text, used) = match find(body, PASTE_END) {
                Some(end) => (&body[..end], PASTE_START.len() + end + PASTE_END.len()),
                None => (body, bytes.len()),
            };
            (
                Some(format!("paste:{}", String::from_utf8_lossy(text))),
                used,
            )
        }
        [0x1b, b'[', b'A', ..] | [0x1b, b'O', b'A', ..] => named("up", 3),
        [0x1b, b'[', b'B', ..] | [0x1b, b'O', b'B', ..] => named("down", 3),
        [0x1b, b'[', b'C', ..] | [0x1b, b'O', b'C', ..] => named("right", 3),
        [0x1b, b'[', b'D', ..] | [0x1b, b'O', b'D', ..] => named("left", 3),
        [0x1b, b'[', b'H', ..] | [0x1b, b'O', b'H', ..] => named("home", 3),
        [0x1b, b'[', b'Z', ..] => named("shift-tab", 3),
        [0x1b, b'[', b'F', ..] | [0x1b, b'O', b'F', ..] => named("end", 3),
        // xterm's modified arrows: `ESC [ 1 ; <mod> <dir>`, 2 for Shift.
        [
            0x1b,
            b'[',
            b'1',
            b';',
            b'2',
            d @ (b'A' | b'B' | b'C' | b'D'),
            ..,
        ] => (Some(format!("shift-{}", arrow(*d))), 6),
        // Alt (Option) with an arrow: xterm's `ESC [ 1 ; 3 <dir>`, and an ESC
        // before the arrow where Option is Meta (the macOS Terminal).
        [
            0x1b,
            b'[',
            b'1',
            b';',
            b'3',
            d @ (b'A' | b'B' | b'C' | b'D'),
            ..,
        ] => (Some(format!("alt-{}", arrow(*d))), 6),
        [0x1b, 0x1b, b'[' | b'O', d @ (b'A' | b'B' | b'C' | b'D'), ..] => {
            (Some(format!("alt-{}", arrow(*d))), 4)
        }
        // Shift+arrows as other terminals send them: older xterm's
        // `ESC [ 2 <dir>` and `ESC O 2 <dir>`, rxvt's `ESC [ a`…`d`.
        [0x1b, b'[' | b'O', b'2', d @ (b'A' | b'B' | b'C' | b'D'), ..] => {
            (Some(format!("shift-{}", arrow(*d))), 4)
        }
        [0x1b, b'[', d @ (b'a' | b'b' | b'c' | b'd'), ..] => {
            (Some(format!("shift-{}", arrow(d.to_ascii_uppercase()))), 3)
        }
        // SS3 (`ESC O x`), F1-F4 among them: three bytes, unnamed unless
        // above.
        [0x1b, b'O', _, ..] => (None, 3),
        [0x1b, b'[', b'1' | b'7', b'~', ..] => named("home", 4),
        [0x1b, b'[', b'4' | b'8', b'~', ..] => named("end", 4),
        [0x1b, b'[', b'5', b'~', ..] => named("pageup", 4),
        [0x1b, b'[', b'6', b'~', ..] => named("pagedown", 4),
        // A mouse report (`mouse(True)`): SGR's `ESC [ < b ; x ; y M` (or
        // `m`, a release), or the legacy `ESC [ M` and three bytes, each
        // offset by 32, from a terminal without SGR ([`mouse_key`]).
        [0x1b, b'[', b'<', rest @ ..] => {
            let Some(end) = rest.iter().position(|b| (0x40..=0x7e).contains(b)) else {
                return (None, bytes.len());
            };
            let fields: Vec<u32> = std::str::from_utf8(&rest[..end])
                .ok()
                .map(|p| p.split(';').filter_map(|f| f.parse().ok()).collect())
                .unwrap_or_default();
            let name = match (fields.as_slice(), rest[end]) {
                (&[b, x, y], f @ (b'M' | b'm')) => mouse_key(b, x, y, f == b'm'),
                _ => None,
            };
            (name, end + 4)
        }
        [0x1b, b'[', b'M', b, x, y, ..] => {
            let field = |v: u8| u32::from(v.wrapping_sub(32));
            (mouse_key(field(*b), field(*x), field(*y), false), 6)
        }
        [0x1b, b'[', rest @ ..] => {
            // Some other sequence: skip it whole, to its final byte.
            let end = rest.iter().position(|b| (0x40..=0x7e).contains(b));
            (None, end.map_or(bytes.len(), |i| i + 3))
        }
        // Alt (Option, where the terminal sends it as Meta) with a key: ESC
        // then the key, read together.
        [0x1b, c @ 0x20..=0x7e, ..] => (Some(format!("alt-{}", *c as char)), 2),
        [0x1b, 0x7f | 0x08, ..] => named("alt-backspace", 2),
        [0x1b, ..] => named("esc", 1),
        [b'\r', ..] | [b'\n', ..] => named("enter", 1),
        [b'\t', ..] => named("tab", 1),
        [0x7f, ..] | [0x08, ..] => named("backspace", 1),
        [c @ 1..=26, ..] => (Some(format!("ctrl-{}", (b'a' + c - 1) as char)), 1),
        [0, ..] => named("ctrl-@", 1),
        [0x1c, ..] => named("ctrl-\\", 1),
        [0x1d, ..] => named("ctrl-]", 1),
        [0x1e, ..] => named("ctrl-^", 1),
        [0x1f, ..] => named("ctrl-_", 1),
        // A continuation byte with no lead: skipped alone, never taking the
        // bytes after it.
        [lead, ..] if utf8_len(*lead).is_none() => (None, 1),
        _ => {
            let len = utf8_len(bytes[0]).unwrap_or(1).min(bytes.len());
            (
                Some(String::from_utf8_lossy(&bytes[..len]).into_owned()),
                len,
            )
        }
    }
}

/// `ctx.std.term`.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.Term>")]
pub struct Term;

starlark_simple_value!(Term);

#[starlark_value(type = "std.Term")]
impl<'v> values::StarlarkValue<'v> for Term {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("term_methods", term_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn term_methods(registry: &mut MethodsBuilder) {
    /// The terminal's size as `(columns, rows)`, or None without one a
    /// screen could take: none at all, or this process in the background.
    fn size<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<NoneOr<(i32, i32)>> {
        Ok(NoneOr::from_option(terminal_size()))
    }

    /// Write to stderr, once a screen of its own is given back, the task's
    /// output since it was taken, as the task's `output.ansi` keeps it
    /// (redacted, its colour kept, its links made again): the scrollback a
    /// run without the screen would have. Its newest `max_lines`, after a
    /// line `older` gives (`{n}` the count left out). Written in chunks, no
    /// child's stderr lines between them, and given up after 2s of stderr
    /// taking nothing; not copied to the output files, which hold it. Whether it wrote: once
    /// per screen, never without the task's files (the history off), when
    /// the screen's own lines are the fallback; true too when the screen's
    /// `release(dump_lines = …)`, or a suspend still in force, wrote it.
    fn dump_output<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = DUMP_LINES as i32)] max_lines: i32,
        #[starlark(require = named, default = "")] older: &str,
    ) -> anyhow::Result<bool> {
        Ok(dump_output(max_lines.max(1) as usize, older))
    }

    /// Keep `text` in the task's output files (`output.log`, `output.ansi`)
    /// without writing it on a screen: for a line the terminal alone shows,
    /// written while a screen holds it, which the scrollback the screen
    /// leaves (`dump_output`) then has in its place. With the screen given
    /// back (a prompt), or none, it is written to stderr too. Decided, written
    /// and copied under stderr's turn, which every screen switch holds: the
    /// copy lands on the side of a dump that matches where the text went.
    fn keep_output<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
    ) -> anyhow::Result<NoneType> {
        // Decided, and copied, under stderr's turn: a screen is taken or
        // given back only with it held, so the copy lands on the side of
        // the dump that matches where the text went.
        let _turn = axl_types::terminal::turn(2);
        if SINK.lock().unwrap_or_else(|p| p.into_inner()).is_none() {
            use std::io::Write as _;
            let _ = std::io::stderr().write_all(text.as_bytes());
            axl_types::terminal::wrote(2, text.as_bytes());
        }
        crate::out::copy_only(text.as_bytes());
        Ok(NoneType)
    }

    /// How many URLs the scrollback a screen left (`dump_output`) has
    /// written out beside its links, as a terminal that shows no link (the
    /// macOS Terminal) has them: a build's console wrote none out while the
    /// screen held the terminal. 0 elsewhere.
    #[starlark(attribute)]
    fn links_written<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<u64> {
        Ok(DUMP_WRITTEN.load(Ordering::SeqCst))
    }

    /// Whether a screen is held now, by any feature.
    #[starlark(attribute)]
    fn taken<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<bool> {
        Ok(HELD.lock().unwrap().is_some())
    }

    /// Take the terminal for a screen of your own; see `std.term`. Fails
    /// when there is no terminal, another screen holds it, or this process
    /// runs in the background (another job has the terminal).
    ///
    /// # Arguments
    /// * `alt_screen` - Draw on the terminal's alternate screen, leaving the
    ///   scrollback as it was. Default True.
    fn take<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named, default = true)] alt_screen: bool,
    ) -> anyhow::Result<Screen> {
        Ok(Screen {
            generation: take(alt_screen)?,
        })
    }
}

/// A screen taken with `ctx.std.term.take()`. Once released, its methods
/// fail, even after another screen is taken, but for `release()`, a no-op,
/// and `held`, False.
#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.term.Screen>")]
pub struct Screen {
    generation: u64,
}

starlark_simple_value!(Screen);

#[starlark_value(type = "std.term.Screen")]
impl<'v> values::StarlarkValue<'v> for Screen {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("screen_methods", screen_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn screen_methods(registry: &mut MethodsBuilder) {
    /// Write `text` to the terminal as it is, escape sequences and all, and
    /// flush. Draw a frame in one write: it is sent as one synchronized
    /// update (`?2026`, [`synchronized`]), which a terminal that knows it
    /// (iTerm2, kitty, WezTerm, Ghostty, Windows Terminal) shows whole, and
    /// any other ignores. On a terminal gone (closed, a hangup) it is
    /// dropped: the task's cancellation says so.
    fn write<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
    ) -> anyhow::Result<NoneType> {
        with_held(this, |h| {
            h.last_frame = text.as_bytes().to_vec();
            h.frame_size = size_of(h.tty.as_raw_fd());
            if h.suspended {
                return Ok(NoneType);
            }
            gone_is_fine(
                h.tty
                    .write_all(&synchronized(text.as_bytes()))
                    .and_then(|()| h.tty.flush()),
            )
        })
    }

    /// The terminal's size now as `(columns, rows)`; read it every frame to
    /// follow a resize.
    fn size<'v>(this: values::Value<'v>) -> anyhow::Result<(i32, i32)> {
        with_held(this, |h| {
            Ok(size_of(h.tty.as_raw_fd()).unwrap_or(FALLBACK_SIZE))
        })
    }

    /// The next key typed, by name, waiting up to `timeout_ms` (default 0,
    /// not at all); None when none was, or the terminal is gone (closed, a
    /// hangup, which the task's cancellation says). A printable character is itself;
    /// otherwise `up`, `down`, `left`, `right`, `shift-up`, `shift-down`,
    /// `shift-left`, `shift-right`, `pageup`, `pagedown`, `home`,
    /// `end`, `enter`, `tab`, `backspace`, `esc`, `ctrl-<letter>`,
    /// `alt-<key>` (where the terminal sends Alt as Meta, and for an Esc
    /// typed just before a key), `alt-backspace`, `paste:<text>` (pasted text: bracketed
    /// paste is on while the screen is held; a long paste comes as several,
    /// the last maybe empty, `pasting` True until it has), `wheel-up` and
    /// `wheel-down` (the mouse's wheel, once `mouse(True)`), and its left
    /// button's `mouse-press:<col>:<row>`, `mouse-move:<col>:<row>` (the
    /// pointer moved, no button held), `mouse-drag:<col>:<row>` (moved
    /// while held) and `mouse-release:<col>:<row>`, from 0 at the top left
    /// (its other buttons are skipped), or `ctrl-@`,
    /// `ctrl-\`, `ctrl-]`, `ctrl-^`, `ctrl-_`. A key it does not
    /// know (a function key) is skipped.
    fn key<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = 0)] timeout_ms: u32,
    ) -> anyhow::Result<NoneOr<String>> {
        // The wait runs without the screen's lock, which the exit path may
        // need to give the terminal back; only reading (with a wait for the rest
        // of a key, `KEY_SLICE` at most) and parsing hold it.
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms as u64);
        loop {
            let (fd, got, gone) = with_held(this, |h| {
                // Given back, the keys are the prompt's or the child's.
                if h.suspended {
                    return Ok((h.tty.as_raw_fd(), None, true));
                }
                let got = h.keys.take(&mut h.tty)?;
                Ok((h.tty.as_raw_fd(), got, h.keys.gone))
            })?;
            if got.is_some() {
                return Ok(NoneOr::from_option(got));
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Ok(NoneOr::None);
            }

            // A terminal gone reads as ready at once: the wait is slept out
            // rather than spun.
            if gone {
                std::thread::sleep(left);
                return Ok(NoneOr::None);
            }
            tty_readable(fd, left.min(KEY_SLICE));
        }
    }

    /// The lines the process wrote to its stdout and stderr since the last
    /// call, escape sequences stripped: the CLI's own lines, `print()` and
    /// diagnostics, which `release()` also writes to the scrollback.
    /// With `with_colour`, each as `[line, links, coloured]`, `coloured` the
    /// line with its colour kept, the same text. With `with_links`, each as
    /// `[line, links]`, `links` the line's OSC 8
    /// links (`print()` links URLs and paths) as `[start, end, url]` by
    /// character, as `output_feed()` gives a build's.
    fn output<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = false)] with_links: bool,
        #[starlark(require = named, default = false)] with_colour: bool,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let got = with_held(this, |h| Ok(h.capture.drain()))?;
        Ok(if with_colour {
            heap.alloc(serde_json::json!(
                got.lines
                    .into_iter()
                    .zip(got.links)
                    .zip(got.colours)
                    .map(|((line, links), coloured)| serde_json::json!([line, links, coloured]))
                    .collect::<Vec<_>>()
            ))
        } else if with_links {
            heap.alloc(serde_json::json!(
                got.lines.into_iter().zip(got.links).collect::<Vec<_>>()
            ))
        } else {
            heap.alloc(got.lines)
        })
    }

    /// What the task wrote before Bazel's console began, as `[line, links,
    /// coloured]` like `output(with_colour = True)`, its links made again
    /// (the banner's URL among them): for a screen that shows the task's
    /// output from its start, before any of Bazel's lines. Once: a second
    /// call gives none.
    fn earlier<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let bytes = with_held(this, |h| Ok(std::mem::take(&mut h.earlier)))?;
        let feed = Feed::default();
        feed.write(Source::Stderr, &relinked(&bytes));
        feed.finish(Source::Stderr);
        let got = feed.drain();
        Ok(heap.alloc(serde_json::json!(
            got.lines
                .into_iter()
                .zip(got.links)
                .zip(got.colours)
                .map(|((line, links), coloured)| serde_json::json!([line, links, coloured]))
                .collect::<Vec<_>>()
        )))
    }

    /// Give the terminal back: its modes, the main screen, and the process's
    /// stdout and stderr, to which everything captured is then written:
    /// stderr's only with `replay_stderr`, for a screen that writes what it
    /// showed itself (its output having been part of it); then stdout's
    /// too only where stdout is not the terminal, and the lines the
    /// captures still held as they stopped are returned, with their colour,
    /// for the screen to add. A no-op once released, returning none.
    ///
    /// With `dump_lines` above 0, the scrollback (`dump_output`, its newest
    /// `dump_lines` after the line `older` gives) is written as part of the
    /// release, nothing else written between the main screen coming back
    /// and it; a later `dump_output` then says it was written.
    fn release<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = true)] replay_stderr: bool,
        #[starlark(require = named, default = 0)] dump_lines: i32,
        #[starlark(require = named, default = "")] older: &str,
    ) -> anyhow::Result<Vec<String>> {
        let generation = generation_of(this)?;
        // No forwarder writes while the screen goes and its capture is
        // replayed: its lines would land inside the replay's.
        let mut order = Order::lock();
        let mut held = HELD.lock().unwrap();
        if held.as_ref().is_some_and(|h| h.generation == generation) {
            let mine = held.take();
            drop(held);
            let dump = (dump_lines > 0 && dump_ready()).then_some((dump_lines as usize, older));
            return Ok(release(mine, replay_stderr, dump, &mut order));
        }
        Ok(vec![])
    }

    /// Interrupt as Ctrl+C does: SIGINT to this process's group, this
    /// process and the Bazel client with it, so the task is cancelled the way
    /// the user's own Ctrl+C cancels it (the CLI takes a signal it sent its
    /// own group as the terminal's).
    fn interrupt<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        with_held(this, |_| {
            // SAFETY: a signal to this process's own group.
            unsafe { libc::killpg(libc::getpgrp(), libc::SIGINT) };
            Ok(NoneType)
        })
    }

    /// Whether Ctrl+C interrupts (True, the default) or arrives from
    /// `key()` as `ctrl-c`: for a screen that stays up once its build is
    /// done, where an interrupt would cancel a task that has already ended.
    /// Off, Ctrl+Z and Ctrl+\ do not signal either: they arrive as `ctrl-z`
    /// and `ctrl-\`.
    fn interrupts<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] enabled: bool,
    ) -> anyhow::Result<NoneType> {
        with_held(this, |h| {
            let set = termios::tcgetattr(&h.tty).and_then(|mut t| {
                t.local_flags.set(LocalFlags::ISIG, enabled);
                termios::tcsetattr(&h.tty, SetArg::TCSANOW, &t)
            });
            gone_is_fine(set.map_err(std::io::Error::from))
        })
    }

    /// Whether the terminal sends its wheel as `up` and `down` keys while it
    /// keeps the mouse (alternate scroll, `?1007`; on the alternate screen
    /// only): the wheel scrolls what the arrows do, and selecting text and
    /// clicking links stay the terminal's, with no modifier. Most terminals
    /// support it (iTerm2, Terminal.app, xterm, GNOME, Windows Terminal);
    /// where one does not, its wheel does what it does on an alternate
    /// screen. Off by default; `mouse(True)` takes the wheel over while on.
    /// `release()` puts it back as the terminal had it, which the first call
    /// asks the terminal (DECRQM; on in some, as in VTE's), else off.
    fn wheel_keys<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] enabled: bool,
    ) -> anyhow::Result<NoneType> {
        with_held(this, |h| {
            // Asked once a screen, before the first change.
            if WHEEL.load(Ordering::SeqCst) == WHEEL_UNTOUCHED {
                let was = alternate_scroll(&mut h.tty, &mut h.keys);
                WHEEL.store(
                    match was {
                        Some(true) => WHEEL_WAS_ON,
                        Some(false) => WHEEL_WAS_OFF,
                        None => WHEEL_ASKED,
                    },
                    Ordering::SeqCst,
                );
            }
            gone_is_fine(
                h.tty
                    .write_all(if enabled {
                        b"\x1b[?1007h"
                    } else {
                        b"\x1b[?1007l"
                    })
                    .and_then(|()| h.tty.flush()),
            )
        })
    }

    /// Whether the terminal reports the mouse (True) or keeps it for itself
    /// (False, the default): reported, its wheel, the pointer's moves and
    /// its left button's presses, drags and releases arrive from `key()`, so
    /// the screen selects, lights and opens links itself; the terminal's own selection then
    /// takes a modifier it names (Shift, Option or Fn with the drag), and
    /// its wheel no longer scrolls past the screen. `release()` turns it off.
    /// For the wheel with the mouse left to the terminal, see `wheel_keys`.
    fn mouse<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] enabled: bool,
    ) -> anyhow::Result<NoneType> {
        with_held(this, |h| {
            h.mouse_on = enabled;
            if h.suspended {
                return Ok(NoneType);
            }
            let set: &[u8] = if enabled {
                b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h"
            } else {
                b"\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l"
            };
            gone_is_fine(h.tty.write_all(set).and_then(|()| h.tty.flush()))
        })
    }

    /// Whether a paste `key()` has handed out part of is not over yet: the
    /// terminal has sent its start and not its end (a long or slow one
    /// comes as several `paste:` keys). False with no screen held.
    #[starlark(attribute)]
    fn pasting<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let generation = generation_of(this)?;
        Ok(HELD
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|h| h.generation == generation && h.keys.in_paste))
    }

    /// Whether this screen still holds the terminal.
    #[starlark(attribute)]
    fn held<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let generation = generation_of(this)?;
        Ok(HELD
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|h| h.generation == generation))
    }
}

#[cfg(test)]
mod tests {

    /// What a dump re-links is counted when it writes a file's URL out (the
    /// macOS Terminal, where the links tip is due), and not when it only
    /// links it (iTerm2, VS Code, Cursor), nor for text naming no file.
    #[test]
    fn a_dump_counts_the_urls_it_writes_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        for (written, want) in [(true, 1), (false, 0)] {
            let count = bazel::console::links::WrittenCount::default();
            let mut linker = bazel::console::links::Linker::new(vec![dir.path().to_path_buf()])
                .with_files(
                    bazel::console::links::FileLinks::new(
                        bazel::console::links::LinkFormat::parse("vscode").unwrap(),
                    )
                    .written(written)
                    .counted(count.clone()),
                );
            linker.link_written(b"see a.txt:1");
            linker.link_written(b"nothing to link here");
            assert_eq!(count.get(), want, "written out: {written}");
        }
    }

    /// A seeded line's URL, linked as the screen's linker links it (no URL
    /// written out), reaches the feed as a link the screen can open.
    #[test]
    fn a_seeded_url_is_a_link_in_the_feed() {
        let mut linker = bazel::console::links::Linker::new(vec![std::env::temp_dir()])
            .with_files(bazel::console::links::FileLinks::none().written(false));
        let line = linker.link_written(b"Aspect CLI - https://aspect.build/go/cli");
        assert!(
            !String::from_utf8_lossy(&line).contains("<https"),
            "no URL written out"
        );
        let feed = Feed::default();
        feed.write(Source::Stderr, &[line, b"\n".to_vec()].concat());
        let got = feed.drain();
        assert_eq!(got.lines, ["Aspect CLI - https://aspect.build/go/cli"]);
        assert!(
            got.links[0]
                .iter()
                .any(|(_, _, url)| url == "https://aspect.build/go/cli"),
            "{:?}",
            got.links
        );
    }

    /// A dump keeps the newest lines, counts the rest on a line before
    /// them, and links each line without its end.
    #[test]
    fn a_dump_keeps_the_newest_lines_linked() {
        let text = "one\ntwo\nthree\nfour";
        assert_eq!(
            super::dump_text(text, 2, "… {n} older", |l| format!("[{l}]")),
            "… 2 older\n[three]\n[four]"
        );
        assert_eq!(
            super::dump_text(text, 9, "… {n} older", |l| l.to_string()),
            text
        );
        assert_eq!(super::dump_text("a\nb\n", 1, "", |l| l.to_string()), "b\n");
    }

    use super::*;

    /// A pipe closed on exec as it is made (`pipe2` on Linux), so a child
    /// another test spawns meanwhile (a `sleep 600`) never holds its write
    /// end and keeps a read here from ending. macOS has no `pipe2`: there a
    /// spawn in the instant before the flag is set still could.
    fn private_pipe() -> (OwnedFd, OwnedFd) {
        let (read, write) = std::io::pipe().unwrap();
        (OwnedFd::from(read), OwnedFd::from(write))
    }

    #[test]
    fn the_background_is_read_from_its_answer() {
        let dark = b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\\x1b[?62;22cq";
        let (range, is_dark) = osc11_reply(dark).unwrap();
        assert_eq!((range, is_dark), (0..25, Some(true)));
        assert_eq!(da1_reply(dark), Some(25..34));
        let light = b"\x1b]11;rgb:ffff/ffff/ffff\x07";
        assert_eq!(osc11_reply(light).unwrap().1, Some(false));
        assert_eq!(
            osc11_reply(b"\x1b]11;rgb:f/f/f\x07").unwrap().1,
            Some(false)
        );
        assert_eq!(osc11_reply(b"\x1b]11;bogus\x07").unwrap().1, None);
        assert!(osc11_reply(b"\x1b]11;rgb:00/00").is_none(), "unended");
        assert!(da1_reply(b"\x1b[?62;22").is_none());
    }

    /// A terminal that answers both: the background, and a key typed
    /// meanwhile kept for the keys. One that answers DA1 alone: unknown, at
    /// once.
    #[test]
    fn a_background_query_keeps_what_was_typed() {
        let pty = private_pty();
        let mut master = File::from(pty.master);
        let mut tty = File::from(pty.slave);
        let answer = std::thread::spawn(move || {
            let mut asked = [0u8; 64];
            let _ = std::io::Read::read(&mut master, &mut asked);
            master
                .write_all(b"x\x1b]11;rgb:0000/0000/0000\x07\x1b[?1;2c")
                .unwrap();
            master
        });
        let mut raw = termios::tcgetattr(&tty).unwrap();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&tty, SetArg::TCSANOW, &raw).unwrap();
        let (dark, typed) = query_background(&mut tty, Duration::from_secs(5));
        assert_eq!((dark, typed), (Some(true), b"x".to_vec()));
        let mut master = answer.join().unwrap();
        let answer = std::thread::spawn(move || {
            let mut asked = [0u8; 64];
            let _ = std::io::Read::read(&mut master, &mut asked);
            master.write_all(b"\x1b[?1;2c").unwrap();
            master
        });
        let started = std::time::Instant::now();
        assert_eq!(
            query_background(&mut tty, Duration::from_secs(5)),
            (None, vec![])
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "ended by the DA1 answer"
        );
        drop(answer.join());
    }

    /// A pty whose descriptors are closed on exec, so a child another test
    /// spawns meanwhile never holds the master open and hides its hangup.
    /// `openpty` takes no flag for it: a spawn in the instant before it is
    /// set still could.
    fn private_pty() -> nix::pty::OpenptyResult {
        let pty = nix::pty::openpty(None, None).unwrap();
        for fd in [&pty.master, &pty.slave] {
            nix::fcntl::fcntl(
                fd,
                nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
            )
            .unwrap();
        }
        pty
    }

    /// Taking the screen again and again keeps the same three descriptors
    /// for a crash, rather than three more each time.
    #[test]
    fn the_crash_state_is_reused_across_takes() {
        let pty = private_pty();
        let tty = pty.slave.as_raw_fd();
        let termios = nix::sys::termios::tcgetattr(&pty.slave).unwrap();
        let kept = || {
            // SAFETY: the one leaked state, not written meanwhile.
            let s = unsafe { &*CRASH.load(Ordering::SeqCst) };
            (s.tty, s.stdout, s.stderr, s.alt_screen)
        };
        remember_for_crash(tty, &termios, true, 1, 2);
        let (t, o, e, _) = kept();
        for _ in 0..20 {
            remember_for_crash(tty, &termios, false, 1, 2);
        }
        assert_eq!(kept(), (t, o, e, false));
        for fd in [t, o, e] {
            assert!(unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC != 0);
        }
        CRASH.store(std::ptr::null_mut(), Ordering::SeqCst);
    }

    /// A terminal gone (its pty master closed) neither fails a write nor a
    /// key read: what a hangup leaves a screen with.
    #[test]
    fn a_hung_up_terminal_is_no_error() {
        let pty = private_pty();
        drop(pty.master);
        let mut tty = std::fs::File::from(pty.slave);
        let wrote = std::io::Write::write_all(&mut tty, b"frame");
        assert!(gone_is_fine(wrote).is_ok());
        let mut buf = [0u8; 8];
        match std::io::Read::read(&mut tty, &mut buf) {
            Ok(0) => {}
            Ok(n) => panic!("read {n} bytes from a closed terminal"),
            Err(e) => assert!(hung_up(&e), "{e}"),
        }
    }

    /// Keys on a terminal gone remember it, so `key()` sleeps out its wait
    /// rather than spinning on a terminal that reads as ready.
    #[test]
    fn keys_on_a_hung_up_terminal_remember_it() {
        let pty = private_pty();
        drop(pty.master);
        let mut tty = std::fs::File::from(pty.slave);
        let mut keys = Keys::default();
        assert_eq!(
            keys.next(&mut tty, Duration::from_millis(50)).unwrap(),
            None
        );
        assert!(keys.gone);
    }

    /// A late background answer split across reads is held until whole,
    /// then dropped: none of it reads as keys; a key after it does.
    #[test]
    fn a_late_answer_in_pieces_is_no_keys() {
        let pty = private_pty();
        let mut master = File::from(pty.master);
        let mut tty = File::from(pty.slave);
        let mut raw = termios::tcgetattr(&tty).unwrap();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&tty, SetArg::TCSANOW, &raw).unwrap();
        let mut keys = Keys::default();
        master.write_all(b"\x1b]11;rgb:1e1").unwrap();
        let mut got = vec![];
        for _ in 0..3 {
            got.extend(keys.next(&mut tty, Duration::from_millis(30)).unwrap());
        }
        master.write_all(b"e/1e1e/1e1e\x1b\\\x1b[?62;22cq").unwrap();
        for _ in 0..6 {
            got.extend(keys.next(&mut tty, Duration::from_millis(30)).unwrap());
        }
        assert_eq!(got, vec!["q".to_string()]);
    }

    /// The alternate scroll answer, come late among the keys, is taken for
    /// what release puts back and not read as a key.
    #[test]
    fn a_late_wheel_answer_settles_what_release_puts_back() {
        WHEEL.store(WHEEL_ASKED, Ordering::SeqCst);
        assert_eq!(late_wheel_answer(b"\x1b[?1007;1$yq"), Some(11));
        assert_eq!(wheel_restore(), b"\x1b[?1007h");
        assert_eq!(late_wheel_answer(b"q"), None);
        assert_eq!(
            late_background_answer(b"\x1b]11;rgb:1e1e/1e1e/1e1e\x07q"),
            Some(24)
        );
        assert_eq!(late_background_answer(b"\x1b[?62;22cq"), Some(9));
        assert_eq!(
            late_background_answer(b"q\x1b[?62;22c"),
            None,
            "only at the start"
        );
        assert!(
            unfinished(b"\x1b]11;rgb:1e1"),
            "an answer split across reads waits"
        );
        assert!(unfinished(b"\x1b]1"));
        assert!(!unfinished(b"\x1b]11;rgb:1e1e/1e1e/1e1e\x07"));
        WHEEL.store(WHEEL_UNTOUCHED, Ordering::SeqCst);
    }

    #[test]
    fn a_frame_is_one_synchronized_update() {
        assert_eq!(synchronized(b"ab"), b"\x1b[?2026hab\x1b[?2026l");
        // One cut short is closed when the terminal is given back.
        assert!(leave_sequence(true).starts_with(b"\x1b[?2026l"));
        assert!(leave_sequence(false).starts_with(b"\x1b[?2026l"));
    }

    #[test]
    fn the_wheel_and_the_left_button_are_named_and_other_buttons_skipped() {
        let k = |b: &[u8]| parse_key(b);
        let some = |s: &str| Some(s.to_string());
        assert_eq!(k(b"\x1b[<64;10;5Mq"), (some("wheel-up"), 11));
        assert_eq!(k(b"\x1b[<65;1;1M"), (some("wheel-down"), 10));
        assert_eq!(k(b"\x1b[<69;1;1M"), (some("wheel-down"), 10), "with Shift");
        assert_eq!(k(b"\x1b[<0;3;4Mq"), (some("mouse-press:2:3"), 9), "a click");
        assert_eq!(k(b"\x1b[<32;5;4M"), (some("mouse-drag:4:3"), 10), "a drag");
        assert_eq!(
            k(b"\x1b[<0;6;4m"),
            (some("mouse-release:5:3"), 9),
            "a release"
        );
        assert_eq!(
            k(b"\x1b[<4;3;4M"),
            (some("mouse-press:2:3"), 9),
            "with Shift"
        );
        assert_eq!(k(b"\x1b[<2;3;4M"), (None, 9), "the right button");
        assert_eq!(k(b"\x1b[<2;3;4m"), (None, 9), "its release");
        assert_eq!(
            k(b"\x1b[<35;3;4M"),
            (some("mouse-move:2:3"), 10),
            "a move with no button"
        );
        assert_eq!(k(b"\x1b[<64;3;4m"), (None, 10));
        assert_eq!(k(b"\x1b[M`!!q"), (some("wheel-up"), 6), "legacy");
        assert_eq!(
            k(b"\x1b[M !!q"),
            (some("mouse-press:0:0"), 6),
            "a legacy click"
        );
        assert_eq!(
            k(b"\x1b[M#\"!"),
            (some("mouse-release:1:0"), 6),
            "a legacy release"
        );
        assert_eq!(
            k(b"\x1b[M@#\""),
            (some("mouse-drag:2:1"), 6),
            "a legacy drag"
        );
        assert!(unfinished(b"\x1b[<64;10"));
        assert!(unfinished(b"\x1b[M`!"));
        assert!(!unfinished(b"\x1b[M`!!"));
    }

    #[test]
    fn keys_are_named() {
        let k = |b: &[u8]| parse_key(b);
        assert_eq!(k(b"q"), (Some("q".into()), 1));
        assert_eq!(k(b"\x1b[A"), (Some("up".into()), 3));
        assert_eq!(k(b"\x1b[6~x"), (Some("pagedown".into()), 4));
        assert_eq!(k(b"\x03"), (Some("ctrl-c".into()), 1));
        assert_eq!(k(b"\r"), (Some("enter".into()), 1));
        assert_eq!(k(b"\x1bc"), (Some("alt-c".into()), 2));
        assert_eq!(
            k(b"\x1b[200~a\nq\x1b[201~j"),
            (Some("paste:a\nq".into()), 15)
        );
        // A stray continuation byte takes itself alone.
        assert_eq!(k(b"\x80jk"), (None, 1));
        assert_eq!(k("é".as_bytes()), (Some("é".into()), 2));
        assert_eq!(k(b"\x1b"), (Some("esc".into()), 1));
        assert_eq!(k(b"\t"), (Some("tab".into()), 1));
        assert_eq!(k(b"\x1b"), (Some("esc".into()), 1));
        assert_eq!(k(b"\x1b[1;2A"), (Some("shift-up".into()), 6));
        assert_eq!(k(b"\x1b[2A"), (Some("shift-up".into()), 4));
        assert_eq!(k(b"\x1bO2B"), (Some("shift-down".into()), 4));
        assert_eq!(k(b"\x1b[a"), (Some("shift-up".into()), 3));
        assert_eq!(k(b"\x1b[b"), (Some("shift-down".into()), 3));
        // `ESC [ 2 ~` is still Insert, unnamed, not Shift.
        assert_eq!(k(b"\x1b[2~").0, None);
        assert_eq!(k(b"\x1bOP"), (None, 3), "F1 is not Esc");
        assert_eq!(
            k(b"\x1c"),
            (Some("ctrl-\\".into()), 1),
            "Ctrl+\\ with interrupts off"
        );
        assert_eq!(k(b"\x1b[1~"), (Some("home".into()), 4), "tmux");
        assert_eq!(k(b"\x1b[4~"), (Some("end".into()), 4), "tmux");
        assert_eq!(k(b"\x1b[8~"), (Some("end".into()), 4), "rxvt");
        assert_eq!(k(b"\x1b[1;2Bx"), (Some("shift-down".into()), 6));
        assert_eq!(k(b"\x1b[1;3A"), (Some("alt-up".into()), 6), "xterm's Alt");
        assert_eq!(
            k(b"\x1b\x1b[Bx"),
            (Some("alt-down".into()), 4),
            "Option as Meta"
        );
        assert_eq!(k(b"\x1b[5~"), (Some("pageup".into()), 4), "Fn+up on a Mac");
        assert_eq!(k(b"\x1b[Z"), (Some("shift-tab".into()), 3), "Shift+Tab");
        assert_eq!(k("é".as_bytes()), (Some("é".into()), 2));
        assert_eq!(
            k(b"\x1b[1;5Cz"),
            (None, 6),
            "an unknown sequence is skipped whole"
        );
    }

    #[test]
    fn an_escape_sequence_without_its_end_is_unfinished() {
        assert!(unfinished(b"\x1b"));
        assert!(unfinished(b"\x1b["));
        assert!(unfinished(b"\x1b[1;5"));
        assert!(!unfinished(b"\x1b[A"));
        assert!(!unfinished(b"q"));
        assert!(unfinished(&"é".as_bytes()[..1]));
        assert!(unfinished(b"\x1b[200~half a paste"));
        assert!(unfinished(b"\x1b[20"));
        assert!(!unfinished(b"\x1b[200~done\x1b[201~"));
    }

    /// A paste whose end marker never comes ends once the terminal has been
    /// found quiet for `PASTE_IDLE`: what is typed after is keys again. What
    /// waited unread through a gap without polls is still the paste.
    #[test]
    fn a_paste_without_its_end_ends_when_input_stops() {
        use std::io::Write;
        let (read, write) = std::io::pipe().unwrap();
        let mut tty = File::from(std::os::fd::OwnedFd::from(read));
        let mut write = File::from(std::os::fd::OwnedFd::from(write));
        let mut keys = Keys::default();
        write.write_all(b"\x1b[200~lost\x1b[20").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:lost".into()));
        // Its end marker arrives split across reads, the start held back.
        std::thread::sleep(Duration::from_millis(50));
        write.write_all(b"1~").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:".into()));
        assert!(!keys.in_paste);

        write.write_all(b"\x1b[200~again\x1b[20").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:again".into()));
        assert_eq!(keys.take(&mut tty).unwrap(), None, "found quiet");
        keys.quiet_since = Some(std::time::Instant::now() - PASTE_IDLE);
        assert_eq!(
            keys.take(&mut tty).unwrap(),
            None,
            "quiet long enough: ended"
        );
        assert!(!keys.in_paste);
        write.write_all(b"q").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("q".into()));
    }

    /// Input waiting when a paste would have gone stale keeps it going.
    #[test]
    fn waiting_input_is_not_quiet() {
        use std::io::Write;
        let (read, write) = std::io::pipe().unwrap();
        let mut tty = File::from(std::os::fd::OwnedFd::from(read));
        let mut write = File::from(std::os::fd::OwnedFd::from(write));
        let mut keys = Keys::default();
        write.write_all(b"\x1b[200~abc").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:abc".into()));
        keys.quiet_since = Some(std::time::Instant::now() - PASTE_IDLE);
        write.write_all(b"q").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:q".into()));
    }

    /// An Esc alone is Esc once the wait for the rest of a sequence is out;
    /// an Esc and a key read together are Alt with it.
    #[test]
    fn keys_read_esc_alone_and_with_a_key() {
        use std::io::Write;
        let (read, write) = std::io::pipe().unwrap();
        let mut tty = File::from(std::os::fd::OwnedFd::from(read));
        let mut write = File::from(std::os::fd::OwnedFd::from(write));
        let mut keys = Keys::default();
        write.write_all(b"\x1b").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("esc".into()));
        write.write_all(b"\x1bq").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("alt-q".into()));
        write.write_all(b"\x1b\x7f").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("alt-backspace".into()));
        assert_eq!(keys.take(&mut tty).unwrap(), None);
    }

    /// Reading from a pipe as the terminal: a paste with no end yet comes as
    /// a piece, then nothing (not a spin) while its rest has not come, then
    /// the rest, and keys after it.
    #[test]
    fn keys_read_a_paste_across_gaps() {
        use std::io::Write;
        let (read, write) = std::io::pipe().unwrap();
        let mut tty = File::from(std::os::fd::OwnedFd::from(read));
        let mut write = File::from(std::os::fd::OwnedFd::from(write));
        let mut keys = Keys::default();
        write.write_all(b"\x1b[200~abc\x1b[20").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:abc".into()));
        let started = std::time::Instant::now();
        assert_eq!(keys.take(&mut tty).unwrap(), None, "the rest has not come");
        assert!(started.elapsed() < Duration::from_secs(1), "no spin");
        write.write_all(b"1~q").unwrap();
        assert_eq!(keys.take(&mut tty).unwrap(), Some("paste:".into()));
        assert_eq!(keys.take(&mut tty).unwrap(), Some("q".into()));
        assert_eq!(keys.take(&mut tty).unwrap(), None);
    }

    /// A paste longer than a read comes as pieces, keys never, its end
    /// marker or a UTF-8 character split across reads held for the next.
    #[test]
    fn a_long_paste_comes_in_pieces() {
        let mut pending = b"abc\x1b[20".to_vec();
        let mut in_paste = true;
        assert_eq!(
            paste_chunk(&mut pending, &mut in_paste),
            Some("paste:abc".into())
        );
        assert!(in_paste);
        assert_eq!(pending, b"\x1b[20");
        pending.extend_from_slice(b"1~qj");
        assert_eq!(
            paste_chunk(&mut pending, &mut in_paste),
            Some("paste:".into())
        );
        assert!(!in_paste, "the end marker ends it");
        assert_eq!(pending, b"qj", "what follows is keys again");
        let mut pending = "xé".as_bytes()[..2].to_vec();
        let mut in_paste = true;
        assert_eq!(
            paste_chunk(&mut pending, &mut in_paste),
            Some("paste:x".into())
        );
        assert_eq!(pending.len(), 1, "half a character waits");
    }

    /// A destination nobody reads stops a replay after `STALL_LIMIT`.
    #[test]
    fn a_stalled_destination_stops_the_replay() {
        let (read, write) = private_pipe();
        let mut out = File::from(write);
        let started = std::time::Instant::now();
        write_patiently(&mut out, &vec![b'x'; 1 << 20]);
        let took = started.elapsed();
        drop(read);
        assert!(
            took >= STALL_LIMIT && took < STALL_LIMIT * 3,
            "took {took:?}"
        );
    }

    /// A child that holds the pipe past `restore()` still gets its writes
    /// through.
    #[test]
    fn a_child_outliving_the_capture_still_writes() {
        let (read, write) = private_pipe();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stdout, false).unwrap();
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 0.3; echo late"])
            .stdout(std::process::Stdio::from(dup(write.as_raw_fd()).unwrap()))
            .spawn()
            .unwrap();
        r.restore();
        drop(write);
        assert!(child.wait().unwrap().success(), "not killed by SIGPIPE");
        let mut got = String::new();
        File::from(read).read_to_string(&mut got).unwrap();
        assert_eq!(got, "late\n");
    }

    /// The CLI's own writes, with a screen's sink set, go to the screen's
    /// feed and its replay, never through the captured descriptors; with
    /// none, nowhere but the descriptor.
    #[test]
    fn the_clis_own_output_is_routed_past_a_capture() {
        route_cli_output();
        let (_read, write) = private_pipe();
        let capture = Arc::new(Feed::default());
        let out = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stdout, false).unwrap();
        let err = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stderr, false).unwrap();
        set_sink(Some((&capture, &out, &err)));
        assert!(axl_types::terminal::routed(2, b"routed line\n"));
        set_sink(None);
        assert!(!axl_types::terminal::routed(2, b"not routed\n"));
        assert_eq!(capture.drain().lines, vec!["routed line"]);
        assert_eq!(&*err.written.lock().unwrap(), b"routed line\n");
        assert!(out.written.lock().unwrap().is_empty());
        err.restore();
        out.restore();
    }

    /// What is written to a redirected descriptor is captured as lines and
    /// written back to it on restore.
    #[test]
    fn a_redirect_captures_and_replays() {
        let (read, write) = private_pipe();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stdout, false).unwrap();
        let mut target = File::from(dup(write.as_raw_fd()).unwrap());
        target.write_all(b"one\ntwo\n").unwrap();
        drop(target);
        std::thread::sleep(Duration::from_millis(100));
        r.restore();
        drop(write);
        assert_eq!(capture.drain().lines, vec!["one", "two"]);
        let mut replayed = String::new();
        File::from(read).read_to_string(&mut replayed).unwrap();
        assert_eq!(replayed, "one\ntwo\n");
    }

    #[test]
    fn the_replay_keeps_the_newest_bytes() {
        let mut buf = vec![b'a'; 2 * MAX_CAPTURED_BYTES];
        keep(&mut buf, b"xyz");
        assert_eq!(buf.len(), MAX_CAPTURED_BYTES);
        assert!(buf.ends_with(b"axyz"));
        let mut small = b"ab".to_vec();
        keep(&mut small, b"c");
        assert_eq!(newest(&small), b"abc");
    }

    /// A child writing without pause does not hold up the restore.
    #[test]
    fn a_restore_does_not_wait_on_a_busy_child() {
        let (read, write) = private_pipe();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture, Source::Stdout, false).unwrap();
        let mut child = std::process::Command::new("sh")
            // `exec`, so the kill below reaches `yes` itself.
            .args(["-c", "exec yes"])
            .stdout(std::process::Stdio::from(dup(write.as_raw_fd()).unwrap()))
            .spawn()
            .unwrap();
        // Something drains the restored descriptor, as a terminal would.
        let drain = std::thread::spawn(move || {
            let _ = std::io::copy(&mut File::from(read), &mut std::io::sink());
        });
        std::thread::sleep(Duration::from_millis(50));
        let started = std::time::Instant::now();
        r.restore();
        let took = started.elapsed();
        child.kill().unwrap();
        child.wait().unwrap();
        drop(write);
        drain.join().unwrap();
        assert!(took < Duration::from_secs(2), "restore took {took:?}");
    }
}
