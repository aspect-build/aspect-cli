use std::os::fd::RawFd;
use std::sync::Once;
use std::sync::atomic::Ordering;

use nix::sys::termios::Termios;

use super::capture::Order;
use super::replies::wheel_restore;
use super::{HELD, LEAVE, release};

/// What a crash handler needs to give the terminal back, without allocating
/// or locking: set while a screen is held. One for the process, its three
/// descriptors its own copies, never closed and renewed in place by each
/// `take()`, so a handler that read the pointer just before a release
/// cleared it still writes to open descriptors.
pub(super) struct CrashState {
    tty: RawFd,
    termios: libc::termios,
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

pub(super) static CRASH: std::sync::atomic::AtomicPtr<CrashState> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

pub(super) fn remember_for_crash(tty: RawFd, termios: &Termios, stdout: RawFd, stderr: RawFd) {
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
    let (note, note_len) = crash_note();
    let state = if *slot == 0 {
        let state = Box::into_raw(Box::new(CrashState {
            tty: keep(tty),
            termios: termios.clone().into(),
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
        libc::write(s.tty, LEAVE.as_ptr().cast(), LEAVE.len());
        let wheel = wheel_restore();
        libc::write(s.tty, wheel.as_ptr().cast(), wheel.len());
        libc::dup2(s.stdout, 1);
        libc::dup2(s.stderr, 2);
        if s.note_len > 0 {
            libc::write(2, s.note.as_ptr().cast(), s.note_len);
        }
    }
}

pub(super) fn install_panic_hook() {
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

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd;

    use super::super::fd::tests::private_pty;
    use super::*;

    /// Taking the screen again and again keeps the same three descriptors
    /// for a crash, rather than three more each time, and the modes of the
    /// latest take.
    #[test]
    fn the_crash_state_is_reused_across_takes() {
        let pty = private_pty();
        let tty = pty.slave.as_raw_fd();
        let termios = nix::sys::termios::tcgetattr(&pty.slave).unwrap();
        let mut later = termios.clone();
        later
            .local_flags
            .toggle(nix::sys::termios::LocalFlags::ECHO);
        let kept = || {
            // SAFETY: the one leaked state, not written meanwhile.
            let s = unsafe { &*CRASH.load(Ordering::SeqCst) };
            (s.tty, s.stdout, s.stderr, s.termios.c_lflag)
        };
        remember_for_crash(tty, &termios, 1, 2);
        let (t, o, e, _) = kept();
        for _ in 0..20 {
            remember_for_crash(tty, &later, 1, 2);
        }
        let later_lflag = libc::termios::from(later).c_lflag;
        assert_eq!(kept(), (t, o, e, later_lflag));
        for fd in [t, o, e] {
            assert!(unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC != 0);
        }
        CRASH.store(std::ptr::null_mut(), Ordering::SeqCst);
    }
}
