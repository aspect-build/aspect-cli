//! Following a regular file that a process we launched writes to: bazel's
//! compact execution log, its build event file, and its workspace log.
//!
//! It can be read at any moment, and it may not exist yet: bazel creates it
//! when it gets to it. So the follower opens it lazily and treats "not there
//! yet" like "nothing written yet". The rules that decide when the stream is
//! over are the [parent module](super)'s.
//!
//! The follower cannot tell a file an earlier run left at its path from this
//! build's, and would read the previous build's contents, so every path it
//! follows is one nothing has used: a [`Reservation`], which removes it once
//! the read is over, a fresh folder of the task invocation's history, or a
//! fresh uuid-named temp file.
//!
//! # A writer that starts the file over
//!
//! A build bazel retries inside one invocation reopens the build event file
//! with `O_TRUNC` for each attempt. Truncating the file being read would lose
//! whatever of the last attempt the follower has not read yet.
//!
//! So a follower that owns its path (a [`Reservation`]) first moves each file
//! that appears there to a name of its own, `<path>.<n>`, and reads it there.
//! The writer's fd follows the file, so a reopen of the path creates a new
//! one. The follower moves each new file away too, as it reads, at most a
//! [`TICK`] after it appears, so a writer that starts over again before the
//! follower has got to it cannot truncate it either. Files are read in the
//! order they appeared, each to its end. While the writer still holds the
//! current one it may not be done with it, so moving on waits for it to let
//! go, for at most [`Gate::quiet`] without a byte; no verdict on the stream is
//! reached while another file is queued. `held` is asked about the name the
//! current file has.
//!
//! A writer that starts over before the follower has opened the file at all
//! still truncates it, which for bazel would take a first attempt shorter
//! than the follower's start.
//!
//! A path the follower does not own stays where it is. A file there that is
//! shorter than what was read from it was truncated in place, and is read
//! again from the start. What was not read before the truncation is lost.
//!
//! Either way, starting over is reported once, as a [`StartedOver`] read
//! error, so that a reader of records can drop one the old file left half
//! written rather than finish it with the new file's bytes.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{Done, Expect, Gate, Rules, TICK};

/// A path a process we launch will write a file to, reserved before it is
/// spawned: the path goes on the command line, then [`Reservation::follow`]
/// starts reading. Nothing is created on disk.
///
/// The path is removed when the reservation, or the [`File`] that follows it,
/// is dropped, along with the names the follower moved files to, so nothing
/// outlives the read.
pub struct Reservation {
    path: PathBuf,
}

