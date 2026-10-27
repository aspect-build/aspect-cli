//! Following a regular file that a process we launched writes to, such as
//! bazel's compact execution log.
//!
//! Unlike a FIFO it can be read at any moment, and unlike a FIFO it may not
//! exist yet: bazel creates it when it gets to it, and the usual shape is to
//! create it empty, hold it open for the whole build, write it in one go at
//! the end and close it. So the follower opens it lazily and treats "not
//! there yet" like "nothing written yet". The rules that decide when the
//! stream is over are the [parent module](super)'s.
//!
//! A file left behind by an earlier run has to be removed *before* bazel is
//! spawned: the follower cannot tell it from this build's file, and would read
//! the previous build's contents.

use std::fs;
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};

use super::{Done, Gate, Rules, TICK};

/// What one attempt to read found.
enum Step {
    Bytes(usize),
    /// Nothing: the file is not there yet, or this is its current end.
    Nothing,
    Broken(io::Error),
}

/// A file being written, read as a stream. See the [module docs](self).
pub struct File {
    path: PathBuf,
    /// `None` until the file exists and has been opened.
    file: Option<fs::File>,
    rules: Rules,
    done: Option<Done>,
}

impl File {
    /// Start following `path`. Touches nothing on disk: the file may be
    /// created at any later moment, or never.
    pub fn follow(path: PathBuf, gate: Gate) -> Self {
        Self {
            path,
            file: None,
            rules: Rules::new(gate),
            done: None,
        }
    }

    /// Why the stream ended. `None` until a read has returned `Ok(0)`.
    pub fn done(&self) -> Option<&Done> {
        self.done.as_ref()
    }

    /// [`File::done`], moved out.
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

    /// Whether the file has been seen to exist.
    pub fn appeared(&self) -> bool {
        self.file.is_some()
    }

    /// Wait until the file exists. `false` means the stream ended first
    /// without it ever appearing (see [`File::done`]), which is ordinary when
    /// bazel failed before it got that far.
    pub fn appear(&mut self) -> bool {
        loop {
            if self.file.is_some() {
                return true;
            }
            if self.done.is_some() {
                return false;
            }
            match self.open() {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => {
                    self.done = Some(self.rules.broken(error));
                    return false;
                }
            }
            if let Some(done) = self.rules.judge(true) {
                // The file can have been created since the look above, and by a
                // writer that has since finished and exited.
                match self.open() {
                    Ok(true) => return true,
                    Ok(false) => {
                        self.done = Some(done);
                        return false;
                    }
                    Err(error) => {
                        self.done = Some(self.rules.broken(error));
                        return false;
                    }
                }
            }
            std::thread::sleep(TICK);
        }
    }

