//! `std.term`: the terminal, for a feature that draws the whole screen.
//!
//! `ctx.std.term.take()` hands out the controlling terminal (`/dev/tty`) as a
//! [`Screen`]: the alternate screen, the cursor hidden, keys read one at a
//! time without echo. Ctrl+C still interrupts, so the CLI's cancel path stays
//! the way out of a stuck screen, unless the feature reads it as a key
//! (`screen.interrupts(False)`). The mouse is the terminal's unless the
//! feature asks for its wheel (`screen.mouse(True)`). While it is held, whatever the process
//! writes to its stdout and stderr (the CLI's own lines, `print()`,
//! diagnostics, a child that inherited them) is captured rather than drawn
//! over the screen: `screen.output()` hands it to the feature, and
//! `release()` puts the terminal back and writes it to the scrollback (the
//! newest 8 MiB of each stream), so what was printed while the screen was up
//! is not lost.
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
    /// The process's stdout and stderr, captured.
    stdout: Redirect,
    stderr: Redirect,
    capture: Arc<Feed>,
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
}

impl Redirect {
    /// Send `fd` into a pipe whose reader keeps what is written, for the
    /// replay, and hands it to `capture` as lines.
    fn new(fd: RawFd, capture: Arc<Feed>, source: Source) -> std::io::Result<Redirect> {
        let saved = dup(fd)?;
        let (read, write) = nix::unistd::pipe()?;
        // Only `fd` is for children (`dup2` leaves it inheritable); a spawn
        // on another thread in this window can still inherit `write`.
        cloexec(read.as_raw_fd())?;
        cloexec(write.as_raw_fd())?;
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
                            // fill the pipe and block every writer, the CLI's
                            // own printing among them.
                            Err(_) => std::thread::sleep(Duration::from_millis(10)),
                            Ok(n) => {
                                keep(&mut written.lock().unwrap(), &buf[..n]);
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
        self.finish();
    }

    /// Point the descriptor at what it was again: quick, so every descriptor
    /// is back before any replay waits on a slow destination.
    fn put_back(&self) {
        let _ = dup2(self.saved.as_raw_fd(), self.fd);
    }

    /// Stop capturing, replay what was written, and copy through what a
    /// child still holding the pipe writes; after [`Redirect::put_back`].
    fn finish(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let open = self.reader.take().and_then(|r| r.join().ok().flatten());
        let written = std::mem::take(&mut *self.written.lock().unwrap());
        let mut out = File::from(self.saved);
        write_patiently(&mut out, newest(&written));
        if let Some(mut pipe) = open {
            let _ = std::thread::Builder::new()
                .name("term-capture-after".into())
                .spawn(move || {
                    let mut buf = [0u8; 8192];
                    loop {
                        match pipe.read(&mut buf) {
                            Ok(0) => return,
                            Ok(n) => write_patiently(&mut out, &buf[..n]),
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(_) => return,
                        }
                    }
                });
        }
    }
}

/// How long a stopped capture reads what is left before handing the pipe on.
const DRAIN_BUDGET: Duration = Duration::from_millis(100);

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

/// The controlling terminal's size, held or not; None without one.
fn terminal_size() -> Option<(i32, i32)> {
    if let Some(h) = HELD.lock().unwrap().as_ref() {
        return size_of(h.tty.as_raw_fd());
    }
    File::open("/dev/tty")
        .ok()
        .and_then(|tty| size_of(tty.as_raw_fd()))
}

/// The terminal's size as `(columns, rows)`, from `fd`.
fn size_of(fd: RawFd) -> Option<(i32, i32)> {
    bazel::console::capture::window_size(fd)
        .filter(|ws| ws.ws_col > 0 && ws.ws_row > 0)
        .map(|ws| (ws.ws_col as i32, ws.ws_row as i32))
}

/// Whether the pipe `fd` has bytes to read within `timeout`.
fn pipe_readable(fd: RawFd, timeout: Duration) -> bool {
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

/// Whether the terminal `fd` has bytes to read within `timeout`.
fn tty_readable(fd: RawFd, timeout: Duration) -> bool {
    select_one(fd, timeout, false).unwrap_or(false)
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

fn dup2(from: RawFd, to: RawFd) -> std::io::Result<()> {
    // SAFETY: both are open descriptors; `to` is replaced.
    if unsafe { libc::dup2(from, to) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Take the terminal; see the module docs. Its generation.
fn take(alt_screen: bool) -> anyhow::Result<u64> {
    let mut held = HELD.lock().unwrap();
    if held.is_some() {
        anyhow::bail!("the terminal is already taken");
    }
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|e| anyhow::anyhow!("no terminal to take: {e}"))?;
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

    let _ = std::io::stdout().flush();
    {
        use std::io::IsTerminal;
        mark_captured(Some((
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal(),
        )));
    }
    let capture = Arc::new(Feed::default());
    let forget_tty = || mark_captured(None);
    let stdout = Redirect::new(1, capture.clone(), Source::Stdout)
        .inspect_err(|_| forget_tty())
        .map_err(undo_termios)?;
    let stderr = match Redirect::new(2, capture.clone(), Source::Stderr) {
        Ok(r) => r,
        Err(e) => {
            stdout.restore();
            forget_tty();
            return Err(undo_termios(e).into());
        }
    };

    let mut tty = tty;
    let enter: &[u8] = if alt_screen {
        b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[2J\x1b[H"
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
    install_panic_hook();
    remember_for_crash(
        tty.as_raw_fd(),
        &saved_termios,
        alt_screen,
        &stdout,
        &stderr,
    );
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::SeqCst);
    *held = Some(Held {
        generation,
        alt_screen,
        tty,
        saved_termios,
        stdout,
        stderr,
        capture,
        keys: Keys::default(),
    });
    Ok(generation)
}

/// What a crash handler needs to give the terminal back, without allocating
/// or locking: set while a screen is held. Its descriptors are its own
/// copies, never closed, so a handler that read the pointer just before a
/// release cleared it still writes to open descriptors (three per `take()`).
struct CrashState {
    tty: RawFd,
    termios: libc::termios,
    alt_screen: bool,
    stdout: RawFd,
    stderr: RawFd,
}

static CRASH: std::sync::atomic::AtomicPtr<CrashState> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

fn remember_for_crash(
    tty: RawFd,
    termios: &Termios,
    alt_screen: bool,
    stdout: &Redirect,
    stderr: &Redirect,
) {
    // SAFETY: a plain descriptor duplicate; -1 on failure, which the
    // handler's calls then fail on harmlessly.
    let keep = |fd: RawFd| unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    let state = Box::new(CrashState {
        tty: keep(tty),
        termios: termios.clone().into(),
        alt_screen,
        stdout: keep(stdout.saved.as_raw_fd()),
        stderr: keep(stderr.saved.as_raw_fd()),
    });
    CRASH.store(Box::into_raw(state), Ordering::SeqCst);
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
    // SAFETY: set from a leaked Box in `take`, never freed, whose
    // descriptors are never closed.
    unsafe {
        let s = &*state;
        libc::tcsetattr(s.tty, libc::TCSANOW, &s.termios);
        let leave = leave_sequence(s.alt_screen);
        libc::write(s.tty, leave.as_ptr().cast(), leave.len());
        libc::dup2(s.stdout, 1);
        libc::dup2(s.stderr, 2);
    }
}

/// Put back a screen still held: the terminal's modes, the main screen, the
/// process's stdout and stderr, and then what was captured from each,
/// written to it. A no-op when none is held.
///
/// Bounded, so the forced exit always ends: the lock is waited on for 500ms,
/// and the replay, run without it, gives up on a destination that takes
/// nothing for `STALL_LIMIT`. A frame still holding the lock after that (stuck
/// in a write) gets the crash path's restore instead, with nothing replayed.
pub fn release_all() {
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    loop {
        match HELD.try_lock() {
            // The replay runs with the lock let go.
            Ok(mut held) => {
                let mine = held.take();
                drop(held);
                return release(mine);
            }
            Err(std::sync::TryLockError::Poisoned(p)) => {
                let mine = p.into_inner().take();
                return release(mine);
            }
            Err(std::sync::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::sync::TryLockError::WouldBlock) => return restore_terminal_after_crash(),
        }
    }
}

fn release(held: Option<Held>) {
    let Some(mut held) = held else {
        return;
    };
    // Both descriptors back first, so a crash during the replays below
    // reports to the terminal.
    held.stdout.put_back();
    held.stderr.put_back();
    let leave = leave_sequence(held.alt_screen);
    let _ = held.tty.write_all(leave);
    let _ = held.tty.flush();
    // Keys typed but never read go too: the shell would run them.
    let _ = termios::tcsetattr(&held.tty, SetArg::TCSAFLUSH, &held.saved_termios);
    CRASH.store(std::ptr::null_mut(), Ordering::SeqCst);
    mark_captured(None);
    held.stdout.finish();
    held.stderr.finish();
}

/// What gives the terminal back its cursor, the mouse (`mouse(True)`) and,
/// for a screen that took it, the main screen.
fn leave_sequence(alt_screen: bool) -> &'static [u8] {
    if alt_screen {
        b"\x1b[?1006l\x1b[?1000l\x1b[?2004l\x1b[?25h\x1b[?1049l"
    } else {
        b"\x1b[?1006l\x1b[?1000l\x1b[?2004l\x1b[?25h"
    }
}

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
                    Ok(mut held) => release(held.take()),
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
            let n = tty.read(&mut buf)?;
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
            let n = tty.read(&mut buf)?;
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
        let (name, used) = parse_key(&self.pending);
        self.pending.drain(..used);
        Ok(name)
    }
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
    match bytes {
        [0x1b] | [0x1b, b'O'] | [0x1b, b'O', b'2'] => true,
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
        // Shift+arrows as other terminals send them: older xterm's
        // `ESC [ 2 <dir>` and `ESC O 2 <dir>`, rxvt's `ESC [ a`…`d`.
        [0x1b, b'[' | b'O', b'2', d @ (b'A' | b'B' | b'C' | b'D'), ..] => {
            (Some(format!("shift-{}", arrow(*d))), 4)
        }
        [0x1b, b'[', d @ (b'a' | b'b' | b'c' | b'd'), ..] => {
            (Some(format!("shift-{}", arrow(d.to_ascii_uppercase()))), 3)
        }
        // SS3 (`ESC O x`), F1–F4 among them: three bytes, unnamed unless
        // above.
        [0x1b, b'O', _, ..] => (None, 3),
        [0x1b, b'[', b'1' | b'7', b'~', ..] => named("home", 4),
        [0x1b, b'[', b'4' | b'8', b'~', ..] => named("end", 4),
        [0x1b, b'[', b'5', b'~', ..] => named("pageup", 4),
        [0x1b, b'[', b'6', b'~', ..] => named("pagedown", 4),
        // A mouse report (`mouse(True)`): SGR's `ESC [ < b ; x ; y M` (or
        // `m`, a release), or the legacy `ESC [ M` and three bytes, `b`
        // offset by 32, from a terminal without SGR. Only the wheel is
        // named; a click or a release is skipped whole.
        [0x1b, b'[', b'<', rest @ ..] => {
            let Some(end) = rest.iter().position(|b| (0x40..=0x7e).contains(b)) else {
                return (None, bytes.len());
            };
            let button = std::str::from_utf8(&rest[..end])
                .ok()
                .and_then(|p| p.split(';').next())
                .and_then(|b| b.parse::<u32>().ok());
            let name = match rest[end] {
                b'M' => button.and_then(wheel),
                _ => None,
            };
            (name.map(str::to_string), end + 4)
        }
        [0x1b, b'[', b'M', b, _, _, ..] => {
            (wheel(u32::from(b.wrapping_sub(32))).map(str::to_string), 6)
        }
        [0x1b, b'[', rest @ ..] => {
            // Some other sequence: skip it whole, to its final byte.
            let end = rest.iter().position(|b| (0x40..=0x7e).contains(b));
            (None, end.map_or(bytes.len(), |i| i + 3))
        }
        // Alt (Option, where the terminal sends it as Meta) with a key: ESC
        // then the key, read together.
        [0x1b, c @ 0x20..=0x7e, ..] => (Some(format!("alt-{}", *c as char)), 2),
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
    /// The terminal's size as `(columns, rows)`, or None without one.
    fn size<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<NoneOr<(i32, i32)>> {
        Ok(NoneOr::from_option(terminal_size()))
    }

    /// Whether a screen is held now, by any feature.
    #[starlark(attribute)]
    fn taken<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<bool> {
        Ok(HELD.lock().unwrap().is_some())
    }

    /// Take the terminal for a screen of your own; see `std.term`. Fails
    /// when there is no terminal or another screen holds it.
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
    /// flush. Draw a frame in one write, so it never shows half drawn.
    fn write<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
    ) -> anyhow::Result<NoneType> {
        with_held(this, |h| {
            h.tty.write_all(text.as_bytes())?;
            h.tty.flush()?;
            Ok(NoneType)
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
    /// not at all); None when none was. A printable character is itself;
    /// otherwise `up`, `down`, `left`, `right`, `shift-up`, `shift-down`,
    /// `shift-left`, `shift-right`, `pageup`, `pagedown`, `home`,
    /// `end`, `enter`, `tab`, `backspace`, `esc`, `ctrl-<letter>`,
    /// `alt-<key>` (where the terminal sends Alt as Meta, and for an Esc
    /// typed just before a key), `paste:<text>` (pasted text: bracketed
    /// paste is on while the screen is held; a long paste comes as several,
    /// the last maybe empty), `wheel-up` and `wheel-down` (the mouse's
    /// wheel, once `mouse(True)`; its clicks are skipped), or `ctrl-@`,
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
            let (fd, got) = with_held(this, |h| {
                let got = h.keys.take(&mut h.tty)?;
                Ok((h.tty.as_raw_fd(), got))
            })?;
            if got.is_some() {
                return Ok(NoneOr::from_option(got));
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Ok(NoneOr::None);
            }
            tty_readable(fd, left.min(KEY_SLICE));
        }
    }

    /// The lines the process wrote to its stdout and stderr since the last
    /// call, escape sequences stripped: the CLI's own lines, `print()` and
    /// diagnostics, which `release()` also writes to the scrollback.
    fn output<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        with_held(this, |h| Ok(h.capture.drain().lines))
    }

    /// Give the terminal back: its modes, the main screen, and the process's
    /// stdout and stderr, to which everything captured is then written. A
    /// no-op once released.
    fn release<'v>(this: values::Value<'v>) -> anyhow::Result<NoneType> {
        let generation = generation_of(this)?;
        let mut held = HELD.lock().unwrap();
        if held.as_ref().is_some_and(|h| h.generation == generation) {
            let mine = held.take();
            drop(held);
            release(mine);
        }
        Ok(NoneType)
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
            let mut t = termios::tcgetattr(&h.tty)?;
            t.local_flags.set(LocalFlags::ISIG, enabled);
            termios::tcsetattr(&h.tty, SetArg::TCSANOW, &t)?;
            Ok(NoneType)
        })
    }