impl Reservation {
    /// A new path in the temp dir named `<name>-<uuid>.bin`.
    pub fn temp(name: &str) -> Self {
        Self {
            path: std::env::temp_dir().join(format!("{name}-{}.bin", uuid::Uuid::new_v4())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Start following the path. The returned [`File`] owns it from here on.
    pub fn follow(mut self, gate: Gate) -> File {
        File::new(std::mem::take(&mut self.path), gate, true)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The error [`File`]'s `read` returns, once, where it moves on to a file the
/// writer started over in: the bytes before and after it come from different
/// files. Reading on continues in the new one.
#[derive(Debug)]
pub struct StartedOver;

impl StartedOver {
    pub fn is(error: &io::Error) -> bool {
        error.get_ref().is_some_and(|e| e.is::<StartedOver>())
    }
}

impl std::fmt::Display for StartedOver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the writer started the file over")
    }
}

impl std::error::Error for StartedOver {}

/// Where reaching the end of the current file leads.
enum Next {
    /// Nowhere yet.
    End,
    /// The current file has more to give.
    Again,
    /// Reading starts over, in the next file or at the start of this one.
    StartedOver,
}

/// What one attempt to read found.
enum Step {
    Bytes(usize),
    /// Nothing: the file is not there yet, this is its current end, or the
    /// next file waits for the writer to let go of this one.
    Nothing,
    /// See [`StartedOver`].
    StartedOver,
    Broken(io::Error),
}

/// A file being read, and the name it has.
struct Opened {
    file: fs::File,
    path: PathBuf,
}

/// A file being written, read as a stream. See the [module docs](self).
pub struct File {
    /// Where the writer opens the file.
    path: PathBuf,
    /// `None` until the file exists and has been opened.
    current: Option<Opened>,
    /// Bytes read from `current`, to notice it being truncated.
    offset: u64,
    /// Files the writer started over in, oldest first, each read once the one
    /// before it is read to its end and let go of.
    next: VecDeque<Opened>,
    /// When the follower last looked for a new file at `path`.
    looked: Option<Instant>,
    /// When reading reached the end of `current` with a file queued after it;
    /// any byte resets it.
    switch_since: Option<Instant>,
    /// The writer let go of `current` (or the wait ran out), and one more read
    /// of it is under way before moving on.
    let_go: bool,
    /// Remove `path` on drop, and move each file that appears there to a name
    /// of its own: it came from a [`Reservation`].
    owned: bool,
    /// Files moved away from `path` so far, named `<path>.1` onwards.
    moved: u32,
    /// See [`File::expect_replacement`].
    awaiting: bool,
    rules: Rules,
    done: Option<Done>,
}

impl File {
    /// Start following `path`, which belongs to the caller. Touches nothing
    /// on disk: the file may be created at any later moment, or never.
    pub fn follow(path: PathBuf, gate: Gate) -> Self {
        Self::new(path, gate, false)
    }

    fn new(path: PathBuf, gate: Gate, owned: bool) -> Self {
        Self {
            path,
            current: None,
            offset: 0,
            next: VecDeque::new(),
            looked: None,
            switch_since: None,
            let_go: false,
            owned,
            moved: 0,
            awaiting: false,
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

    /// Bytes delivered so far, across every file a writer started over in.
    /// (Not `bytes`, which `Read::bytes` would win over at every call.)
    pub fn bytes_read(&self) -> u64 {
        self.rules.bytes()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file last read, from its start, for going over it once more after
    /// the stream is over: the writer's file, even if its path has since been
    /// reused. Reading it moves the stream's offset too, so the stream must not
    /// be read once it is used. Fails if no file was ever opened.
    pub fn rewound(&self) -> io::Result<fs::File> {
        let current = self
            .current
            .as_ref()
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "the file never appeared"))?;
        let mut file = current.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    }

    /// Whether the file has been seen to exist.
    pub fn appeared(&self) -> bool {
        self.current.is_some()
    }

    /// What the protocol says about the writer, such as a build bazel is
    /// about to retry ([`Expect::More`]). See the [parent module](super).
    pub fn expect(&mut self, what: Expect) {
        self.rules.expect(what);
    }

    /// The writer will start over in a new file once it is done with the
    /// current one, as bazel does for each attempt of a build it retries.
    /// Moving to that file needs nothing from this (see the
    /// [module docs](self)); it only arms [`File::awaiting_replacement`].
    pub fn expect_replacement(&mut self) {
        self.awaiting = true;
    }

    /// A replacement was expected and the writer has not started over yet:
    /// everything read since came from the file it was to be done with.
    pub fn awaiting_replacement(&self) -> bool {
        self.awaiting
    }

    /// Wait until the file exists. `false` means the stream ended first
    /// without it ever appearing (see [`File::done`]), which is ordinary when
    /// bazel failed before it got that far.
    pub fn appear(&mut self) -> bool {
        loop {
            if self.current.is_some() {
                return true;
            }
            if self.done.is_some() {
                return false;
            }
            match self.open() {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => return self.broke(error),
            }
            if let Some(done) = self.judge() {
                // The file can have been created since the look above, and by a
                // writer that has since finished and exited.
                match self.open() {
                    Ok(true) => return true,
                    Ok(false) => {
                        self.done = Some(done);
                        return false;
                    }
                    Err(error) => return self.broke(error),
                }
            }
            std::thread::sleep(TICK);
        }
    }

    /// R1: end the stream on `error`. `false`, for [`File::appear`].
    fn broke(&mut self, error: io::Error) -> bool {
        self.done = Some(self.rules.broken(error));
        false
    }

    /// R2 to R4 for a read that found nothing, with `held` asked about the
    /// name the file being read has now.
    fn judge(&mut self) -> Option<Done> {
        if !self.next.is_empty() {
            // The stream goes on in `next`.
            return None;
        }
        let reading = self.current.as_ref().map_or(&self.path, |c| &c.path);
        self.rules.judge(true, reading)
    }

    /// Open the file if it is there: `Ok(false)` when it is not.
    fn open(&mut self) -> io::Result<bool> {
        if self.current.is_none() {
            self.current = self.claim()?;
        }
        Ok(self.current.is_some())
    }

    /// The file at `path`, if there is one. An owned path's file is moved to
    /// a name of its own first.
    fn claim(&mut self) -> io::Result<Option<Opened>> {
        if !self.owned {
            return Ok(open_if_there(&self.path)?.map(|file| Opened {
                file,
                path: self.path.clone(),
            }));
        }
        let to = moved_name(&self.path, self.moved + 1);
        match fs::rename(&self.path, &to) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
        self.moved += 1;
        let file = fs::File::open(&to)?;
        Ok(Some(Opened { file, path: to }))
    }

    /// Move a file that appeared at an owned `path` into the queue, at most
    /// once a [`TICK`].
    fn look_for_next(&mut self) -> io::Result<()> {
        if !self.owned || self.looked.is_some_and(|t| t.elapsed() < TICK) {
            return Ok(());
        }
        self.looked = Some(Instant::now());
        if let Some(next) = self.claim()? {
            self.next.push_back(next);
        }
        Ok(())
    }

    fn step(&mut self, buf: &mut [u8]) -> Step {
        match self.open() {
            Ok(true) => {}
            Ok(false) => return Step::Nothing,
            Err(e) => return Step::Broken(e),
        }
        if let Err(e) = self.look_for_next() {
            return Step::Broken(e);
        }
        loop {
            match self.read_current(buf) {
                Ok(0) => {}
                Ok(n) => return Step::Bytes(n),
                Err(e) => return Step::Broken(e),
            }
            match self.next_file() {
                Ok(Next::Again) => {}
                Ok(Next::StartedOver) => return Step::StartedOver,
                Ok(Next::End) => return Step::Nothing,
                Err(e) => return Step::Broken(e),
            }
        }
    }

    fn read_current(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let current = self.current.as_mut().expect("opened before reading");
        loop {
            match current.file.read(buf) {
                Ok(0) => return Ok(0),
                Ok(n) => {
                    self.offset += n as u64;
                    self.rules.arrived(n);
                    // Still being written: the wait to move on starts over.
                    self.switch_since = None;
                    self.let_go = false;
                    return Ok(n);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// At the end of the current file, move to where the writer has started
    /// over, if it has (see the [module docs](self)).
    fn next_file(&mut self) -> io::Result<Next> {
        if !self.owned {
            let current = self.current.as_mut().expect("opened before reading");
            if current.file.metadata()?.len() >= self.offset {
                return Ok(Next::End);
            }
            current.file.seek(SeekFrom::Start(0))?;
            self.started_over();
            return Ok(Next::StartedOver);
        }
        if self.next.is_empty() {
            // The end just read came before this look: one more read of the
            // old file, once its writer has let go, sees the rest of it.
            self.looked = Some(Instant::now());
            return Ok(match self.claim()? {
                Some(next) => {
                    self.next.push_back(next);
                    Next::Again
                }
                None => Next::End,
            });
        }
        if !self.let_go {
            let since = *self.switch_since.get_or_insert_with(Instant::now);
            let current = self.current.as_ref().expect("opened before reading");
            if since.elapsed() < self.rules.quiet() && self.rules.held(&current.path) == Some(true)
            {
                return Ok(Next::End);
            }
            // What the writer wrote before letting go is read first.
            self.let_go = true;
            return Ok(Next::Again);
        }
        let next = self.next.pop_front().expect("checked above");
        if let Some(done_with) = self.current.replace(next) {
            let _ = fs::remove_file(&done_with.path);
        }
        self.switch_since = None;
        self.let_go = false;
        self.started_over();
        Ok(Next::StartedOver)
    }

    /// What [`Read::read`] returns for `step`, or `None` when it found
    /// nothing.
    fn reply(&mut self, step: Step) -> Option<io::Result<usize>> {
        match step {
            Step::Bytes(n) => Some(Ok(n)),
            Step::StartedOver => Some(Err(io::Error::other(StartedOver))),
            Step::Broken(error) => {
                self.broke(error);
                Some(Ok(0))
            }
            Step::Nothing => None,
        }
    }

    fn started_over(&mut self) {
        self.offset = 0;
        self.awaiting = false;
    }
}

/// Where an owned path's `n`th file is moved to: `<path>.<n>`.
pub(crate) fn moved_name(path: &Path, n: u32) -> PathBuf {
    let mut name = OsString::from(path);
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

fn open_if_there(path: &Path) -> io::Result<Option<fs::File>> {
    match fs::File::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if self.owned {
            let _ = fs::remove_file(&self.path);
            for n in 1..=self.moved {
                let _ = fs::remove_file(moved_name(&self.path, n));
            }
        }
    }
}

impl Read for File {
    /// Bytes, or `Ok(0)` once the stream is over (see [`File::done`]). Blocks,
    /// in slices, until one of the two. Fails with [`StartedOver`], once each
    /// time, where reading starts over.
    ///
    /// A verdict is not final until one more look: bytes written between the
    /// read and the writer letting go are ours, and so is a file that appeared
    /// in that window. Bazel's execution log is exactly this shape, empty for
    /// the whole build and written in one go at the end. Once the writer has
    /// let go nothing more can arrive, so one more look sees it all. A look
    /// that finds a new file queued drops the verdict.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // `Read` requires an empty buffer to return immediately.
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.done.is_some() {
                return Ok(0);
            }
            let step = self.step(buf);
            if let Some(reply) = self.reply(step) {
                return reply;
            }
            if let Some(done) = self.judge() {
                let step = self.step(buf);
                if let Some(reply) = self.reply(step) {
                    return reply;
                }
                if self.next.is_empty() {
                    self.done = Some(done);
                    return Ok(0);
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
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use crate::stream::testing::{
        Flags, QUIET_EMPTY_TEST, QUIET_TEST, finished, still_waiting, test_windows,
    };

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!("bazel-file-test-{}.log", uuid::Uuid::new_v4()))
    }

    /// A file follower whose owner and writers are flags the test flips.
    struct Rig {
        path: PathBuf,
        flags: Flags,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                path: temp_path(),
                flags: Flags::new(),
            }
        }

        fn gate(&self) -> Gate {
            self.flags.gate(true)
        }

        fn follow(&self) -> File {
            File::follow(self.path.clone(), self.gate())
        }

        /// A follower that owns the path, as a [`Reservation`]'s does.
        fn own(&self) -> File {
            File::new(self.path.clone(), self.gate(), true)
        }

        /// [`Rig::own`] with a quiet window no stalled test thread outlasts,
        /// for tests where the window must not run out first.
        fn own_with_quiet(&self, quiet: Duration) -> File {
            let mut gate = self.gate();
            gate.quiet = quiet;
            File::new(self.path.clone(), gate, true)
        }

        fn end_owner(&self) {
            self.flags.end();
        }

        fn release(&self) {
            self.flags.release();
        }

        /// The client exits and the server lets go: the ordinary end.
        fn finish(&self) {
            self.end_owner();
            self.release();
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
            loop {
                match f.read_to_end(&mut out) {
                    Ok(_) => break,
                    Err(e) if StartedOver::is(&e) => {}
                    Err(e) => panic!("{e}"),
                }
            }
            let _ = tx.send((out, f));
        });
        rx
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
        rig.finish();
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
        let held = rig.flags.held.clone();
        let rx = read_in_background(rig.follow());
        for piece in [&b"one "[..], b"two ", b"three"] {
            std::thread::sleep(Duration::from_millis(50));
            w.write_all(piece).unwrap();
        }
        assert!(still_waiting(&rx), "the writer still holds the file");
        held.store(false, Ordering::SeqCst);
        rig.end_owner();
        let (got, f) = finished(rx);
        assert_eq!(got, b"one two three");
        assert!(matches!(f.done(), Some(Done::Drained { bytes: 13 })));
    }

    #[test]
    fn a_file_created_late_is_waited_for_and_read() {
        let rig = Rig::new();
        let path = rig.path.clone();
        let held = rig.flags.held.clone();
        let rx = read_in_background(rig.follow());
        assert!(still_waiting(&rx), "no file yet and the owner is alive");
        fs::write(&path, b"late").unwrap();
        held.store(false, Ordering::SeqCst);
        rig.end_owner();
        let (got, f) = finished(rx);
        assert_eq!(got, b"late");
        assert!(f.appeared());
    }

    #[test]
    fn a_log_empty_for_the_whole_build_and_written_at_the_end_is_not_lost() {
        // Created empty, held open, written in one go, closed.
        let rig = Rig::new();
        let w = fs::File::create(&rig.path).unwrap();
        let (held, ended) = (rig.flags.held.clone(), rig.flags.ended.clone());
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
        let mut gate = Gate::new(|| true).with_held(move |_| {
            fs::write(&path, b"payload").unwrap();
            false
        });
        test_windows(&mut gate);
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
        let mut gate = Gate::new(|| true).with_held(move |_| {
            // The writer creates, fills and closes the file just as we are
            // told it is gone.
            fs::write(&path, b"whole").unwrap();
            false
        });
        test_windows(&mut gate);
        let mut f = File::follow(rig.path.clone(), gate);
        let mut got = Vec::new();
        f.read_to_end(&mut got).unwrap();
        assert_eq!(got, b"whole");
        assert!(f.appeared());
    }

    #[test]
    fn a_file_nobody_holds_is_not_over_while_the_owner_lives() {
        // What a stale server pid looks like: it never holds the file. The
        // pauses between writes must not end the stream while the client runs.
        let rig = Rig::new();
        let mut w = fs::File::create(&rig.path).unwrap();
        rig.release();
        let rx = read_in_background(rig.follow());
        w.write_all(b"first").unwrap();
        assert!(still_waiting(&rx), "a pause is not the end");
        w.write_all(b" second").unwrap();
        rig.end_owner();
        let (got, f) = finished(rx);
        assert_eq!(got, b"first second");
        assert!(matches!(f.done(), Some(Done::Drained { bytes: 12 })));
    }

    #[test]
    fn expecting_more_does_not_outlast_the_owner() {
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        let mut f = rig.follow();
        f.expect(Expect::More);
        rig.finish();
        let t = Instant::now();
        let (got, f) = finished(read_in_background(f));
        assert_eq!(got, b"one");
        assert!(t.elapsed() >= QUIET_TEST, "R4's window applied, not R2");
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 3, .. })));
    }

    #[test]
    fn without_a_held_hint_a_file_ends_only_on_the_window() {
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        let gate = rig.flags.gate(false);
        rig.end_owner();
        let t = Instant::now();
        let (got, f) = finished(read_in_background(File::follow(rig.path.clone(), gate)));
        assert_eq!(got, b"one");
        assert!(t.elapsed() >= QUIET_TEST);
        assert!(matches!(f.done(), Some(Done::Ended { bytes: 3, .. })));
    }

    // Reservations

    #[test]
    fn a_reservation_touches_nothing_and_cleans_up_what_was_written() {
        let r = Reservation::temp("bazel-file-test");
        let path = r.path().to_path_buf();
        assert!(!path.exists());
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("bazel-file-test-")
        );
        fs::write(&path, b"x").unwrap();
        drop(r);
        assert!(!path.exists(), "an unfollowed reservation removes the file");
    }

