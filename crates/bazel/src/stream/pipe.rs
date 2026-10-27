//! Following a FIFO that a process we launched writes to.
//!
//! The reader never parks in the kernel: the FIFO is opened nonblocking, and
//! every wait is a short slice in which the [`Rules`] ask two questions of
//! whoever started the writers (see [`Gate`]). That is what lets it end a
//! stream no writer will ever finish, without a thread to poke a blocked open
//! and without a read that can only be abandoned.
//!
//! [`Pipe`] is a plain [`Read`]: bytes, or `Ok(0)` once the stream is over.
//! Why it is over is [`Pipe::done`]. The rules are in the [parent
//! module](super).

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use nix::sys::stat::Mode;
use nix::unistd::mkfifo;

use super::{Done, Gate, Rules, TICK};

/// A FIFO reserved at a path, before anything is spawned: the path goes on
/// the command line, then [`Channel::follow`] starts reading.
///
/// Dropped without following, it removes the FIFO.
pub struct Channel {
    path: PathBuf,
}

impl Channel {
    /// A new FIFO in the temp dir named `<name>-<uuid>.bin`.
    pub fn fifo(name: &str) -> io::Result<Self> {
        Self::at(std::env::temp_dir().join(format!("{name}-{}.bin", uuid::Uuid::new_v4())))
    }

