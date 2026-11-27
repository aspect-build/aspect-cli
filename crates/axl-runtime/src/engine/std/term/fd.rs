use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::Duration;

/// How long [`write_patiently`] waits on a destination that takes nothing.
pub(super) const STALL_LIMIT: Duration = Duration::from_secs(2);

/// The most [`write_patiently`] writes at once: what a pipe takes whole once
/// it is writable, so a blocking descriptor cannot stall in a write.
const WRITE_CHUNK: usize = 512;

/// Write all of `bytes` to `out`, a chunk at a time once it is writable;
/// an error, or no progress for [`STALL_LIMIT`] (a pipe nobody reads, a
/// terminal stopped by Ctrl+S), drops the rest. Bounded either way, blocking
/// descriptor or not.
pub(super) fn write_patiently(out: &mut File, mut bytes: &[u8]) {
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

/// The terminal's size as `(columns, rows)`, from `fd`.
pub(super) fn size_of(fd: RawFd) -> Option<(i32, i32)> {
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
pub(super) fn hung_up(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EIO) | Some(libc::ENXIO))
}

/// Whether the terminal `fd` has bytes to read within `timeout`.
pub(super) fn tty_readable(fd: RawFd, timeout: Duration) -> bool {
    select_one(fd, timeout, false).unwrap_or(false)
}

/// Write all of `bytes` to `fd`, waiting as long as it takes, but never
/// inside a `write` that blocks: each chunk of at most [`WRITE_CHUNK`] once
/// `fd` is writable. The caller holds `fd`'s target for the whole. A
/// take's `dup2` over `fd` waits out a write in flight on it on macOS, so a
/// forwarder to a pipe nobody reads, blocked in one, would hold up the take,
/// and every stream fed to the screen with it. A `select` that fails rather
/// than waits (not a descriptor it can watch) gets the write anyway, after a
/// backoff that grows to [`BACKOFF_MAX`], which says why.
pub(crate) fn write_when_writable(fd: RawFd, mut bytes: &[u8]) -> std::io::Result<()> {
    let mut failed_fast = 0u32;
    while !bytes.is_empty() {
        let asked = std::time::Instant::now();
        if !writable(fd, WRITABLE_WAIT) {
            if asked.elapsed() >= WRITABLE_WAIT {
                continue;
            }
            // `select` failed rather than waited: back off, then the write
            // says why (or goes through).
            failed_fast += 1;
            std::thread::sleep((BACKOFF_STEP * failed_fast).min(BACKOFF_MAX));
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

/// How long [`write_when_writable`] waits at once for its descriptor to take
/// a write before it looks again.
const WRITABLE_WAIT: Duration = Duration::from_millis(100);

/// How much longer [`write_when_writable`] backs off after each `select`
/// that failed rather than waited.
const BACKOFF_STEP: Duration = Duration::from_millis(10);

/// The longest [`write_when_writable`] backs off before a write.
const BACKOFF_MAX: Duration = Duration::from_millis(100);

/// Whether `fd` takes a write within `timeout`; a descriptor `select` cannot
/// watch is written to anyway.
pub(super) fn writable(fd: RawFd, timeout: Duration) -> bool {
    select_one(fd, timeout, true).unwrap_or(true)
}

/// Whether `fd` is ready for reading (or, with `write`, writing) within
/// `timeout`; None for a descriptor past `FD_SETSIZE`. `select`, not `poll`:
/// macOS's `poll` does not support terminal devices.
pub(super) fn select_one(fd: RawFd, timeout: Duration, write: bool) -> Option<bool> {
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
pub(super) fn dup(fd: RawFd) -> std::io::Result<OwnedFd> {
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
pub(super) fn dup2(from: RawFd, to: RawFd) -> std::io::Result<()> {
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
pub(super) fn in_foreground(fd: RawFd) -> bool {
    // SAFETY: tcgetpgrp and getpgrp take no pointers.
    let (fg, own) = unsafe { (libc::tcgetpgrp(fd), libc::getpgrp()) };
    fg == own
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A pipe closed on exec as it is made (`pipe2` on Linux), so a child
    /// another test spawns meanwhile (a `sleep 600`) never holds its write
    /// end and keeps a read here from ending. macOS has no `pipe2`: there a
    /// spawn in the instant before the flag is set still could.
    pub(crate) fn private_pipe() -> (OwnedFd, OwnedFd) {
        let (read, write) = std::io::pipe().unwrap();
        (OwnedFd::from(read), OwnedFd::from(write))
    }

    /// A pty whose descriptors are closed on exec, so a child another test
    /// spawns meanwhile never holds the master open and hides its hangup.
    /// `openpty` takes no flag for it: a spawn in the instant before it is
    /// set still could.
    pub(crate) fn private_pty() -> nix::pty::OpenptyResult {
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
}