    #[test]
    fn a_followed_reservation_removes_the_file_when_the_follower_goes() {
        let rig = Rig::new();
        let r = Reservation::temp("bazel-file-test");
        let path = r.path().to_path_buf();
        fs::write(&path, b"payload").unwrap();
        rig.finish();
        let (got, f) = finished(read_in_background(r.follow(rig.gate())));
        assert_eq!(got, b"payload");
        let moved = moved_name(&path, 1);
        assert!(moved.exists(), "still there while the follower is");
        drop(f);
        assert!(!moved.exists());
    }

    #[test]
    fn a_follower_that_does_not_own_its_path_leaves_it() {
        let rig = Rig::new();
        fs::write(&rig.path, b"x").unwrap();
        drop(rig.follow());
        assert!(rig.path.exists());
    }

    // A writer that starts over

    /// Read whatever is there now, without waiting for more.
    fn read_now(f: &mut File, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        let mut got = 0;
        while got < n {
            match f.read(&mut buf[got..]) {
                Ok(0) => panic!("ended after {got} of {n} bytes"),
                Ok(k) => got += k,
                Err(e) if StartedOver::is(&e) => {}
                Err(e) => panic!("{e}"),
            }
        }
        buf
    }

    #[test]
    fn moving_on_to_the_next_file_is_reported_once() {
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        let mut f = rig.own();
        rig.release();
        assert_eq!(read_now(&mut f, 3), b"one");
        fs::write(&rig.path, b"two").unwrap();
        let mut buf = [0u8; 8];
        let err = match f.read(&mut buf) {
            Err(e) => e,
            Ok(n) => panic!("read {n} bytes before the boundary"),
        };
        assert!(StartedOver::is(&err), "{err}");
        assert_eq!(f.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf[..3], b"two");
    }

