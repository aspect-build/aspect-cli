//! Following the read end of a pipe or PTY that a child writes as one of its
//! stdio fds.
//!
//! Unlike a FIFO, nothing can attach to it later, so EOF is final: every
//! writer has closed and the stream is [`Done::Drained`]. A PTY master
//! reports that as `EIO` on Linux, which is read the same way.
//!
//! A PTY's master can reach EOF with output still unread, which macOS then
//! discards, so a PTY reader holds a copy of the slave ([`Fd::holding`])
//! until a read made after the owner was seen ended finds nothing left.
//!
//! What EOF cannot settle is a writer that outlives the child: a background
//! process the child forked with the fd inherited holds it open indefinitely.
//! The [`Gate`] covers that case (R4 of the [parent module](super)): once the
//! child has ended and the fd has stayed quiet for the gate's window, the
//! stream is [`Done::Ended`]. A writer that keeps writing is cut off by the
//! gate's `linger` (R5), [`LINGER`] for a console.
//!
//! The fd is nonblocking and every wait is one [`TICK`] of `poll`, so a read
//! never parks in the kernel past that decision.

use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::os::fd::AsFd;

use nix::fcntl::{FcntlArg, OFlag, fcntl};

use std::time::Duration;

use super::{Activity, Done, Gate, Rules, wait_readable};

/// The `linger` a console capture's gate gets: how long a process the client
/// left behind may keep writing to its stderr before the capture stops.
pub const LINGER: Duration = Duration::from_secs(10);

/// The reading end of a child's stdio fd. A plain [`Read`]: bytes, or `Ok(0)`
/// once the stream is over, with the reason in [`Fd::done`].
pub struct Fd {
    file: File,
    rules: Rules,
    done: Option<Done>,
    activity: Option<Activity>,
    /// See [`Fd::holding`].
    hold: Option<std::os::fd::OwnedFd>,
    /// The owner was seen ended while holding: the next read that finds
    /// nothing lets go of the hold, everything written before is read.
    ended_seen: bool,
}

