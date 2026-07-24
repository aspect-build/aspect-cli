//! Allocate the captured-stderr fd for a Bazel invocation.
//!
//! When a build is spawned with `output = bazel.output.processor(...)`, the
//! child's stderr must go to a fd the runtime controls instead of being
//! inherited, so [`super::stream::OutputStream`] can read, process, and forward
//! it. Two modes:
//!
//! - [`CaptureMode::Pipe`] — a plain anonymous pipe. Used in non-TTY contexts;
//!   Bazel emits clean newline-terminated lines.
//! - [`CaptureMode::Pty`] — a pseudo-terminal. The slave becomes the child's
//!   stderr; we read the master. Bazel keeps its live curses UI. Unix-only;
//!   callers fall back to `Pipe` on platforms without `openpty`.
//!
//! # The slave-drop discipline (PTY)
//!
//! The parent MUST drop its copy of the PTY slave fd right after spawning the
//! child, or the master read never sees EOF and the forwarder thread hangs
//! forever. [`Capture`] holds the slave in `parent_slave` precisely so the
//! caller can drop it (via [`Capture::release_after_spawn`]) at the right
//! moment — after `cmd.spawn()` has dup'd it into the child.

use std::io::Read;
use std::process::Stdio;

use super::build::CaptureMode;

/// The parent's side of a captured invocation: the read end the forwarder
/// consumes, and (PTY only) the retained slave fd.
///
/// The child's stderr `Stdio` is returned separately by [`Capture::open`] so
/// the caller moves it straight into the `Command` and this handle owns only
/// what the parent keeps.
pub struct Capture {
    /// The read end the `OutputStream` reads from.
    pub reader: Box<dyn Read + Send>,
    /// PTY slave fd retained by the parent; dropped after spawn so the master
    /// can observe EOF. `None` for the pipe path.
    parent_slave: Option<std::fs::File>,
}

impl Capture {
    /// Drop the parent's retained slave fd (no-op for the pipe path). Call
    /// immediately after `cmd.spawn()` so the master can observe EOF once the
    /// child exits.
    pub fn release_after_spawn(&mut self) {
        self.parent_slave = None;
    }

    /// Allocate the capture fds for `mode`, returning the child's stderr
    /// `Stdio` alongside the parent-side handle. Falls back to a pipe if PTY
    /// allocation isn't available on this platform or fails.
    pub fn open(mode: CaptureMode) -> std::io::Result<(Stdio, Capture)> {
        match mode {
            CaptureMode::Pipe => open_pipe(),
            CaptureMode::Pty => match open_pty() {
                Ok(c) => Ok(c),
                Err(e) => {
                    crate::trace!("PTY capture unavailable ({e}); falling back to pipe");
                    open_pipe()
                }
            },
        }
    }
}

/// Anonymous pipe: the child writes the write end (its stderr), we read the
/// read end.
///
/// Both ends are opened `O_CLOEXEC` so a concurrent `fork`+`exec` on another
/// thread can't leak the write end into an unrelated child — a leaked writer
/// would keep the read end from ever seeing EOF, hanging the forwarder. The
/// child's own stderr is set up by `Command`'s `Stdio` dup, which re-clears
/// CLOEXEC on fd 2, so the child still inherits it correctly.
#[cfg(unix)]
fn open_pipe() -> std::io::Result<(Stdio, Capture)> {
    use std::os::fd::FromRawFd;

    // `pipe(2)` (not `pipe2`, which macOS lacks) + explicit CLOEXEC on both
    // ends. CLOEXEC keeps a concurrent `fork`+`exec` on another thread from
    // leaking the writer into an unrelated child — a leaked writer would keep
    // the reader from ever seeing EOF, hanging the forwarder. `Command`'s
    // `Stdio` dup re-clears CLOEXEC on the child's fd 2, so the Bazel child
    // still inherits its stderr.
    let mut fds = [0_i32; 2];
    // SAFETY: `pipe` writes two valid fds into `fds` on success; each is
    // wrapped in an owning `File` exactly once below.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let reader = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let writer = unsafe { std::fs::File::from_raw_fd(fds[1]) };
    set_cloexec(&reader)?;
    set_cloexec(&writer)?;
    Ok((
        Stdio::from(writer),
        Capture {
            reader: Box::new(reader),
            parent_slave: None,
        },
    ))
}

#[cfg(not(unix))]
fn open_pipe() -> std::io::Result<(Stdio, Capture)> {
    Err(std::io::Error::other(
        "output capture is only supported on Unix",
    ))
}

/// Allocate a PTY, returning the master as the reader and the slave as the
/// child's stderr. The slave is dup'd: one copy goes to the child (via the
/// `Stdio`), one is retained in `parent_slave` to be dropped after spawn.
///
/// All three fds are `O_CLOEXEC`. The master and the parent's retained slave
/// must not leak into any child. The slave handed to the child is `CLOEXEC`
/// too — `Command` re-clears it on fd 2 via the `Stdio` dup, so the Bazel
/// child still gets it, but a concurrent `fork`+`exec` on another thread can't
/// leak it into an unrelated child (which would keep the master from ever
/// seeing EOF and hang the forwarder).
#[cfg(unix)]
fn open_pty() -> std::io::Result<(Stdio, Capture)> {
    use nix::pty::openpty;
    use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};

    // Seed the slave winsize from the real terminal so Bazel wraps at the
    // right width; best-effort (ignored if stderr isn't a terminal).
    let winsize = current_winsize();
    let pty = openpty(winsize.as_ref(), None).map_err(std::io::Error::from)?;
    let master: OwnedFd = pty.master;
    let slave: OwnedFd = pty.slave;

    set_cloexec(&master)?;
    set_cloexec(&slave)?;

    // Two owning handles to the slave: one becomes the child's stderr Stdio,
    // one is retained so the parent can hold the slave open until just after
    // spawn (then drop it so the master observes EOF on child exit).
    let slave_for_child = slave.try_clone()?; // inherits CLOEXEC
    let child_stderr = unsafe { Stdio::from_raw_fd(slave_for_child.into_raw_fd()) };
    let parent_slave = unsafe { std::fs::File::from_raw_fd(slave.into_raw_fd()) };
    let reader = unsafe { std::fs::File::from_raw_fd(master.into_raw_fd()) };

    Ok((
        child_stderr,
        Capture {
            reader: Box::new(reader),
            parent_slave: Some(parent_slave),
        },
    ))
}