    #[test]
    fn an_owned_file_is_read_under_a_name_of_its_own() {
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        let mut f = rig.own();
        assert_eq!(read_now(&mut f, 3), b"one");
        assert!(!rig.path.exists(), "the path is free for a reopen");
        assert!(moved_name(&rig.path, 1).exists());
    }

    #[test]
    fn a_replacement_is_read_after_the_rest_of_the_old_file() {
        // Bazel's retry: attempt 1 is still writing its tail, then the next
        // attempt reopens the path with O_TRUNC.
        let rig = Rig::new();
        let mut old = fs::File::create(&rig.path).unwrap();
        old.write_all(b"one").unwrap();
        let mut f = rig.own();
        assert_eq!(read_now(&mut f, 3), b"one");
        old.write_all(b"-tail").unwrap();
        drop(old);
        fs::File::create(&rig.path)
            .unwrap()
            .write_all(b"two")
            .unwrap();
        rig.finish();
        let (got, f) = finished(read_in_background(f));
        assert_eq!(got, b"-tailtwo");
        assert_eq!(f.bytes_read(), 11);
    }

    #[test]
    fn a_writer_that_starts_over_before_the_old_file_is_read_loses_nothing() {
        // The follower is behind: most of attempt 1 is unread when attempt 2
        // reopens the path.
        let rig = Rig::new();
        let mut old = fs::File::create(&rig.path).unwrap();
        old.write_all(b"o").unwrap();
        let mut f = rig.own();
        assert_eq!(read_now(&mut f, 1), b"o");
        old.write_all(b"ne-unread-tail").unwrap();
        drop(old);
        fs::File::create(&rig.path)
            .unwrap()
            .write_all(b"two")
            .unwrap();
        rig.finish();
        let (got, _) = finished(read_in_background(f));
        assert_eq!(got, b"ne-unread-tailtwo");
    }

