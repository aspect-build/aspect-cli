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
//! AXL streams, the build console) is routed to it ([`capture::ROUTER`]),
//! and what else reaches fds 1 and 2 (a stray writer, a child that
//! inherited them) is captured. `screen.output()` hands both to the
//! feature, and `release()` puts the terminal back and writes them to the
//! scrollback (the newest 8 MiB of each stream, or the task's `output.ansi`
//! dumped), so what was printed while the screen was up is not lost.
//!
//! One screen at a time, process-wide. [`release_all`] restores a screen
//! still held when the CLI's run returns and on its forced exit; a panic
//! that ends the process releases it too (`install_panic_hook`), as does
//! a crash (`restore_terminal_after_crash`), and a build with frame hooks
//! whose hook or wait fails;
//! a process killed outright leaves the terminal as the screen had it
//! (`reset` fixes it). The replay writes stdout's capture, then stderr's, so
//! the order between the two is lost.
//!
//! The `std.term` namespace holds what needs no terminal, for a module's
//! top level as much as a task: `dump_lines`, the dump's size, and `width`
//! and `fit`, which measure text in cells ([`register_globals`]).
//!
//! Here: the held screen, its Starlark `Term` and `Screen`, and taking,
//! suspending, resuming and releasing it. Beside it: [`capture`] (fds 1
//! and 2 sent into pipes, their replay, the CLI output's route), [`fd`]
//! (raw descriptor helpers), [`keys`] (keys, the mouse and pastes as
//! `key()` names them), [`replies`] (the terminal's answers to the screen's
//! queries), [`dump`] (the task's output written to the scrollback),
//! [`crash`] (the terminal given back after a crash or a panic) and
//! [`width`] (text in cells).

mod capture;
mod crash;
mod dump;
mod fd;
mod keys;
mod replies;
mod width;

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once, TryLockError};
use std::time::{Duration, Instant};

use allocative::Allocative;
use bazel::console::feed::{Drained, Feed, Source};
use derive_more::Display;
use nix::sys::termios::{self, InputFlags, LocalFlags, SetArg, SpecialCharacterIndices, Termios};
use starlark::StarlarkResultExt;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{self, NoSerialize, ProvidesStaticType, ValueLike, starlark_value};

use capture::{
    MAX_CAPTURED_BYTES, Order, ROUTER, Redirect, SWITCH_WAIT, give_back_descriptors, mark_captured,
    same_file,
};
use crash::{CRASH, install_panic_hook, remember_for_crash};
use dump::{
    DUMP_LINES, DUMP_WRITTEN, DUMPED, dump_output, dump_output_turned, dump_ready,
    older_lines_note, set_dump_mark,
};
use fd::{hung_up, in_foreground, size_of, tty_readable};
use keys::{KEY_SLICE, Keys};
use replies::{
    BACKGROUND_WAIT, WHEEL, WHEEL_ASKED, WHEEL_KEYS_OFF, WHEEL_KEYS_ON, WHEEL_UNTOUCHED,
    WHEEL_WAS_OFF, WHEEL_WAS_ON, alternate_scroll, drop_answers_due, query_background,
    wheel_restore,
};
use width::{fit as fit_text, width as text_width};

pub use capture::route_cli_output;
pub(crate) use capture::{feed_screen, forwarding, write_std_unit};
pub use crash::restore_terminal_after_crash;
pub(crate) use fd::pipe_readable;

/// The screen held now, if any.
static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// How long a wait for a lock held elsewhere sleeps between tries.
const LOCK_RETRY: Duration = Duration::from_millis(10);

/// `m` locked (a poisoned lock's guard too), tried until `deadline`; None
/// when another still holds it then. A deadline already past tries once.
fn lock_until<T>(m: &Mutex<T>, deadline: Instant) -> Option<MutexGuard<'_, T>> {
    loop {
        match m.try_lock() {
            Ok(g) => return Some(g),
            Err(TryLockError::Poisoned(p)) => return Some(p.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(LOCK_RETRY);
            }
            Err(TryLockError::WouldBlock) => return None,
        }
    }
}

/// The generation the next `take()` gets.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