    /// Open the file if it is there: `Ok(false)` when it is not.
    fn open(&mut self) -> io::Result<bool> {
        if self.file.is_some() {
            return Ok(true);
        }
        match fs::File::open(&self.path) {
            Ok(file) => {
                self.file = Some(file);
                Ok(true)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn step(&mut self, buf: &mut [u8]) -> Step {
        match self.open() {
            Ok(true) => {}
            Ok(false) => return Step::Nothing,
            Err(e) => return Step::Broken(e),
        }
        let file = self.file.as_mut().expect("opened above");
        loop {
            match file.read(buf) {
                Ok(0) => return Step::Nothing,
                Ok(n) => {
                    self.rules.arrived(n);
                    return Step::Bytes(n);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Step::Broken(e),
            }
        }
    }
}

impl Read for File {
    /// Bytes, or `Ok(0)` once the stream is over (see [`File::done`]). Blocks,
    /// in slices, until one of the two.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // `Read` requires an empty buffer to return immediately.
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.done.is_some() {
                return Ok(0);
            }
            match self.step(buf) {
                Step::Bytes(n) => return Ok(n),
                Step::Broken(error) => {
                    self.done = Some(self.rules.broken(error));
                    return Ok(0);
                }
                Step::Nothing => {}
            }
            if let Some(done) = self.rules.judge(true) {
                // A verdict is not final until one more look: bytes written
                // between the read above and the writer letting go are ours,
                // and so is a file that appeared in that window. Bazel's
                // execution log is exactly this shape, empty for the whole
                // build and written in one go at the end. Once the writer has
                // let go nothing more can arrive, so one more look sees it all.
                match self.step(buf) {
                    Step::Bytes(n) => return Ok(n),
                    Step::Broken(error) => {
                        self.done = Some(self.rules.broken(error));
                        return Ok(0);
                    }
                    Step::Nothing => {
                        self.done = Some(done);
                        return Ok(0);
                    }
                }
            }
            std::thread::sleep(TICK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    const QUIET_TEST: Duration = Duration::from_millis(200);
    const QUIET_EMPTY_TEST: Duration = Duration::from_millis(100);

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!("bazel-file-test-{}.log", uuid::Uuid::new_v4()))
    }

    /// A file follower whose owner and writers are flags the test flips.
    struct Rig {
        path: PathBuf,
        ended: Arc<AtomicBool>,
        held: Arc<AtomicBool>,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                path: temp_path(),
                ended: Arc::new(AtomicBool::new(false)),
                held: Arc::new(AtomicBool::new(true)),
            }
        }

        fn gate(&self) -> Gate {
            let (e, h) = (self.ended.clone(), self.held.clone());
            let mut gate = Gate::new(move || e.load(Ordering::SeqCst))
                .with_held(move || h.load(Ordering::SeqCst));
            gate.quiet = QUIET_TEST;
            gate.quiet_empty = QUIET_EMPTY_TEST;
            gate
        }

        fn follow(&self) -> File {
            File::follow(self.path.clone(), self.gate())
        }

        fn end_owner(&self) {
            self.ended.store(true, Ordering::SeqCst);
        }

        fn release(&self) {
            self.held.store(false, Ordering::SeqCst);
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn read_in_background(mut f: File) -> mpsc::Receiver<(Vec<u8>, File)> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            f.read_to_end(&mut out).unwrap();
            let _ = tx.send((out, f));
        });
        rx
    }

    fn still_waiting(rx: &mpsc::Receiver<(Vec<u8>, File)>) -> bool {
        matches!(
            rx.recv_timeout(Duration::from_millis(400)),
            Err(mpsc::RecvTimeoutError::Timeout)
        )
    }

    fn finished(rx: mpsc::Receiver<(Vec<u8>, File)>) -> (Vec<u8>, File) {
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the read should have ended")
    }

    #[test]
    fn following_touches_nothing_on_disk() {
        let rig = Rig::new();
        let f = rig.follow();
        assert!(!rig.path.exists());
        assert!(!f.appeared());
        assert!(f.done().is_none());
        assert_eq!(f.path(), rig.path);
        assert_eq!(f.bytes_read(), 0);
    }

    #[test]
    fn a_finished_file_reads_whole_then_drains() {
        let rig = Rig::new();
        fs::write(&rig.path, b"the whole log").unwrap();
        rig.release();
        let (got, f) = finished(read_in_background(rig.follow()));
        assert_eq!(got, b"the whole log");
        assert!(f.appeared());
        assert!(matches!(f.done(), Some(Done::Drained { bytes: 13 })));
    }

    #[test]
    fn a_file_written_before_the_owner_exited_is_still_read() {
        // The writer finished and the client is already gone by the time we
        // look: the bytes are there and must not be hidden by that.
        let rig = Rig::new();
        fs::write(&rig.path, b"payload").unwrap();
        rig.end_owner();
        rig.release();
        let (got, f) = finished(read_in_background(rig.follow()));
        assert_eq!(got, b"payload");
        assert!(matches!(f.done(), Some(Done::Drained { bytes: 7 })));
    }

    #[test]
    fn a_growing_file_is_followed_to_the_end() {
        let rig = Rig::new();
        let mut w = fs::File::create(&rig.path).unwrap();
        let held = rig.held.clone();
        let rx = read_in_background(rig.follow());
        for piece in [&b"one "[..], b"two ", b"three"] {
            std::thread::sleep(Duration::from_millis(50));
            w.write_all(piece).unwrap();
        }
        assert!(still_waiting(&rx), "the writer still holds the file");
        held.store(false, Ordering::SeqCst);
        let (got, f) = finished(rx);
        assert_eq!(got, b"one two three");
        assert!(matches!(f.done(), Some(Done::Drained { bytes: 13 })));
    }