    #[test]
    fn the_old_file_is_read_until_its_writer_lets_go() {
        // The next attempt's file is there while the old one's writer is
        // still flushing.
        let rig = Rig::new();
        let mut old = fs::File::create(&rig.path).unwrap();
        old.write_all(b"one").unwrap();
        let mut f = rig.own_with_quiet(Duration::from_secs(10));
        assert_eq!(read_now(&mut f, 3), b"one");
        fs::write(&rig.path, b"two").unwrap();
        let rx = read_in_background(f);
        std::thread::sleep(QUIET_TEST / 4);
        old.write_all(b"-late").unwrap();
        drop(old);
        rig.finish();
        let (got, _) = finished(rx);
        assert_eq!(got, b"-latetwo");
    }

    #[test]
    fn bytes_written_just_before_the_writer_lets_go_of_the_old_file_are_read() {
        let rig = Rig::new();
        let old = Arc::new(std::sync::Mutex::new(Some(
            fs::File::create(&rig.path).unwrap(),
        )));
        old.lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .write_all(b"one")
            .unwrap();
        let o = old.clone();
        let mut gate = Gate::new(|| true).with_held(move |_| {
            // The tail lands between the follower's last read and its look.
            if let Some(mut w) = o.lock().unwrap().take() {
                w.write_all(b"-tail").unwrap();
            }
            false
        });
        gate.quiet = Duration::from_secs(10);
        let mut f = File::new(rig.path.clone(), gate, true);
        assert_eq!(read_now(&mut f, 3), b"one");
        fs::write(&rig.path, b"two").unwrap();
        let (got, _) = finished(read_in_background(f));
        assert_eq!(got, b"-tailtwo");
    }

