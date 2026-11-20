//! The fd a captured bazel client gets as its stderr.
//!
//! [`open`] returns the child's end as a [`Stdio`] and the parent's read end
//! as a [`File`] for [`crate::stream::fd::Fd`]. Every fd is made `CLOEXEC`, so
//! a concurrent spawn on another thread does not inherit a write end; a leaked
//! writer would hold off EOF. `Command` clears the flag on the child's own
//! fd 2 when it installs it. Where the flag can only be set after the fd is
//! created (macOS pipes, every `openpty`), a spawn in between can still
//! inherit it; that costs the quiet window [`crate::stream::fd`] waits before
//! ending the stream, not a hang.
//!
//! The parent keeps no copy of a pipe's write end: the [`Stdio`] lives in the
//! `Command` until that is dropped, so EOF arrives once the child has exited
//! and the `Command` is gone.
//!
//! A PTY's slave it does keep a copy of ([`Capture::hold`]): on macOS, once
//! the last fd on the slave closes, what nobody has read yet survives well
//! under a second before the master reads EOF, so a forwarder running behind
//! (a held line, a slow terminal) would lose the end of the console. Its
//! reader drops the copy once the client has ended and it has read everything
//! ([`crate::stream::fd::Fd::holding`]).
//!
//! # Stdout on the same file
//!
//! Bazel writes action and test output to its stdout, and the message above
//! it (`INFO: From Testing …:`) to stderr. Were stdout to go to its
//! destination directly while stderr takes the detour through the capture,
//! the output could arrive before its header, and under a PTY one of curses'
//! erases could arrive after a stdout line and wipe it (the test summary,
//! say). [`open_with_stdout`] therefore gives the child the capture as its
//! stdout too, so both are one ordered stream, for a caller whose stdout and
//! stderr are the same file: one terminal, or one pipe after `2>&1`.

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::process::Stdio;

use nix::fcntl::{FcntlArg, FdFlag, fcntl};

use super::Mode;

/// Allocate the capture for `mode`: `(child stderr, parent read end)`. A PTY
/// that cannot be allocated falls back to a pipe; bazel, told it has a
/// terminal, then writes its curses UI into the pipe, which goes through the
/// stages like any other.
pub fn open(mode: Mode) -> io::Result<(Stdio, File)> {
    let (writer, reader) = open_fds(mode)?;
    Ok((Stdio::from(writer), File::from(reader)))
}

/// A capture for one child.
pub struct Capture {
    /// The child's stderr.
    pub stderr: Stdio,
    /// The child's stdout too, when asked for.
    pub stdout: Option<Stdio>,
    /// The parent's read end.
    pub reader: File,
    /// For a PTY, the parent's own copy of the slave, for the reader to hold
    /// until it has read everything (see the module docs).
    pub hold: Option<OwnedFd>,
}

/// [`open`], plus the same capture as the child's stdout when `stdout` asks
/// for it. Only for a caller whose stdout and stderr are the same file, since
/// what the capture reads is forwarded to stderr.
pub fn open_with_stdout(mode: Mode, stdout: bool) -> io::Result<Capture> {
    let (writer, reader, is_pty) = open_fds_kind(mode)?;
    let child_stdout = match stdout {
        true => Some(Stdio::from(writer.try_clone()?)),
        false => None,
    };
    let hold = if is_pty {
        Some(writer.try_clone()?)
    } else {
        None
    };
    Ok(Capture {
        stderr: Stdio::from(writer),
        stdout: child_stdout,
        reader: File::from(reader),
        hold,
    })
}

/// Whether this process's stdout and stderr are one and the same file (a
/// terminal, or one pipe after `2>&1`), so a capture may take both and keep
/// them in the order bazel wrote them.
pub fn stdout_is_stderrs_file() -> bool {
    let (out, err) = (io::stdout(), io::stderr());
    match (
        nix::sys::stat::fstat(out.as_fd()),
        nix::sys::stat::fstat(err.as_fd()),
    ) {
        (Ok(a), Ok(b)) => a.st_dev == b.st_dev && a.st_ino == b.st_ino,
        _ => false,
    }
}

/// [`open`] as `(write end, read end)`.
fn open_fds(mode: Mode) -> io::Result<(OwnedFd, OwnedFd)> {
    open_fds_kind(mode).map(|(w, r, _)| (w, r))
}