impl Fd {
    /// Start following `file`, switching it to nonblocking. `gate.ended`
    /// should report the writing child gone. The gate must not have a
    /// `held`: an fd has no path to ask it about, and EOF already says when
    /// no writer is left.
    pub fn follow(file: File, gate: Gate) -> io::Result<Fd> {
        let flags = OFlag::from_bits_truncate(fcntl(file.as_fd(), FcntlArg::F_GETFL)?);
        fcntl(file.as_fd(), FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
        Ok(Fd {
            file,
            rules: Rules::new(gate),
            done: None,
            activity: None,
            hold: None,
            ended_seen: false,
        })
    }

    /// Hold `fd`, a copy of a PTY's slave, so the master cannot reach EOF and
    /// drop what is unread while the reader is behind. Let go of once a read
    /// made after the owner was seen ended finds nothing left: anything it
    /// wrote is in the buffer by then, and closing the last slave fd drops
    /// what is unread at once. The next read then sees EOF as soon as every
    /// other writer is gone.
    pub fn holding(mut self, fd: std::os::fd::OwnedFd) -> Self {
        self.hold = Some(fd);
        self
    }

    /// Report to `activity` around each wait for bytes: waiting while blocked,
    /// busy the moment a wait ends, before the read that takes the bytes.
    pub fn reporting(mut self, activity: Activity) -> Self {
        self.activity = Some(activity);
        self
    }

    /// Why the stream ended. `None` until a read has returned `Ok(0)`.
    pub fn done(&self) -> Option<&Done> {
        self.done.as_ref()
    }
}

impl Read for Fd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.done.is_some() {
                return Ok(0);
            }
            // Marked reading before the read that may take bytes, so it is
            // never seen waiting while bytes leave the stream; one that
            // finds none unmarks it below, and `Activity::busy` the rest.
            if let Some(a) = &self.activity {
                a.reading(true);
            }
            match self.file.read(buf) {
                Ok(0) => {
                    self.done = Some(Done::Drained {
                        bytes: self.rules.bytes(),
                    });
                    return Ok(0);
                }
                Ok(n) => {
                    self.rules.arrived(n);
                    if let Some(done) = self.rules.lingered() {
                        tracing::debug!("stream cut: {done}");
                        self.done = Some(done);
                    }
                    return Ok(n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if let Some(a) = &self.activity {
                        a.reading(false);
                    }
                    if self.hold.is_some() {
                        if self.ended_seen {
                            self.hold = None;
                            continue;
                        }
                        if self.rules.owner_ended() {
                            // Read once more: the owner may have written its
                            // last bytes after the read that just found none.
                            self.ended_seen = true;
                            continue;
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if e.raw_os_error() == Some(libc::EIO) => {
                    self.done = Some(Done::Drained {
                        bytes: self.rules.bytes(),
                    });
                    return Ok(0);
                }
                Err(error) => {
                    // Raised once, so the stream's reader can report it; a
                    // later read finds the stream over.
                    let raised = io::Error::new(error.kind(), error.to_string());
                    self.done = Some(self.rules.broken(error));
                    return Err(raised);
                }
            }
            // An fd has no path, and its gates no `held` to ask about one.
            if let Some(done) = self.rules.judge(false, std::path::Path::new("")) {
                if matches!(done, Done::Cut { .. }) {
                    tracing::debug!("stream cut: {done}");
                }
                self.done = Some(done);
                return Ok(0);
            }
            if let Some(a) = &self.activity {
                a.waiting();
            }
            if wait_readable(&self.file)
                && let Some(a) = &self.activity
            {
                a.busy();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    fn pipe() -> (File, File) {
        let (r, w) = std::io::pipe().unwrap();
        (
            File::from(std::os::fd::OwnedFd::from(r)),
            File::from(std::os::fd::OwnedFd::from(w)),
        )
    }

    fn gate(ended: &Arc<AtomicBool>, quiet: Duration) -> Gate {
        let e = ended.clone();
        let mut gate = Gate::new(move || e.load(Ordering::SeqCst));
        gate.quiet = quiet;
        gate.quiet_empty = quiet;
        gate
    }

    #[test]
    fn every_writer_closing_drains_the_stream() {
        let (r, mut w) = pipe();
        let ended = Arc::new(AtomicBool::new(false));
        let mut fd = Fd::follow(r, gate(&ended, Duration::from_secs(60))).unwrap();
        w.write_all(b"hello").unwrap();
        drop(w);
        let mut out = Vec::new();
        fd.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"hello");
        assert!(matches!(fd.done(), Some(Done::Drained { bytes: 5 })));
    }

    #[test]
    fn a_writer_outliving_the_child_is_cut_off_after_the_quiet_window() {
        let (r, mut w) = pipe();
        let ended = Arc::new(AtomicBool::new(false));
        let mut fd = Fd::follow(r, gate(&ended, Duration::from_millis(50))).unwrap();
        w.write_all(b"tail").unwrap();
        ended.store(true, Ordering::SeqCst);
        let start = Instant::now();
        let mut out = Vec::new();
        fd.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"tail");
        assert!(matches!(fd.done(), Some(Done::Ended { bytes: 4, .. })));
        assert!(start.elapsed() < Duration::from_secs(5));
        drop(w);
    }

    #[test]
    fn a_writer_that_keeps_writing_is_cut_off_after_the_linger() {
        let (r, mut w) = pipe();
        let ended = Arc::new(AtomicBool::new(true));
        let mut gate = gate(&ended, Duration::from_secs(60));
        gate.linger = Some(Duration::from_millis(200));
        let mut fd = Fd::follow(r, gate).unwrap();
        let writer = std::thread::spawn(move || {
            while w.write_all(b"tick\n").is_ok() {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let start = Instant::now();
        let mut out = Vec::new();
        fd.read_to_end(&mut out).unwrap();
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(
            matches!(fd.done(), Some(Done::Cut { .. })),
            "{:?}",
            fd.done()
        );
        drop(fd);
        writer.join().unwrap();
    }

    /// The linger runs from when the reader caught up, so a dead writer's
    /// backlog is read in full however slowly.
    #[test]
    fn a_slow_reader_still_gets_a_dead_writers_backlog() {
        let (r, mut w) = pipe();
        w.write_all(&[b'x'; 16 * 1024]).unwrap();
        drop(w);
        let ended = Arc::new(AtomicBool::new(true));
        let mut gate = gate(&ended, Duration::from_secs(60));
        gate.linger = Some(Duration::from_millis(100));
        let mut fd = Fd::follow(r, gate).unwrap();
        let mut total = 0;
        let mut buf = [0u8; 512];
        loop {
            let n = fd.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            total += n;
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(total, 16 * 1024);
        assert!(
            matches!(fd.done(), Some(Done::Drained { .. })),
            "{:?}",
            fd.done()
        );
    }

    /// A writer faster than the reader never lets a read find nothing; it
    /// is cut all the same, once it has written more than any buffer holds.
    #[test]
    fn a_writer_that_outpaces_the_reader_is_cut_off() {
        let (r, mut w) = pipe();
        let ended = Arc::new(AtomicBool::new(true));
        let mut gate = gate(&ended, Duration::from_secs(60));
        gate.linger = Some(Duration::from_millis(200));
        let mut fd = Fd::follow(r, gate).unwrap();
        let writer = std::thread::spawn(move || while w.write_all(&[b'x'; 4096]).is_ok() {});
        let start = Instant::now();
        let mut buf = [0u8; 4096];
        while fd.read(&mut buf).unwrap() > 0 {
            std::thread::sleep(Duration::from_micros(200));
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        assert!(
            matches!(fd.done(), Some(Done::Cut { .. })),
            "{:?}",
            fd.done()
        );
        drop(fd);
        writer.join().unwrap();
    }

    /// Waiting only while blocked, busy from the moment bytes are seen: a
    /// watcher never sees the reader waiting while it takes bytes.
    #[test]
    fn reporting_marks_only_blocked_time_as_waiting() {
        let (r, mut w) = pipe();
        let ended = Arc::new(AtomicBool::new(false));
        let activity = Activity::default();
        activity.busy();
        let mut fd = Fd::follow(r, gate(&ended, Duration::from_secs(60)))
            .unwrap()
            .reporting(activity.clone());
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 16];
            let n = fd.read(&mut buf).unwrap();
            (fd, n)
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while activity.is_busy() {
            assert!(Instant::now() < deadline, "never reported waiting");
            std::thread::sleep(Duration::from_millis(5));
        }
        w.write_all(b"x").unwrap();
        let (_fd, n) = reader.join().unwrap();
        assert_eq!(n, 1);
        assert!(activity.is_busy(), "busy once the read took bytes");
    }

    /// A silent stream's idle time keeps growing across poll ticks.
    #[test]
    fn a_reporting_readers_idle_time_grows() {
        let (r, w) = pipe();
        let ended = Arc::new(AtomicBool::new(false));
        let activity = Activity::default();
        let mut fd = Fd::follow(r, gate(&ended, Duration::from_secs(60)))
            .unwrap()
            .reporting(activity.clone());
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            fd.read_to_end(&mut out).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while activity.idle_ms() < 200 {
            assert!(Instant::now() < deadline, "idle time never grew");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(w);
        reader.join().unwrap();
    }

    /// The owner's last bytes, written in the moment between an empty read
    /// and the check that it has ended, are not lost by letting go of the
    /// hold: on macOS a living process's close of the last slave fd drops
    /// what is unread at once.
    #[test]
    fn letting_go_of_the_hold_loses_no_last_bytes() {
        let pty = nix::pty::openpty(None, None).unwrap();
        let (master, slave) = (File::from(pty.master), pty.slave);
        let writer = slave.try_clone().unwrap();
        let mut writer = Some(File::from(writer));
        let gate = Gate::new(move || {
            // The owner writes and exits just as it is first asked about.
            if let Some(mut w) = writer.take() {
                w.write_all(b"LAST LINE\n").unwrap();
            }
            true
        });
        let mut fd = Fd::follow(master, gate).unwrap().holding(slave);
        let mut out = Vec::new();
        fd.read_to_end(&mut out).unwrap();
        assert!(
            String::from_utf8_lossy(&out).contains("LAST LINE"),
            "{out:?}"
        );
    }

    #[test]
    fn a_living_child_keeps_a_quiet_stream_open() {
        let (r, w) = pipe();
        let ended = Arc::new(AtomicBool::new(false));
        let mut fd = Fd::follow(r, gate(&ended, Duration::from_millis(10))).unwrap();
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            fd.read_to_end(&mut out).unwrap();
            fd
        });
        std::thread::sleep(Duration::from_millis(100));
        assert!(!reader.is_finished(), "ended only once the child is gone");
        drop(w);
        let fd = reader.join().unwrap();
        assert!(matches!(fd.done(), Some(Done::Drained { bytes: 0 })));
    }
}