    #[test]
    fn a_file_found_by_the_last_look_before_a_verdict_is_still_read() {
        let rig = Rig::new();
        let path = rig.path.clone();
        let mut started_over = false;
        // The next file appears just as the owner is seen to have ended.
        let mut gate = Gate::new(move || {
            if !started_over {
                started_over = true;
                fs::write(&path, b"two").unwrap();
            }
            true
        })
        .with_held(|_| true);
        test_windows(&mut gate);
        fs::write(&rig.path, b"one").unwrap();
        let mut f = File::new(rig.path.clone(), gate, true);
        assert_eq!(read_now(&mut f, 3), b"one");
        f.expect(Expect::NothingOnceEnded);
        let (got, _) = finished(read_in_background(f));
        assert_eq!(got, b"two");
    }

    #[test]
    fn a_tail_written_slowly_before_the_writer_lets_go_is_read_whole() {
        // Each write comes within the window, and all of them outlast it.
        let rig = Rig::new();
        let mut old = fs::File::create(&rig.path).unwrap();
        let mut f = rig.own_with_quiet(Duration::from_secs(1));
        old.write_all(b"one").unwrap();
        assert_eq!(read_now(&mut f, 3), b"one");
        fs::write(&rig.path, b"two").unwrap();
        let rx = read_in_background(f);
        for part in [b"-a", b"-b", b"-c", b"-d"] {
            std::thread::sleep(Duration::from_millis(300));
            old.write_all(part).unwrap();
        }
        drop(old);
        rig.finish();
        let (got, _) = finished(rx);
        assert_eq!(got, b"-a-b-c-dtwo");
    }