    /// A FIFO at `path`. Creating one that already exists is not an error.
    pub fn at(path: PathBuf) -> io::Result<Self> {
        match mkfifo(&path, Mode::S_IRWXU | Mode::S_IRWXG | Mode::S_IRWXO) {
            Ok(()) | Err(nix::errno::Errno::EEXIST) => Ok(Self { path }),
            Err(e) => Err(e.into()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Start reading. Never blocks: a writer that has not arrived yet is just
    /// an empty read, and the gate decides what that means.
    pub fn follow(mut self, gate: Gate) -> io::Result<Pipe> {
        let path = std::mem::take(&mut self.path);
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)?;
        let meta = file.metadata()?;
        Ok(Pipe {
            file,
            path,
            ident: (meta.dev(), meta.ino()),
            rules: Rules::new(gate),
            done: None,
        })
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The reading end of a [`Channel`]. See the [parent module](super) for when
/// it ends.
pub struct Pipe {
    file: File,
    path: PathBuf,
    /// Device and inode of the FIFO as opened, to notice the path being
    /// removed or replaced.
    ident: (u64, u64),
    rules: Rules,
    done: Option<Done>,
}

impl Pipe {
    /// Why the stream ended. `None` until a read has returned `Ok(0)`.
    pub fn done(&self) -> Option<&Done> {
        self.done.as_ref()
    }

    /// [`Pipe::done`], moved out. For a layer on top that reports the reason
    /// onward and has no further use for the pipe.
    pub fn take_done(&mut self) -> Option<Done> {
        self.done.take()
    }

    /// Bytes delivered so far. (Not `bytes`, which `Read::bytes` would win
    /// over at every call.)
    pub fn bytes_read(&self) -> u64 {
        self.rules.bytes()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The protocol says a writer will reconnect, such as a build retrying
    /// after the previous attempt closed the file. While set, a gap with no
    /// writer is a pause and not the end (R2 is off); the owner ending still
    /// ends the stream (R4).
    pub fn expect_more(&mut self, yes: bool) {
        self.rules.expect_more(yes);
    }

    fn still_names_this_fifo(&self) -> bool {
        std::fs::metadata(&self.path)
            .map(|m| (m.dev(), m.ino()) == self.ident)
            .unwrap_or(false)
    }

    /// R1, then the shared rules, for a read that returned nothing. `eof` is
    /// whether the read saw zero bytes, as opposed to a writer attached but
    /// quiet.
    fn judge(&mut self, eof: bool) -> Option<Done> {
        if !self.still_names_this_fifo() {
            return Some(self.rules.broken(io::Error::new(
                ErrorKind::NotFound,
                "the channel path no longer names the FIFO being read",
            )));
        }
        self.rules.judge(eof)
    }

    /// Wait one tick for bytes. On EOF a poll would return at once (a hung-up
    /// FIFO is always readable), so that case sleeps.
    fn wait(&self, eof: bool) {
        if eof {
            std::thread::sleep(TICK);
            return;
        }
        let mut fd = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `fd` is a valid pollfd and the count is one. An interrupted
        // or failed poll only shortens the wait; the rules run again anyway.
        unsafe { libc::poll(&mut fd, 1, TICK.as_millis() as libc::c_int) };
    }
}

impl Read for Pipe {
    /// Bytes, or `Ok(0)` once the stream is over (see [`Pipe::done`]).
    /// Blocks, in slices, until one of the two. Never returns `WouldBlock`,
    /// `BrokenPipe` or an error that is the stream's own end: those are
    /// [`Done`].
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // `Read` requires an empty buffer to return immediately.
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.done.is_some() {
                return Ok(0);
            }
            let eof = match self.file.read(buf) {
                Ok(0) => true,
                Ok(n) => {
                    self.rules.arrived(n);
                    return Ok(n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => false,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.done = Some(self.rules.broken(error));
                    return Ok(0);
                }
            };
            if let Some(done) = self.judge(eof) {
                self.done = Some(done);
                return Ok(0);
            }
            self.wait(eof);
        }
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::FileTypeExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    const QUIET_TEST: Duration = Duration::from_millis(200);
    const QUIET_EMPTY_TEST: Duration = Duration::from_millis(100);

    /// A follower whose owner and writers are flags the test flips.
    struct Rig {
        follower: Pipe,
        path: PathBuf,
        ended: Arc<AtomicBool>,
        held: Arc<AtomicBool>,
        ended_calls: Arc<AtomicUsize>,
        held_calls: Arc<AtomicUsize>,
    }

    impl Rig {
        fn new() -> Self {
            Self::with(true)
        }

        /// `with_held` false builds a gate with no `held` hint at all.
        fn with(with_held: bool) -> Self {
            let channel = Channel::fifo("bazel-pipe-test").unwrap();
            let path = channel.path().to_path_buf();
            let ended = Arc::new(AtomicBool::new(false));
            let held = Arc::new(AtomicBool::new(true));
            let ended_calls = Arc::new(AtomicUsize::new(0));
            let held_calls = Arc::new(AtomicUsize::new(0));
            let (e, ec) = (ended.clone(), ended_calls.clone());
            let mut gate = Gate::new(move || {
                ec.fetch_add(1, Ordering::SeqCst);
                e.load(Ordering::SeqCst)
            });
            if with_held {
                let (h, hc) = (held.clone(), held_calls.clone());
                gate = gate.with_held(move || {
                    hc.fetch_add(1, Ordering::SeqCst);
                    h.load(Ordering::SeqCst)
                });
            }
            gate.quiet = QUIET_TEST;
            gate.quiet_empty = QUIET_EMPTY_TEST;
            Self {
                follower: channel.follow(gate).unwrap(),
                path,
                ended,
                held,
                ended_calls,
                held_calls,
            }
        }

        fn writer(&self) -> std::fs::File {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&self.path)
                .unwrap()
        }

        fn end_owner(&self) {
            self.ended.store(true, Ordering::SeqCst);
        }

        fn release(&self) {
            self.held.store(false, Ordering::SeqCst);
        }
    }

    /// Reads a follower to its end on a thread, handing it back with what it
    /// read. A receive that times out means the read is still waiting.
    fn read_in_background(mut f: Pipe) -> mpsc::Receiver<(Vec<u8>, Pipe)> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            f.read_to_end(&mut out).unwrap();
            let _ = tx.send((out, f));
        });
        rx
    }

    fn still_waiting(rx: &mpsc::Receiver<(Vec<u8>, Pipe)>) -> bool {
        matches!(
            rx.recv_timeout(Duration::from_millis(400)),
            Err(mpsc::RecvTimeoutError::Timeout)
        )
    }

    fn finished(rx: mpsc::Receiver<(Vec<u8>, Pipe)>) -> (Vec<u8>, Pipe) {
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the read should have ended")
    }

    // R5 and the byte path

    #[test]
    fn bytes_come_through_in_order() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(b"hello ").unwrap();
        w.write_all(b"world").unwrap();
        let mut buf = [0u8; 11];
        rig.follower.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello world");
        assert_eq!(rig.follower.bytes_read(), 11);
        assert!(rig.follower.done().is_none());
    }