struct Held {
    /// Which `take()` this is: a [`Screen`] acts only on its own.
    generation: u64,
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

/// A screen method's terminal write or mode change, done; one a terminal
/// gone refused ([`hung_up`]) as well.
fn gone_is_fine(done: std::io::Result<()>) -> anyhow::Result<NoneType> {
    match done {
        Err(e) if !hung_up(&e) => Err(e.into()),
        _ => Ok(NoneType),
    }
}

/// Take the terminal; see the module docs. Its generation. Refused while
/// this process runs in the background (another job has the terminal).
fn take() -> anyhow::Result<u64> {
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
    // The screen shows the task's output from its start: what it wrote
    // before Bazel's console began, handed over apart (`earlier()`).
    let earlier = crate::out::output_before_console_ansi(MAX_CAPTURED_BYTES as u64);
    let forget_tty = || {
        mark_captured(None);
        crate::out::set_capture_copies(false);
        ROUTER.set(None);
    };
    // No write half to the terminal and half to the capture.
    let moving = axl_types::terminal::every_turn_within(SWITCH_WAIT);
    let stdout = Redirect::new(1, capture.clone(), Source::Stdout, true)
        .inspect_err(|_| forget_tty())
        .map_err(undo_termios)?;
    let stderr = match Redirect::new(2, capture.clone(), Source::Stderr, true) {
        Ok(r) => r,
        Err(e) => {
            stdout.restore();
            forget_tty();
            return Err(undo_termios(e).into());
        }
    };
    // Once both are captured, the capture copies them, and the CLI's own
    // output goes to the screen instead ([`route`]). The screen is dumped on
    // leaving, from here on.
    crate::out::set_capture_copies(true);
    ROUTER.set(Some((&capture, &stdout, &stderr)));
    set_dump_mark();
    drop(moving);

    let mut tty = tty;
    if let Err(e) = tty.write_all(ENTER).and_then(|()| tty.flush()) {
        stderr.restore();
        stdout.restore();
        forget_tty();
        let _ = termios::tcsetattr(&tty, SetArg::TCSANOW, &saved_termios);
        return Err(e.into());
    }
    // Asked once the modes are set, so the answer is read, not echoed.
    let (dark, typed) = query_background(&mut tty, BACKGROUND_WAIT);
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
        stdout.saved.as_raw_fd(),
        stderr.saved.as_raw_fd(),
    );
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::SeqCst);
    axl_types::stream::set_terminal_input(suspend, resume);
    stop_with_the_screen();
    *held = Some(Held {
        generation,
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

/// How long a forced release waits for the output order's locks and the
/// screen's, and a suspend for the output order's: one replaying to a
/// stalled stdout holds them, and the exit, or Ctrl+Z, must not wait it out.
const LOCK_WAIT: Duration = Duration::from_millis(500);

/// Put back a screen still held: the terminal's modes, the main screen, the
/// process's stdout and stderr, and then what was captured from each,
/// written to it. A no-op when none is held. Whether one was.
///
/// Bounded unless the terminal itself stops taking output (the leave sequence
/// and the mode restore wait on it), so the forced exit ends: the lock is
/// waited on for `LOCK_WAIT`,
/// and the replay, run without it, gives up on a destination that takes
/// nothing for `STALL_LIMIT`. A frame still holding the lock after that (stuck
/// in a write) gets the crash path's restore instead, with nothing replayed.
pub fn release_all() -> bool {
    let deadline = Instant::now() + LOCK_WAIT;
    // No forwarder writes into the replay (see [`OUTPUT_ORDER`]), unless one
    // holds a lock past the deadline.
    let mut order = Order::lock_until(deadline);
    let Some(mut held) = lock_until(&HELD, deadline) else {
        restore_terminal_after_crash();
        return true;
    };
    // The replay runs with the lock let go.
    let mine = held.take();
    drop(held);
    release_dumping(mine, &mut order)
}

/// Give a held screen back for a while, for something that needs
/// the real terminal (a prompt reading stdin, an interactive child): the
/// task's output so far dumped to it ([`dump_output`]) and stdout's captured
/// lines to a stdout that is not the terminal, the main screen (the screen's
/// own cleared first), its modes and stdout and stderr back, the capture's
/// copies stopped, and no answer to its own queries or key typed for it left
/// for the child to read ([`drop_answers_due`]). Whether
/// one was given back, for [`resume`], which each true needs once: a screen
/// already suspended counts another, and comes back with the last resume.
/// None held: false.
pub(crate) fn suspend() -> bool {
    let finishing;
    let moving;
    {
        // Bounded, as a release's: one replaying to a stalled stdout holds
        // these, and Ctrl+Z must not wait it out.
        let _order = Order::lock_until(Instant::now() + LOCK_WAIT);
        let mut held = HELD.lock().unwrap();
        let Some(h) = held.as_mut() else {
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
        let _ = h.tty.write_all(CLEAR);
        let _ = h.tty.write_all(LEAVE);
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
    if dump_output_turned(DUMP_LINES, &older_lines_note(), held) {
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
/// Installed once, with the first screen; the signal handler only
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

/// Whether a screen holds the terminal now, not given back: a child spawned
/// meanwhile has no terminal of its own.
pub(crate) fn screen_holds_terminal() -> bool {
    HELD.lock()
        .ok()
        .is_some_and(|h| h.as_ref().is_some_and(|h| !h.suspended))
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
        ROUTER.set(Some((&h.capture, out, err)));
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
    let mut enter = ENTER.to_vec();
    if h.mouse_on {
        enter.extend_from_slice(MOUSE_ON);
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
    let older = older_lines_note();
    let _ = release(
        held,
        !dumping,
        dumping.then_some((DUMP_LINES, older.as_str())),
        order,
    );
    was
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
    let _ = held.tty.write_all(LEAVE);
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

/// What takes the terminal for the screen: the alternate screen
/// (`?1049h`), no wrapping (`?7l`), the cursor hidden (`?25l`), bracketed
/// paste (`?2004h`), cleared, the cursor at the top left.
const ENTER: &[u8] = b"\x1b[?1049h\x1b[?7l\x1b[?25l\x1b[?2004h\x1b[2J\x1b[H";

/// What gives the terminal back its cursor, the mouse (`mouse(True)`), a
/// synchronized update a frame cut short left open (`?2026`), bracketed
/// paste, wrapping and the main screen; its wheel (`wheel_keys`) is put back
/// as it was apart ([`wheel_restore`]).
const LEAVE: &[u8] =
    b"\x1b[?2026l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?2004l\x1b[?7h\x1b[?25h\x1b[?1049l";

/// Clear the screen and put the cursor at the top left.
const CLEAR: &[u8] = b"\x1b[2J\x1b[H";

/// Report the mouse: presses, drags with a button held and moves without
/// one (`?1000h`, `?1002h`, `?1003h`), in SGR's encoding (`?1006h`).
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";

/// Leave the mouse to the terminal again: [`MOUSE_ON`] undone.
const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

/// Begin a synchronized update (DEC mode 2026): the terminal holds what it
/// draws until [`SYNC_END`].
const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";

/// End a synchronized update: the terminal shows what it held.
const SYNC_END: &[u8] = b"\x1b[?2026l";

/// `frame` as one synchronized update ([`SYNC_BEGIN`]): the terminal holds
/// what it draws until the end, so a frame never shows half drawn.
fn synchronized(frame: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() + SYNC_BEGIN.len() + SYNC_END.len());
    out.extend_from_slice(SYNC_BEGIN);
    out.extend_from_slice(frame);
    out.extend_from_slice(SYNC_END);
    out
}

/// What a feed drained, as `[line, links, coloured]` a line: what
/// `screen.output()` and `screen.earlier()` hand out.
fn lines_linked_and_coloured(got: Drained) -> serde_json::Value {
    serde_json::json!(
        got.lines
            .into_iter()
            .zip(got.links)
            .zip(got.colours)
            .map(|((line, links), coloured)| serde_json::json!([line, links, coloured]))
            .collect::<Vec<_>>()
    )
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

/// The `std.term` namespace's values beside its types, which AXL reads
/// anywhere, a module's top level among them; the terminal itself is
/// `ctx.std.term` ([`Term`]).
#[starlark_module]
pub(crate) fn register_globals(globals: &mut GlobalsBuilder) {
    /// The most lines a screen's dump writes when the runtime writes it (a
    /// suspend, a release at exit) and `dump_output`'s default: a screen
    /// that keeps its own log keeps as many, so the dump and the screen
    /// show the same lines.
    const dump_lines: i32 = DUMP_LINES as i32;

    /// How many cells a terminal draws `text` in: a wide character (`✅`,
    /// CJK) two, a combining mark or a zero-width character none, a
    /// one-cell character with U+FE0F after it (`⚠️`) two, and every code
    /// point below U+0300 one, a control character or a tab among them.
    fn width(#[starlark(require = pos)] text: &str) -> anyhow::Result<i32> {
        Ok(i32::try_from(text_width(text)).unwrap_or(i32::MAX))
    }

    /// `text` cut or padded with spaces to exactly `width` cells, as
    /// `width()` counts them, a tab written as two spaces; "" for a `width`
    /// of 0 or less: a row wider than the screen would wrap and scroll it.
    /// Cut, it keeps what fits in one cell less, then `…` (no `…` at a
    /// `width` of 1); a wide character that would cross the edge is left
    /// out with what follows it, padding in its place, and zero-width
    /// characters right after the last one kept stay. Only as much of a
    /// long `text` is measured as could show.
    fn fit(#[starlark(require = pos)] text: &str, width: i32) -> anyhow::Result<String> {
        Ok(fit_text(text, width.into()))
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

    /// Write to stderr, once a screen is given back, the task's
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
        let _turn = axl_types::terminal::turn(2);
        if !ROUTER.is_set() {
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

    /// Take the terminal for a screen of your own, drawn on the terminal's
    /// alternate screen, leaving the scrollback as it was; see `std.term`.
    /// Fails when there is no terminal, another screen holds it, or this
    /// process runs in the background (another job has the terminal).
    fn take<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<Screen> {
        Ok(Screen {
            generation: take()?,
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
    /// `shift-left`, `shift-right`, `alt-up`, `alt-down`, `alt-left`,
    /// `alt-right` (xterm's Alt, or Option as Meta), `pageup`, `pagedown`,
    /// `home`, `end`, `enter`, `tab`, `shift-tab`, `backspace`, `esc`,
    /// `ctrl-<letter>`,
    /// `alt-<key>` (where the terminal sends Alt as Meta, and for an Esc
    /// typed just before a key), `alt-backspace`, `paste:<text>` (pasted text: bracketed
    /// paste is on while the screen is held; a long paste comes as several,
    /// the last maybe empty), `wheel-up` and
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
    /// call, each as `[line, links, coloured]`: `line` with its escape
    /// sequences stripped, `links` its OSC 8 links (`print()` links URLs and
    /// paths) as `[start, end, url]` by character, as `output_feed()` gives
    /// a build's, and `coloured` the same text with its colour kept. The
    /// CLI's own lines, `print()` and diagnostics, which `release()` also
    /// writes to the scrollback.
    fn output<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let got = with_held(this, |h| Ok(h.capture.drain()))?;
        Ok(heap.alloc(lines_linked_and_coloured(got)))
    }

    /// What the task wrote before Bazel's console began, as `[line, links,
    /// coloured]` like `output()`, its links made again (the banner's URL
    /// among them): for a screen that shows the task's output from its
    /// start, before any of Bazel's lines. Once: a second call gives none.
    fn earlier<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let bytes = with_held(this, |h| Ok(std::mem::take(&mut h.earlier)))?;
        let feed = Feed::default();
        feed.write(Source::Stderr, &relinked(&bytes));
        feed.finish(Source::Stderr);
        Ok(heap.alloc(lines_linked_and_coloured(feed.drain())))
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
                        WHEEL_KEYS_ON
                    } else {
                        WHEEL_KEYS_OFF
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
    /// its wheel does not scroll past the screen. `release()` turns it off.
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
            let set = if enabled { MOUSE_ON } else { MOUSE_OFF };
            gone_is_fine(h.tty.write_all(set).and_then(|()| h.tty.flush()))
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
    use super::fd::tests::private_pty;
    use super::*;

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

    /// `std.term.dump_lines` is the dump's size, read where AXL has no
    /// `ctx`: a module's top level.
    #[test]
    fn the_dump_size_is_read_at_the_top_level() {
        assert_eq!(
            crate::axl_eval!("x = std.term.dump_lines\nx").unwrap(),
            DUMP_LINES.to_string()
        );
    }

    /// `std.term.width` and `std.term.fit` are the namespace's functions,
    /// `fit`'s width by position or by name.
    #[test]
    fn width_and_fit_are_read_at_the_top_level() {
        assert_eq!(crate::axl_eval!("std.term.width('⚠️ x')").unwrap(), "4");
        assert_eq!(
            crate::axl_eval!("std.term.fit('a✅b', 3) == 'a… '").unwrap(),
            "True"
        );
        assert_eq!(
            crate::axl_eval!("std.term.fit('ab', width = -1)").unwrap(),
            "\"\""
        );
    }

    #[test]
    fn a_frame_is_one_synchronized_update() {
        assert_eq!(synchronized(b"ab"), b"\x1b[?2026hab\x1b[?2026l");
        // One cut short is closed when the terminal is given back.
        assert!(LEAVE.starts_with(SYNC_END));
    }
}