    #[test]
    fn a_writer_that_never_lets_go_is_moved_on_from_after_the_window() {
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        let mut f = rig.own();
        assert_eq!(read_now(&mut f, 3), b"one");
        fs::write(&rig.path, b"two").unwrap();
        let t = Instant::now();
        assert_eq!(read_now(&mut f, 3), b"two", "held throughout");
        assert!(t.elapsed() >= QUIET_TEST);
    }

    #[test]
    fn a_writer_that_starts_over_twice_before_the_follower_gets_there_loses_nothing() {
        let rig = Rig::new();
        fs::write(&rig.path, b"one-unread").unwrap();
        let mut f = rig.own();
        assert_eq!(read_now(&mut f, 1), b"o");
        fs::write(&rig.path, b"two").unwrap();
        std::thread::sleep(TICK * 3);
        assert_eq!(read_now(&mut f, 1), b"n", "still in the first file");
        assert!(
            !rig.path.exists(),
            "the second was moved away as it appeared"
        );
        fs::write(&rig.path, b"six").unwrap();
        rig.finish();
        let (got, _) = finished(read_in_background(f));
        assert_eq!(got, b"e-unreadtwosix");
    }

    #[test]
    fn a_new_file_that_cannot_be_moved_breaks_the_stream_while_bytes_flow() {
        let rig = Rig::new();
        let mut old = fs::File::create(&rig.path).unwrap();
        old.write_all(b"one").unwrap();
        let mut f = rig.own();
        assert_eq!(read_now(&mut f, 1), b"o");
        // The name the next file would be moved to is taken.
        let taken = moved_name(&rig.path, 2);
        fs::create_dir_all(taken.join("in-the-way")).unwrap();
        fs::write(&rig.path, b"two").unwrap();
        std::thread::sleep(TICK * 3);
        let mut buf = [0u8; 8];
        let read = f.read(&mut buf).unwrap();
        drop(old);
        let _ = fs::remove_dir_all(&taken);
        assert_eq!(read, 0);
        assert!(
            matches!(f.done(), Some(Done::Broken { .. })),
            "{:?}",
            f.done()
        );
    }

    #[test]
    fn each_of_several_replacements_is_followed() {
        let rig = Rig::new();
        let mut f = rig.own();
        for attempt in [&b"one"[..], b"two", b"six"] {
            fs::write(&rig.path, attempt).unwrap();
            assert_eq!(read_now(&mut f, 3), attempt);
        }
        fs::write(&rig.path, b"last").unwrap();
        rig.finish();
        let (got, f) = finished(read_in_background(f));
        assert_eq!(got, b"last");
        assert_eq!(f.bytes_read(), 13);
    }

    #[test]
    fn an_owned_follower_removes_every_file_it_read() {
        let rig = Rig::new();
        let mut f = rig.own();
        fs::write(&rig.path, b"one").unwrap();
        assert_eq!(read_now(&mut f, 3), b"one");
        fs::write(&rig.path, b"two").unwrap();
        assert_eq!(read_now(&mut f, 3), b"two");
        assert!(
            !moved_name(&rig.path, 1).exists(),
            "a file is removed once it is moved on from"
        );
        drop(f);
        assert!(!rig.path.exists());
        assert!(!moved_name(&rig.path, 2).exists());
    }

    #[test]
    fn held_is_asked_about_the_name_the_file_is_read_under() {
        let rig = Rig::new();
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let a = asked.clone();
        let mut gate = Gate::new(|| true).with_held(move |path| {
            a.lock().unwrap().push(path.to_path_buf());
            false
        });
        test_windows(&mut gate);
        fs::write(&rig.path, b"one").unwrap();
        let (got, _) = finished(read_in_background(File::new(rig.path.clone(), gate, true)));
        assert_eq!(got, b"one");
        assert_eq!(*asked.lock().unwrap(), vec![moved_name(&rig.path, 1)]);
    }