    #[test]
    fn a_stream_larger_than_the_pipe_arrives_whole() {
        let rig = Rig::new();
        rig.release();
        let mut w = rig.writer();
        let payload: Vec<u8> = (0..2_000_000u32).map(|i| (i % 251) as u8).collect();
        let sent = payload.clone();
        let writer = std::thread::spawn(move || w.write_all(&sent).unwrap());
        let (got, f) = finished(read_in_background(rig.follower));
        writer.join().unwrap();
        assert_eq!(got, payload);
        assert!(matches!(f.done(), Some(Done::Drained { bytes: 2_000_000 })));
    }

    #[test]
    fn an_empty_buffer_reads_zero_without_ending_anything() {
        let mut rig = Rig::new();
        assert_eq!(rig.follower.read(&mut []).unwrap(), 0);
        assert!(rig.follower.done().is_none());
    }

    #[test]
    fn before_any_writer_with_the_owner_alive_the_read_waits() {
        let rig = Rig::new();
        let ended = rig.ended.clone();
        let rx = read_in_background(rig.follower);
        assert!(still_waiting(&rx), "no writer yet is not an end");
        ended.store(true, Ordering::SeqCst);
        let (got, f) = finished(rx);
        assert!(got.is_empty());
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 0, .. })));
    }

    #[test]
    fn a_writer_arriving_late_is_read() {
        let rig = Rig::new();
        let mut w = rig.writer();
        let held = rig.held.clone();
        let rx = read_in_background(rig.follower);
        assert!(still_waiting(&rx));
        w.write_all(b"late").unwrap();
        held.store(false, Ordering::SeqCst);
        drop(w);
        let (got, f) = finished(rx);
        assert_eq!(got, b"late");
        assert!(matches!(f.done(), Some(Done::Drained { bytes: 4 })));
    }

    #[test]
    fn eof_with_no_hint_and_the_owner_alive_never_ends() {
        let mut rig = Rig::with(false);
        {
            let mut w = rig.writer();
            w.write_all(b"x").unwrap();
        }
        let mut one = [0u8; 1];
        rig.follower.read_exact(&mut one).unwrap();
        let ended = rig.ended.clone();
        let rx = read_in_background(rig.follower);
        assert!(
            still_waiting(&rx),
            "without a hint only the owner can end it"
        );
        ended.store(true, Ordering::SeqCst);
        let (_, f) = finished(rx);
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 1, .. })));
    }

    // R2

    #[test]
    fn r2_everyone_let_go_after_bytes_is_drained() {
        let rig = Rig::new();
        rig.release();
        {
            let mut w = rig.writer();
            w.write_all(b"payload").unwrap();
        }
        let (got, f) = finished(read_in_background(rig.follower));
        assert_eq!(got, b"payload");
        assert!(matches!(f.done(), Some(Done::Drained { bytes: 7 })));
    }

    #[test]
    fn r2_still_held_is_not_drained() {
        let rig = Rig::new(); // held stays true
        {
            let mut w = rig.writer();
            w.write_all(b"payload").unwrap();
        }
        let ended = rig.ended.clone();
        let rx = read_in_background(rig.follower);
        assert!(still_waiting(&rx), "the server may come back to the file");
        ended.store(true, Ordering::SeqCst);
        let (_, f) = finished(rx);
        assert!(matches!(f.done(), Some(Done::Ended { .. })));
    }

    #[test]
    fn r2_needs_bytes_so_an_empty_open_and_close_is_not_the_end() {
        let rig = Rig::new();
        rig.release();
        drop(rig.writer()); // attached and gone, nothing written
        let ended = rig.ended.clone();
        let rx = read_in_background(rig.follower);
        assert!(
            still_waiting(&rx),
            "zero bytes with the owner alive is 'not yet'"
        );
        ended.store(true, Ordering::SeqCst);
        let (_, f) = finished(rx);
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 0, .. })));
    }

    #[test]
    fn r2_is_off_while_more_is_expected() {
        let mut rig = Rig::new();
        rig.release();
        {
            let mut w = rig.writer();
            w.write_all(b"one").unwrap();
        }
        let mut buf = [0u8; 3];
        rig.follower.read_exact(&mut buf).unwrap();
        rig.follower.expect_more(true);

        let path = rig.path.clone();
        let second = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let mut w = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            w.write_all(b"two").unwrap();
        });
        let mut next = [0u8; 3];
        rig.follower.read_exact(&mut next).unwrap();
        second.join().unwrap();
        assert_eq!(&next, b"two", "the reader waited out the gap");

        rig.follower.expect_more(false);
        let mut rest = Vec::new();
        rig.follower.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty());
        assert!(matches!(
            rig.follower.done(),
            Some(Done::Drained { bytes: 6 })
        ));
    }

    #[test]
    fn expecting_more_does_not_outlast_the_owner() {
        let mut rig = Rig::new();
        rig.release();
        {
            let mut w = rig.writer();
            w.write_all(b"one").unwrap();
        }
        rig.follower.expect_more(true);
        rig.end_owner();
        let mut out = Vec::new();
        let t = Instant::now();
        rig.follower.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"one");
        assert!(t.elapsed() >= QUIET_TEST, "R4's window applied");
        assert!(matches!(
            rig.follower.done(),
            Some(Done::Ended { bytes: 3, .. })
        ));
    }

    #[test]
    fn r2_wins_over_r4_when_the_owner_ended_and_nobody_holds_the_file() {
        let mut rig = Rig::new();
        rig.release();
        rig.end_owner();
        {
            let mut w = rig.writer();
            w.write_all(b"done").unwrap();
        }
        let t = Instant::now();
        let mut out = Vec::new();
        rig.follower.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"done");
        assert!(t.elapsed() < QUIET_TEST, "no quiet window was waited");
        assert!(matches!(
            rig.follower.done(),
            Some(Done::Drained { bytes: 4 })
        ));
    }

    // R3

    #[test]
    fn r3_an_owner_that_ended_without_writing_ends_it_now_when_nobody_holds_the_file() {
        let mut rig = Rig::new();
        rig.release();
        rig.end_owner();
        rig.follower.rules.gate.quiet_empty = Duration::from_secs(30);
        let t = Instant::now();
        let mut out = Vec::new();
        rig.follower.read_to_end(&mut out).unwrap();
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "R3 does not wait for the window"
        );
        match rig.follower.done() {
            Some(Done::Ended { bytes: 0, quiet }) => assert_eq!(*quiet, Duration::ZERO),
            other => panic!("expected an immediate Ended, got {other:?}"),
        }
    }

    #[test]
    fn r3_without_the_hint_waits_the_short_window() {
        let mut rig = Rig::with(false);
        rig.end_owner();
        let t = Instant::now();
        let mut out = Vec::new();
        rig.follower.read_to_end(&mut out).unwrap();
        let waited = t.elapsed();
        assert!(waited >= QUIET_EMPTY_TEST, "waited {waited:?}");
        assert!(waited < QUIET_TEST + Duration::from_secs(2));
        assert!(matches!(
            rig.follower.done(),
            Some(Done::Ended { bytes: 0, .. })
        ));
    }

    #[test]
    fn r3_with_the_file_held_waits_the_short_window_too() {
        let mut rig = Rig::new(); // held
        rig.end_owner();
        let t = Instant::now();
        let mut out = Vec::new();
        rig.follower.read_to_end(&mut out).unwrap();
        assert!(t.elapsed() >= QUIET_EMPTY_TEST);
        assert!(matches!(
            rig.follower.done(),
            Some(Done::Ended { bytes: 0, .. })
        ));
    }

    // R4

    #[test]
    fn r4_a_silent_writer_after_the_owner_ended_is_cut_off_after_the_window() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(b"some").unwrap();
        rig.end_owner();
        let t = Instant::now();
        let mut out = Vec::new();
        rig.follower.read_to_end(&mut out).unwrap();
        let waited = t.elapsed();
        assert_eq!(out, b"some");
        assert!(waited >= QUIET_TEST && waited < QUIET_TEST + Duration::from_secs(2));
        match rig.follower.done() {
            Some(Done::Ended { bytes: 4, quiet }) => assert!(*quiet >= QUIET_TEST),
            other => panic!("expected Ended, got {other:?}"),
        }
        drop(w);
    }

    #[test]
    fn r4_bytes_after_the_owner_ended_are_delivered_not_cut_off() {
        // `fully_async`: the client is gone and the server is still flushing.
        let rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(b"head").unwrap();
        rig.end_owner();
        let tail = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            w.write_all(b"tail").unwrap();
            w
        });
        let (got, f) = finished(read_in_background(rig.follower));
        let _w = tail.join().unwrap();
        assert_eq!(got, b"headtail");
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 8, .. })));
    }

    #[test]
    fn r4_every_byte_restarts_the_window() {
        let rig = Rig::new();
        let mut w = rig.writer();
        rig.end_owner();
        let writer = std::thread::spawn(move || {
            // Seven pieces 40ms apart: 280ms of dribble against a 200ms window,
            // so only the restart on each byte keeps it open, with a 5x margin
            // per gap so a starved thread cannot let the window lapse.
            for i in 0..7u8 {
                std::thread::sleep(Duration::from_millis(40));
                w.write_all(&[i]).unwrap();
            }
            w
        });
        let (got, f) = finished(read_in_background(rig.follower));
        let _w = writer.join().unwrap();
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5, 6]);
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 7, .. })));
    }

    // R1

    #[test]
    fn r1_a_removed_path_is_a_broken_channel() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(b"ab").unwrap();
        let mut two = [0u8; 2];
        rig.follower.read_exact(&mut two).unwrap();
        std::fs::remove_file(&rig.path).unwrap();
        let mut out = Vec::new();
        rig.follower.read_to_end(&mut out).unwrap();
        match rig.follower.done() {
            Some(Done::Broken { bytes: 2, error }) => assert_eq!(error.kind(), ErrorKind::NotFound),
            other => panic!("expected Broken, got {other:?}"),
        }
        drop(w);
    }

    #[test]
    fn r1_a_replaced_path_is_a_broken_channel() {
        let mut rig = Rig::new();
        std::fs::remove_file(&rig.path).unwrap();
        // The same name, a different FIFO: the server would write to that one.
        let other = Channel::at(rig.path.clone()).unwrap();
        let mut out = Vec::new();
        rig.follower.read_to_end(&mut out).unwrap();
        assert!(matches!(rig.follower.done(), Some(Done::Broken { .. })));
        drop(other);
    }

    #[test]
    fn r1_a_read_error_is_a_broken_channel() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(b"abc").unwrap();
        let mut three = [0u8; 3];
        rig.follower.read_exact(&mut three).unwrap();

        // Make the follower's descriptor unreadable without closing it:
        // overwrite it with a write-only one.
        let devnull = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap();
        // SAFETY: both descriptors are valid and owned by this test.
        let rc = unsafe { libc::dup2(devnull.as_raw_fd(), rig.follower.file.as_raw_fd()) };
        assert!(rc >= 0);

        let mut out = Vec::new();
        assert_eq!(rig.follower.read_to_end(&mut out).unwrap(), 0);
        match rig.follower.done() {
            Some(Done::Broken { bytes: 3, error }) => {
                assert_eq!(error.raw_os_error(), Some(libc::EBADF))
            }
            other => panic!("expected Broken, got {other:?}"),
        }
        drop(w);
    }

    // After the end

    #[test]
    fn a_finished_follower_keeps_reading_zero() {
        let mut rig = Rig::new();
        rig.release();
        rig.end_owner();
        let mut buf = [0u8; 8];
        assert_eq!(rig.follower.read(&mut buf).unwrap(), 0);
        assert!(rig.follower.done().is_some());
        for _ in 0..3 {
            assert_eq!(rig.follower.read(&mut buf).unwrap(), 0);
        }
    }

    #[test]
    fn take_done_moves_the_reason_out() {
        let mut rig = Rig::new();
        rig.release();
        rig.end_owner();
        let mut out = Vec::new();
        rig.follower.read_to_end(&mut out).unwrap();
        assert!(matches!(rig.follower.take_done(), Some(Done::Ended { .. })));
        assert!(rig.follower.take_done().is_none());
    }

    // The gate is only asked what a rule needs

    #[test]
    fn the_gate_is_not_asked_while_bytes_flow() {
        let mut rig = Rig::new();
        let mut w = rig.writer();
        w.write_all(b"abcdef").unwrap();
        let mut buf = [0u8; 6];
        rig.follower.read_exact(&mut buf).unwrap();
        assert_eq!(rig.ended_calls.load(Ordering::SeqCst), 0);
        assert_eq!(rig.held_calls.load(Ordering::SeqCst), 0);
        drop(w);
    }

    #[test]
    fn a_costly_held_is_not_asked_while_a_writer_is_attached_and_quiet() {
        let rig = Rig::new();
        let w = rig.writer();
        let (ended_calls, held_calls) = (rig.ended_calls.clone(), rig.held_calls.clone());
        let ended = rig.ended.clone();
        let rx = read_in_background(rig.follower);
        assert!(still_waiting(&rx));
        assert!(
            ended_calls.load(Ordering::SeqCst) > 0,
            "the owner is asked each tick"
        );
        assert_eq!(held_calls.load(Ordering::SeqCst), 0, "held is not");
        ended.store(true, Ordering::SeqCst);
        drop(finished(rx));
        drop(w);
    }

    // Channel

    #[test]
    fn a_channel_is_a_fifo_named_for_its_caller() {
        let channel = Channel::fifo("build-event-out").unwrap();
        let name = channel
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(
            name.starts_with("build-event-out-") && name.ends_with(".bin"),
            "{name}"
        );
        assert_eq!(channel.path().parent().unwrap(), std::env::temp_dir());
        assert!(
            std::fs::metadata(channel.path())
                .unwrap()
                .file_type()
                .is_fifo()
        );
    }

    #[test]
    fn two_channels_never_share_a_path() {
        let a = Channel::fifo("x").unwrap();
        let b = Channel::fifo("x").unwrap();
        assert_ne!(a.path(), b.path());
    }

    #[test]
    fn making_a_channel_at_an_existing_fifo_is_fine() {
        let first = Channel::fifo("again").unwrap();
        let path = first.path().to_path_buf();
        let follower = first.follow(Gate::new(|| true)).unwrap();
        // Still there: the follower owns it now.
        let second = Channel::at(path.clone()).unwrap();
        assert_eq!(second.path(), path);
        drop(second); // removes it, as any unfollowed channel does
        drop(follower);
    }

    #[test]
    fn a_channel_in_a_missing_directory_is_an_error() {
        let path = std::env::temp_dir().join("bazel-no-such-dir").join("p.bin");
        assert!(Channel::at(path).is_err());
    }

    #[test]
    fn an_unfollowed_channel_removes_its_fifo() {
        let channel = Channel::fifo("unfollowed").unwrap();
        let path = channel.path().to_path_buf();
        assert!(path.exists());
        drop(channel);
        assert!(!path.exists());
    }

    #[test]
    fn a_follower_owns_the_fifo_and_removes_it_on_drop() {
        let channel = Channel::fifo("owned").unwrap();
        let path = channel.path().to_path_buf();
        let follower = channel.follow(Gate::new(|| false)).unwrap();
        assert!(path.exists(), "following does not remove it");
        assert_eq!(follower.path(), path);
        drop(follower);
        assert!(!path.exists());
    }

    #[test]
    fn following_a_vanished_channel_is_an_error() {
        let channel = Channel::fifo("vanished").unwrap();
        std::fs::remove_file(channel.path()).unwrap();
        assert!(channel.follow(Gate::new(|| false)).is_err());
    }
}