    /// Whether the terminal reports the mouse (True) or keeps it for itself
    /// (False, the default): reported, its wheel arrives from `key()`, and
    /// selecting text takes a modifier the terminal names (Shift, Option or
    /// Fn with the drag). `release()` turns it off.
    fn mouse<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] enabled: bool,
    ) -> anyhow::Result<NoneType> {
        with_held(this, |h| {
            let set: &[u8] = if enabled {
                b"\x1b[?1000h\x1b[?1006h"
            } else {
                b"\x1b[?1006l\x1b[?1000l"
            };
            h.tty.write_all(set)?;
            h.tty.flush()?;
            Ok(NoneType)
        })
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
    use super::*;

    #[test]
    fn the_wheel_is_named_and_other_mouse_reports_skipped() {
        let k = |b: &[u8]| parse_key(b);
        assert_eq!(k(b"\x1b[<64;10;5Mq"), (Some("wheel-up".into()), 11));
        assert_eq!(k(b"\x1b[<65;1;1M"), (Some("wheel-down".into()), 10));
        assert_eq!(
            k(b"\x1b[<69;1;1M"),
            (Some("wheel-down".into()), 10),
            "with Shift"
        );
        assert_eq!(k(b"\x1b[<0;3;4Mq"), (None, 9), "a click");
        assert_eq!(k(b"\x1b[<0;3;4m"), (None, 9), "a release");
        assert_eq!(k(b"\x1b[<64;3;4m"), (None, 10));
        assert_eq!(k(b"\x1b[M`!!q"), (Some("wheel-up".into()), 6), "legacy");
        assert_eq!(k(b"\x1b[M !!q"), (None, 6), "a legacy click");
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
        let (read, write) = nix::unistd::pipe().unwrap();
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
        let (read, write) = nix::unistd::pipe().unwrap();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stdout).unwrap();
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

    /// What is written to a redirected descriptor is captured as lines and
    /// written back to it on restore.
    #[test]
    fn a_redirect_captures_and_replays() {
        let (read, write) = nix::unistd::pipe().unwrap();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture.clone(), Source::Stdout).unwrap();
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
        let (read, write) = nix::unistd::pipe().unwrap();
        let capture = Arc::new(Feed::default());
        let r = Redirect::new(write.as_raw_fd(), capture, Source::Stdout).unwrap();
        let mut child = std::process::Command::new("sh")
            .args(["-c", "yes"])
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