    #[test]
    fn the_gap_before_a_replacement_is_waited_out() {
        // Owner ended and nobody holding the file would be R2 at once; only
        // `Expect::More` turns the gap into R4's window.
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        // The read ends on R4, so this window is also how long it takes.
        let mut f = rig.own_with_quiet(Duration::from_secs(2));
        assert_eq!(read_now(&mut f, 3), b"one");
        f.expect(Expect::More);
        f.expect_replacement();
        rig.finish();
        let path = rig.path.clone();
        let replacement = std::thread::spawn(move || {
            std::thread::sleep(QUIET_TEST / 4);
            fs::write(&path, b"two").unwrap();
        });
        let (got, f) = finished(read_in_background(f));
        replacement.join().unwrap();
        assert_eq!(got, b"two");
        assert!(!f.awaiting_replacement());
    }

    #[test]
    fn a_replacement_that_never_comes_ends_with_the_owner() {
        // Bazel ran out of retries. `held` says "held", and is not asked.
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        let mut f = rig.own();
        assert_eq!(read_now(&mut f, 3), b"one");
        f.expect_replacement();
        f.expect(Expect::NothingOnceEnded);
        let rx = read_in_background(f);
        assert!(
            still_waiting(&rx),
            "a replacement can come while the owner runs"
        );
        rig.end_owner();
        let (got, f) = finished(rx);
        assert_eq!(got, b"");
        assert!(f.awaiting_replacement());
        assert!(
            matches!(f.done(), Some(Done::Drained { bytes: 3 })),
            "R2′, not R4's window: {:?}",
            f.done()
        );
    }

    #[test]
    fn a_replacement_is_still_moved_to_once_nothing_more_is_expected_of_the_old_file() {
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        let mut f = rig.own();
        assert_eq!(read_now(&mut f, 3), b"one");
        f.expect_replacement();
        f.expect(Expect::NothingOnceEnded);
        fs::write(&rig.path, b"two").unwrap();
        rig.end_owner();
        let (got, f) = finished(read_in_background(f));
        assert_eq!(got, b"two");
        assert!(!f.awaiting_replacement());
    }

    #[test]
    fn a_path_the_follower_does_not_own_is_never_moved() {
        let rig = Rig::new();
        fs::write(&rig.path, b"one").unwrap();
        let mut f = rig.follow();
        assert_eq!(read_now(&mut f, 3), b"one");
        assert!(rig.path.exists());
    }

    #[test]
    fn a_file_truncated_in_place_is_read_again_from_the_start() {
        // Only a path the follower does not own can be started over in place.
        let rig = Rig::new();
        fs::write(&rig.path, b"attempt one").unwrap();
        let mut f = rig.follow();
        assert_eq!(read_now(&mut f, 11), b"attempt one");
        f.expect_replacement();
        fs::write(&rig.path, b"two").unwrap();
        let mut buf = [0u8; 8];
        let err = f.read(&mut buf).unwrap_err();
        assert!(StartedOver::is(&err), "{err}");
        assert!(!f.awaiting_replacement());
        rig.finish();
        let (got, _) = finished(read_in_background(f));
        assert_eq!(got, b"two");
    }

    #[test]
    fn without_a_held_hint_the_next_file_is_moved_to_at_once() {
        // What a missing server pid gives: no telling whether the writer is
        // done with the old file, so no waiting on it.
        let rig = Rig::new();
        let mut gate = rig.flags.gate(false);
        gate.quiet = Duration::from_secs(10);
        fs::write(&rig.path, b"one").unwrap();
        let mut f = File::new(rig.path.clone(), gate, true);
        assert_eq!(read_now(&mut f, 3), b"one");
        fs::write(&rig.path, b"two").unwrap();
        let t = Instant::now();
        assert_eq!(read_now(&mut f, 3), b"two");
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
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
        let (tx, rx) = mpsc::channel();
        let mut f = rig.follow();
        std::thread::spawn(move || {
            let _ = tx.send(f.appear());
        });
        assert!(
            still_waiting(&rx),
            "no file and a living owner is not an answer"
        );
        rig.end_owner();
        assert!(!finished(rx));
    }

    #[test]
    fn appear_does_not_miss_a_file_created_in_the_verdict_window() {
        let rig = Rig::new();
        let path = rig.path.clone();
        let mut gate = Gate::new(|| true).with_held(move |_| {
            fs::write(&path, b"whole").unwrap();
            false
        });
        test_windows(&mut gate);
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