#[cfg(not(unix))]
fn open_pty() -> std::io::Result<(Stdio, Capture)> {
    Err(std::io::Error::other("PTY capture is only supported on Unix"))
}

#[cfg(unix)]
fn set_cloexec<F: std::os::fd::AsFd>(fd: &F) -> std::io::Result<()> {
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    let raw = fcntl(fd, FcntlArg::F_GETFD).map_err(std::io::Error::from)?;
    let mut flags = FdFlag::from_bits_truncate(raw);
    flags.insert(FdFlag::FD_CLOEXEC);
    fcntl(fd, FcntlArg::F_SETFD(flags)).map_err(std::io::Error::from)?;
    Ok(())
}

/// The current terminal's window size, read from the real stderr. `None` when
/// stderr isn't a terminal (the PTY then uses the kernel default).
#[cfg(unix)]
fn current_winsize() -> Option<nix::pty::Winsize> {
    use std::io::IsTerminal;
    use std::os::fd::AsRawFd;
    let stderr = std::io::stderr();
    if !stderr.is_terminal() {
        return None;
    }
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes a `winsize` through the pointer; we pass a
    // valid stack slot and check the return code.
    let rc = unsafe { libc::ioctl(stderr.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) };
    if rc != 0 {
        return None;
    }
    Some(nix::pty::Winsize {
        ws_row: ws.ws_row,
        ws_col: ws.ws_col,
        ws_xpixel: ws.ws_xpixel,
        ws_ypixel: ws.ws_ypixel,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::engine::bazel::stream::OutputStream;
    use std::io::Write;
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// End-to-end through the real capture machinery + `OutputStream`: open a
    /// capture, spawn a child whose stderr is the captured fd, forward to a
    /// sink, and assert the forwarder drains everything and terminates (no
    /// hang). This exercises the slave-drop / EOF / EIO discipline that the
    /// `Cursor`-based `OutputStream` unit tests can't.
    fn round_trip(mode: CaptureMode) {
        let (child_stderr, mut capture) = Capture::open(mode).expect("open capture");

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("printf 'err line 1\\nerr line 2\\n' 1>&2")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(child_stderr)
            .spawn()
            .expect("spawn child");

        // The #1 PTY mistake: the parent must drop its slave copy so the
        // master read observes EOF. (No-op for the pipe path.)
        capture.release_after_spawn();

        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut stream = OutputStream::spawn(
            capture.reader,
            Box::new(SharedSink(sink.clone())),
            vec![],
        );

        child.wait().expect("child wait");
        // join() must return — if the slave/EOF handling were wrong this hangs.
        stream.join().expect("stream join");

        let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        assert!(out.contains("err line 1"), "missing line 1 in {out:?}");
        assert!(out.contains("err line 2"), "missing line 2 in {out:?}");
    }

    #[test]
    fn pipe_round_trip() {
        round_trip(CaptureMode::Pipe);
    }

    #[test]
    fn pty_round_trip() {
        round_trip(CaptureMode::Pty);
    }

    /// An unrelated child spawned while the capture is open must NOT inherit
    /// the captured fd. Without `CLOEXEC` on the pipe/PTY ends, a long-lived
    /// interposer would keep the write end open and the forwarder would never
    /// see EOF after the real child exits — hanging `join()` forever.
    ///
    /// Here `sleep 30` inherits the parent's fds (default `Command` stdio). The
    /// real child then writes and exits; the forwarder must terminate promptly
    /// despite the sleep still running.
    fn cloexec_no_hang(mode: CaptureMode) {
        let (child_stderr, mut capture) = Capture::open(mode).expect("open capture");

        // Interposer spawned AFTER the capture fds exist, BEFORE the real
        // child — the exact window in which a non-CLOEXEC fd would leak.
        let mut interposer = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn interposer");

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("printf 'done\\n' 1>&2")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(child_stderr)
            .spawn()
            .expect("spawn child");
        capture.release_after_spawn();

        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut stream =
            OutputStream::spawn(capture.reader, Box::new(SharedSink(sink.clone())), vec![]);

        child.wait().expect("child wait");
        // Terminates only because the interposer did NOT inherit the write end.
        stream.join().expect("stream join");
        let _ = interposer.kill();
        let _ = interposer.wait();

        let out = String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        assert!(out.contains("done"), "missing forwarded output in {out:?}");
    }

    #[test]
    fn pipe_cloexec_no_hang() {
        cloexec_no_hang(CaptureMode::Pipe);
    }

    #[test]
    fn pty_cloexec_no_hang() {
        cloexec_no_hang(CaptureMode::Pty);
    }
}