    #[test]
    fn a_file_created_late_is_waited_for_and_read() {
        let rig = Rig::new();
        let path = rig.path.clone();
        let held = rig.held.clone();
        let rx = read_in_background(rig.follow());
        assert!(still_waiting(&rx), "no file yet and the owner is alive");
        fs::write(&path, b"late").unwrap();
        held.store(false, Ordering::SeqCst);
        let (got, f) = finished(rx);
        assert_eq!(got, b"late");
        assert!(f.appeared());
    }

    #[test]
    fn a_log_empty_for_the_whole_build_and_written_at_the_end_is_not_lost() {
        // Created empty, held open, written in one go, closed.
        let rig = Rig::new();
        let w = fs::File::create(&rig.path).unwrap();
        let (held, ended) = (rig.held.clone(), rig.ended.clone());
        let rx = read_in_background(rig.follow());
        assert!(still_waiting(&rx), "empty and held is not the end");
        {
            let mut w = w;
            w.write_all(b"written at the very end").unwrap();
        }
        ended.store(true, Ordering::SeqCst);
        held.store(false, Ordering::SeqCst);
        let (got, _) = finished(rx);
        assert_eq!(got, b"written at the very end");
    }

    #[test]
    fn bytes_written_just_before_the_writer_lets_go_are_not_lost() {
        // The window is as wide as one fd-table walk, so the write is made
        // inside the `held` question rather than raced for: the answer is
        // "no longer held" and, in the same instant, the log is on disk.
        let rig = Rig::new();
        fs::File::create(&rig.path).unwrap();
        let path = rig.path.clone();
        let mut gate = Gate::new(|| true).with_held(move || {
            fs::write(&path, b"payload").unwrap();
            false
        });
        gate.quiet = QUIET_TEST;
        gate.quiet_empty = QUIET_EMPTY_TEST;
        let mut f = File::follow(rig.path.clone(), gate);
        let mut got = Vec::new();
        f.read_to_end(&mut got).unwrap();
        assert_eq!(
            got, b"payload",
            "the second look sees what landed in the window"
        );
    }

    #[test]
    fn a_file_created_in_the_window_before_the_verdict_is_not_lost() {
        let rig = Rig::new();
        let path = rig.path.clone();
        let mut gate = Gate::new(|| true).with_held(move || {
            // The writer creates, fills and closes the file just as we are
            // told it is gone.
            fs::write(&path, b"whole").unwrap();
            false
        });
        gate.quiet = QUIET_TEST;
        gate.quiet_empty = QUIET_EMPTY_TEST;
        let mut f = File::follow(rig.path.clone(), gate);
        let mut got = Vec::new();
        f.read_to_end(&mut got).unwrap();
        assert_eq!(got, b"whole");
        assert!(f.appeared());
    }

    // appear()