/// [`open_fds`], and whether it is a PTY after all.
fn open_fds_kind(mode: Mode) -> io::Result<(OwnedFd, OwnedFd, bool)> {
    match mode {
        Mode::Pipe => open_pipe().map(|(w, r)| (w, r, false)),
        Mode::Pty => open_pty().map(|(w, r)| (w, r, true)).or_else(|e| {
            tracing::debug!("PTY capture unavailable ({e}); falling back to a pipe");
            open_pipe().map(|(w, r)| (w, r, false))
        }),
    }
}

fn open_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let (reader, writer) = io::pipe()?;
    Ok((writer.into(), reader.into()))
}

/// The slave becomes the child's stderr and the master is read. The slave's
/// window size is copied from the real stderr so bazel wraps at the
/// terminal's width; it is not updated on a later resize.
fn open_pty() -> io::Result<(OwnedFd, OwnedFd)> {
    let winsize = terminal_size();
    let pty = nix::pty::openpty(winsize.as_ref(), None)?;
    set_cloexec(&pty.master)?;
    set_cloexec(&pty.slave)?;
    Ok((pty.slave, pty.master))
}

fn set_cloexec(fd: &impl AsFd) -> io::Result<()> {
    let flags = FdFlag::from_bits_truncate(fcntl(fd.as_fd(), FcntlArg::F_GETFD)?);
    fcntl(fd.as_fd(), FcntlArg::F_SETFD(flags | FdFlag::FD_CLOEXEC))?;
    Ok(())
}