    #[test]
    fn appear_returns_true_when_the_file_shows_up() {
        let rig = Rig::new();
        let path = rig.path.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            fs::write(&path, b"x").unwrap();
        });
        let mut f = rig.follow();
        assert!(f.appear());
        assert!(f.appeared());
        writer.join().unwrap();
        assert!(f.appear(), "asking again is fine");
    }

    #[test]
    fn appear_returns_false_when_the_owner_ends_with_no_file() {
        let rig = Rig::new();
        rig.end_owner();
        rig.release();
        let mut f = rig.follow();
        let t = Instant::now();
        assert!(!f.appear());
        assert!(t.elapsed() < QUIET_EMPTY_TEST + Duration::from_secs(2));
        assert!(!f.appeared());
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 0, .. })));
        assert!(!f.appear(), "and it stays false");
    }

    #[test]
    fn appear_waits_while_the_owner_lives() {
        let rig = Rig::new();
        let ended = rig.ended.clone();
        let (tx, rx) = mpsc::channel();
        let mut f = rig.follow();
        std::thread::spawn(move || {
            let _ = tx.send(f.appear());
        });
        assert!(
            matches!(
                rx.recv_timeout(Duration::from_millis(400)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "no file and a living owner is not an answer"
        );
        ended.store(true, Ordering::SeqCst);
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(false));
    }

    #[test]
    fn appear_does_not_miss_a_file_created_in_the_verdict_window() {
        let rig = Rig::new();
        let path = rig.path.clone();
        let mut gate = Gate::new(|| true).with_held(move || {
            fs::write(&path, b"whole").unwrap();
            false
        });
        gate.quiet_empty = QUIET_EMPTY_TEST;
        let mut f = File::follow(rig.path.clone(), gate);
        assert!(f.appear());
    }

    #[test]
    fn appear_reports_a_path_that_cannot_be_opened() {
        let rig = Rig::new();
        let blocker = temp_path();
        fs::write(&blocker, b"i am a file").unwrap();
        // A file where a directory is needed: ENOTDIR, not "not found".
        let mut f = File::follow(blocker.join("inside.log"), rig.gate());
        assert!(!f.appear());
        assert!(matches!(f.done(), Some(Done::Broken { .. })));
        let _ = fs::remove_file(&blocker);
    }

    // The other rules, as a file sees them

    #[test]
    fn an_empty_finished_file_ends_empty_once_the_owner_is_gone() {
        let rig = Rig::new();
        fs::File::create(&rig.path).unwrap();
        rig.end_owner();
        rig.release();
        let (got, f) = finished(read_in_background(rig.follow()));
        assert!(got.is_empty());
        assert!(f.appeared());
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 0, .. })));
    }

    #[test]
    fn a_writer_that_never_closes_is_cut_off_after_the_window() {
        let rig = Rig::new();
        fs::write(&rig.path, b"some").unwrap();
        rig.end_owner(); // held stays true: the server keeps the file open
        let t = Instant::now();
        let (got, f) = finished(read_in_background(rig.follow()));
        assert_eq!(got, b"some");
        assert!(t.elapsed() >= QUIET_TEST);
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 4, .. })));
    }

    #[test]
    fn bytes_after_the_owner_ended_restart_the_window() {
        let rig = Rig::new();
        let mut w = fs::File::create(&rig.path).unwrap();
        rig.end_owner();
        let writer = std::thread::spawn(move || {
            // 40ms apart against a 200ms window: a 5x margin per gap.
            for i in 0..6u8 {
                std::thread::sleep(Duration::from_millis(40));
                w.write_all(&[i]).unwrap();
            }
            w
        });
        let (got, f) = finished(read_in_background(rig.follow()));
        let _w = writer.join().unwrap();
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5]);
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 6, .. })));
    }

    // Failures

    #[test]
    fn a_read_error_is_a_broken_stream() {
        // A directory opens fine and fails to read.
        let rig = Rig::new();
        fs::create_dir(&rig.path).unwrap();
        let mut f = rig.follow();
        let mut out = Vec::new();
        assert_eq!(f.read_to_end(&mut out).unwrap(), 0);
        match f.done() {
            Some(Done::Broken { bytes: 0, error }) => {
                assert_eq!(error.raw_os_error(), Some(libc::EISDIR))
            }
            other => panic!("expected Broken, got {other:?}"),
        }
        let _ = fs::remove_dir(&rig.path);
    }

    #[test]
    fn an_unopenable_path_is_a_broken_stream() {
        let rig = Rig::new();
        let blocker = temp_path();
        fs::write(&blocker, b"file").unwrap();
        let mut f = File::follow(blocker.join("inside.log"), rig.gate());
        let mut out = Vec::new();
        f.read_to_end(&mut out).unwrap();
        assert!(matches!(f.done(), Some(Done::Broken { .. })));
        let _ = fs::remove_file(&blocker);
    }

    // After the end

    #[test]
    fn an_empty_buffer_reads_zero_without_ending_anything() {
        let rig = Rig::new();
        let mut f = rig.follow();
        assert_eq!(f.read(&mut []).unwrap(), 0);
        assert!(f.done().is_none());
    }

    #[test]
    fn a_finished_file_keeps_reading_zero_and_gives_its_reason_once() {
        let rig = Rig::new();
        rig.end_owner();
        rig.release();
        let mut f = rig.follow();
        let mut buf = [0u8; 4];
        assert_eq!(f.read(&mut buf).unwrap(), 0);
        for _ in 0..3 {
            assert_eq!(f.read(&mut buf).unwrap(), 0);
        }
        assert!(matches!(f.take_done(), Some(Done::Ended { .. })));
        assert!(f.take_done().is_none());
    }
}