/// The real stderr's window size, or `None` when it is not a terminal.
fn terminal_size() -> Option<nix::pty::Winsize> {
    use std::io::IsTerminal;
    use std::os::fd::AsRawFd;
    let stderr = io::stderr();
    if !stderr.is_terminal() {
        return None;
    }
    // SAFETY: TIOCGWINSZ writes one `winsize` through the pointer, which
    // points at a valid zeroed slot; the return code is checked.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(stderr.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) } != 0 {
        return None;
    }
    Some(nix::pty::Winsize {
        ws_row: ws.ws_row,
        ws_col: ws.ws_col,
        ws_xpixel: ws.ws_xpixel,
        ws_ypixel: ws.ws_ypixel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::forward::Forwarder;
    use crate::stream::Gate;
    use crate::stream::fd::Fd;
    use crate::testing::SharedSink;
    use std::process::Command;
    use std::time::Duration;

    /// Spawn `script` with the captured fd as its stderr, forward it to a
    /// sink, and return what arrived once the stream ends: at EOF, or `quiet`
    /// after the child exits if something else still holds the write end
    /// (another test's spawn can, through the window the module docs
    /// describe).
    fn round_trip(mode: Mode, script: &str, quiet: Duration) -> String {
        let Capture {
            stderr: child_stderr,
            reader,
            hold,
            ..
        } = open_with_stdout(mode, false).expect("open capture");
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(child_stderr)
            .spawn()
            .expect("spawn child");
        let pid = child.id();
        let mut gate = Gate::new(move || !crate::process::is_pid_alive(pid));
        gate.quiet = quiet;
        // Held as production holds it, so a writer the child left behind
        // still ends the stream by the gate's rules.
        let mut fd = Fd::follow(reader, gate).expect("follow");
        if let Some(hold) = hold {
            fd = fd.holding(hold);
        }
        let sink = SharedSink::default();
        let mut forwarder = Forwarder::spawn(Box::new(fd), Box::new(sink.clone()), vec![]).unwrap();
        child.wait().expect("child wait");
        forwarder.join().expect("forwarder join");
        sink.text()
    }

    #[test]
    fn pipe_round_trip() {
        let out = round_trip(
            Mode::Pipe,
            "printf 'err 1\\nerr 2\\n' 1>&2",
            Duration::from_secs(2),
        );
        assert_eq!(out, "err 1\nerr 2\n");
    }

    /// A capture that is the child's stdout too keeps both streams in the
    /// order they were written.
    fn takes_stdout_in_order(mode: Mode) {
        let Capture {
            stderr,
            stdout,
            reader,
            ..
        } = open_with_stdout(mode, true).unwrap();
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("printf 'err 1\\n' 1>&2; printf 'out 1\\n'; printf 'err 2\\n' 1>&2")
            .stdin(Stdio::null())
            .stdout(stdout.expect("the capture takes stdout"))
            .stderr(stderr)
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut gate = Gate::new(move || !crate::process::is_pid_alive(pid));
        gate.quiet = Duration::from_secs(2);
        let sink = SharedSink::default();
        let mut forwarder = Forwarder::spawn(
            Box::new(Fd::follow(reader, gate).unwrap()),
            Box::new(sink.clone()),
            vec![],
        )
        .unwrap();
        child.wait().unwrap();
        forwarder.join().unwrap();
        assert_eq!(sink.text().replace("\r\n", "\n"), "err 1\nout 1\nerr 2\n");
        assert!(open_with_stdout(mode, false).unwrap().stdout.is_none());
    }

    #[test]
    fn a_pty_takes_stdout_in_order() {
        takes_stdout_in_order(Mode::Pty);
    }

    #[test]
    fn a_pipe_takes_stdout_in_order() {
        takes_stdout_in_order(Mode::Pipe);
    }

    /// A forwarder that falls behind still gets everything the child wrote
    /// before it exited, PTY included (see the module docs on the slave).
    fn a_lagging_forwarder_loses_nothing(mode: Mode) {
        struct Slow(bool);
        impl crate::console::forward::LineProcessor for Slow {
            fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                if !std::mem::replace(&mut self.0, true) {
                    std::thread::sleep(Duration::from_millis(1500));
                }
                Some(record.to_vec())
            }
        }
        let capture = open_with_stdout(mode, false).unwrap();
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("i=0; while [ $i -lt 50 ]; do echo line $i 1>&2; i=$((i+1)); done; echo LAST 1>&2")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(capture.stderr)
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut gate = Gate::new(move || !crate::process::is_pid_alive(pid));
        gate.quiet = Duration::from_secs(2);
        let mut fd = Fd::follow(capture.reader, gate).unwrap();
        if let Some(hold) = capture.hold {
            fd = fd.holding(hold);
        }
        let sink = SharedSink::default();
        let mut forwarder = Forwarder::spawn(
            Box::new(fd),
            Box::new(sink.clone()),
            vec![Box::new(Slow(false))],
        )
        .unwrap();
        child.wait().unwrap();
        forwarder.join().unwrap();
        let out = sink.text();
        assert_eq!(out.matches("line ").count(), 50, "{mode:?}: {out}");
        assert!(out.contains("LAST"), "{mode:?}: {out}");
    }

    #[test]
    fn a_lagging_pipe_forwarder_loses_nothing() {
        a_lagging_forwarder_loses_nothing(Mode::Pipe);
    }

    #[test]
    fn a_lagging_pty_forwarder_loses_nothing() {
        a_lagging_forwarder_loses_nothing(Mode::Pty);
    }

    #[test]
    fn pty_round_trip() {
        // The PTY's line discipline turns `\n` into `\r\n`.
        let out = round_trip(
            Mode::Pty,
            "printf 'err 1\\nerr 2\\n' 1>&2",
            Duration::from_secs(2),
        );
        assert_eq!(out.replace("\r\n", "\n"), "err 1\nerr 2\n");
    }

    /// A background process the child leaves holding stderr must not keep the
    /// forwarder (and so `build.wait()`) running: the stream ends one quiet
    /// window after the child exits.
    fn outliving_writer_is_cut_off(mode: Mode) {
        let start = std::time::Instant::now();
        let out = round_trip(
            mode,
            "printf 'done\\n' 1>&2; sleep 2 &",
            Duration::from_millis(200),
        );
        assert!(out.contains("done"), "missing forwarded output in {out:?}");
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn pipe_outliving_writer_is_cut_off() {
        outliving_writer_is_cut_off(Mode::Pipe);
    }

    #[test]
    fn pty_outliving_writer_is_cut_off() {
        outliving_writer_is_cut_off(Mode::Pty);
    }

    /// Every fd of a capture is close-on-exec, so no spawn but the child's
    /// own (whose `Command` clears it on fd 2) inherits a write end.
    #[test]
    fn every_capture_fd_is_close_on_exec() {
        for mode in [Mode::Pipe, Mode::Pty] {
            let (writer, reader) = open_fds(mode).expect("open capture");
            for fd in [&writer, &reader] {
                let flags =
                    FdFlag::from_bits_truncate(fcntl(fd.as_fd(), FcntlArg::F_GETFD).unwrap());
                assert!(flags.contains(FdFlag::FD_CLOEXEC), "{mode:?}");
            }
        }
    }
}
