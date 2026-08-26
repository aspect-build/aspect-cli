use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};

use nix::sys::stat::Mode;
use nix::unistd::mkfifo;

/// Returns `false` when the process does not exist (ESRCH) or is a zombie.
/// EPERM (process exists but we can't signal it) is treated as alive.
pub fn is_pid_alive(pid: u32) -> bool {
    // SAFETY: kill(pid, 0) is the standard POSIX existence check. Signal 0 is
    // never delivered; the call only validates the pid and our permission to
    // signal it.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc != 0 {
        return io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
    }
    // kill(pid, 0) succeeds for zombie processes: they still hold a PID slot
    // until the parent calls waitpid, but they have already exited and will
    // never create new files. Treat them as dead so callers don't spin forever.
    !is_pid_zombie(pid)
}

#[cfg(target_os = "linux")]
fn is_pid_zombie(pid: u32) -> bool {
    use procfs::process::Process;
    Process::new(pid as i32)
        .and_then(|p| p.stat())
        .map(|s| s.state == 'Z')
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn is_pid_zombie(pid: u32) -> bool {
    // Two zombie signals on macOS:
    //   1. ret > 0 with pbi_status == SZOMB (5) — kernel populated bsdinfo
    //      and explicitly reports the zombie state.
    //   2. ret == 0 — kernel returned no bsdinfo even though `kill(pid, 0)`
    //      succeeded a moment ago. In practice this is what we observe for
    //      zombies on contemporary macOS: bsdinfo stops being populated
    //      once the process has exited but the pid slot is held open
    //      waiting for `wait()` (case (1) is documented but not produced
    //      in our reproductions). Verified empirically against a `true`
    //      child that had exited but not yet been waited on.
    use std::mem;
    unsafe {
        let mut info: libc::proc_bsdinfo = mem::zeroed();
        let ret = libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
        );
        ret == 0 || (ret > 0 && info.pbi_status == 5)
    }
}

#[cfg(target_os = "linux")]
fn is_path_open_for_pid(path: &Path, pid: u32) -> io::Result<bool> {
    use procfs::process::{FDTarget, Process};
    // A dead pid has no /proc/<pid> directory. Treat that as "not open"
    // rather than propagating an error: callers use this to decide whether
    // to keep waiting for more bytes, and a dead writer can never write more.
    if !is_pid_alive(pid) {
        return Ok(false);
    }
    let proc = match Process::new(pid as i32) {
        Ok(p) => p,
        // Race: pid was alive a moment ago but exited before we could open
        // its procfs entry. Same logical answer — no longer holding the file.
        Err(_) => return Ok(false),
    };
    for fd in proc.fd().map_err(|err| io::Error::other(err))? {
        let fd = fd.map_err(|err| io::Error::other(err))?;
        if let FDTarget::Path(fd_path) = &fd.target {
            if fd_path == path {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn is_path_open_for_pid(path: &Path, pid: u32) -> io::Result<bool> {
    use proc_pidinfo::*;
    // proc_pidinfo silently returns 0 fds for a dead pid on macOS, so the
    // loop below would already report "not open" — but skip it explicitly
    // to keep the cross-platform contract identical with the Linux branch.
    if !is_pid_alive(pid) {
        return Ok(false);
    }
    let pid_val = Pid(pid);
    for fd in proc_pidinfo_list::<ProcFDInfo>(pid_val)? {
        match proc_pidfdinfo::<VnodeFdInfoWithPath>(pid_val, fd.proc_fd)? {
            Some(vnode) => match vnode.path() {
                Ok(p) => {
                    if path == p {
                        return Ok(true);
                    }
                }
                _ => continue,
            },
            None => continue,
        }
    }
    Ok(false)
}

/// Drop `O_NONBLOCK` from an open descriptor, so reads block again.
fn clear_nonblocking(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let fd = file.as_raw_fd();
    // SAFETY: `fd` is owned by `file` and stays open for this call; F_GETFL and
    // F_SETFL only read and rewrite this descriptor's status flags.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub struct Pipe {
    path: PathBuf,
    inner: File,
    policy: RetryPolicy,
    /// Bytes already read off the FIFO while waiting for a writer to show up
    /// (see [`Pipe::open_waiting_for_writer`]), handed to the next `read`
    /// before the fd is touched again. Empty for every other constructor.
    pending: Vec<u8>,
}

pub enum RetryPolicy {
    /// Never retry
    Never,
    /// Retries EOF errors if the FD is still open by the PID
    IfOpenForPid(u32),
}

impl Pipe {
    /// Create the FIFO inode at `path`. Does not open it. Idempotent —
    /// returns `Ok(())` if the FIFO already exists at `path` (EEXIST).
    ///
    /// Useful when the caller needs the FIFO to exist on disk before
    /// spawning the writer process — e.g. so the spawned process can pass
    /// the path as a flag and `open(O_WRONLY)` will find the FIFO instead
    /// of `ENOENT`. After mkfifo, call `open` from whichever thread owns
    /// the read end.
    pub fn mkfifo(path: &Path) -> io::Result<()> {
        match mkfifo(path, Mode::S_IRWXO | Mode::S_IRWXU | Mode::S_IRWXG) {
            Ok(()) => Ok(()),
            Err(nix::errno::Errno::EEXIST) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Open the read end of an existing FIFO at `path`. Blocks until a
    /// writer connects (POSIX FIFO semantics) unless one already has.
    /// Pair with `mkfifo` when the caller needs to control ordering
    /// between FIFO creation and writer spawn.
    pub fn open(path: PathBuf, policy: RetryPolicy) -> io::Result<Self> {
        let inner = File::open(&path)?;
        let path = path.canonicalize()?;
        Ok(Self {
            inner,
            policy,
            path,
            pending: Vec::new(),
        })
    }

    /// Open the read end of an existing FIFO without committing to a writer
    /// ever arriving. Returns `ErrorKind::BrokenPipe` once `should_wait`
    /// reports the writer is no longer coming.
    ///
    /// [`Pipe::open`] cannot do this: a blocking `open(O_RDONLY)` on a FIFO
    /// parks in the kernel until a writer opens the other end, and nothing
    /// short of a signal calls it off — so a writer that dies *before* opening
    /// strands the caller for good, with no read for [`RetryPolicy`] to
    /// govern. `O_NONBLOCK` makes the open return at once; this then polls
    /// every `poll_interval`, asking `should_wait` whether to keep waiting.
    ///
    /// Readiness is decided by a read rather than by `poll` flags, whose
    /// meaning for an unconnected FIFO varies by platform. The read results
    /// POSIX does pin down are easy to read backwards:
    ///
    /// * `Ok(0)` — *nobody* has the FIFO open for writing. Ambiguous between
    ///   "the writer has not gotten to it yet" and "the writer is finished",
    ///   so it is a waiting state rather than an answer. Mistaking it for a
    ///   writer sighting hands back a pipe whose first read reports the stream
    ///   already over, losing everything the writer had yet to send.
    /// * `EAGAIN` — a writer *is* attached, with nothing written yet. Proof it
    ///   arrived, so the wait ends here.
    /// * `Ok(n)` — data, which this read has consumed; the bytes are held in
    ///   `pending` for the first [`Read::read`] call.
    ///
    /// Once a writer has been seen the descriptor goes back to blocking, so
    /// streaming reads cost exactly what [`Pipe::open`]'s do.
    pub fn open_waiting_for_writer(
        path: PathBuf,
        policy: RetryPolicy,
        poll_interval: std::time::Duration,
        mut should_wait: impl FnMut() -> bool,
    ) -> io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;

        /// Enough to hold whatever a writer managed to send before this thread
        /// got to its first read; the rest stays in the FIFO for later reads.
        const FIRST_READ_CAPACITY: usize = 4096;

        let inner = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)?;

        let mut pending = vec![0u8; FIRST_READ_CAPACITY];
        let mut buffered = 0;
        // Whether `should_wait` has already gone false. Reading once more
        // after it does is what keeps a writer that filled the FIFO and exited
        // between two polls from being treated as one that never wrote.
        let mut writer_gone = false;
        loop {
            match (&inner).read(&mut pending) {
                Ok(n) if n > 0 => {
                    buffered = n;
                    break;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Ok(_) => {}
                Err(e) => return Err(e),
            }
            if writer_gone {
                return Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "no writer opened the pipe",
                ));
            }
            writer_gone = !should_wait();
            std::thread::sleep(poll_interval);
        }
        pending.truncate(buffered);

        clear_nonblocking(&inner)?;
        let path = path.canonicalize()?;
        Ok(Self {
            inner,
            policy,
            path,
            pending,
        })
    }

    /// Convenience: `mkfifo` + `open`. Equivalent to the original
    /// monolithic constructor; appropriate when the caller does not need
    /// to interleave other work between the two steps.
    pub fn new(path: PathBuf, policy: RetryPolicy) -> io::Result<Self> {
        Self::mkfifo(&path)?;
        Self::open(path, policy)
    }

    fn read_with_policy(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.pending.is_empty() {
            let n = buf.len().min(self.pending.len());
            buf[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            return Ok(n);
        }
        match self.policy {
            RetryPolicy::Never => self.inner.read(buf).map_err(|err| err.into()),
            RetryPolicy::IfOpenForPid(pid) => loop {
                match self.inner.read(buf) {
                    Ok(nr) if nr == 0 => {
                        if is_path_open_for_pid(&self.path, pid)? {
                            return Ok(nr);
                        } else {
                            return Err(std::io::Error::new(
                                ErrorKind::BrokenPipe,
                                "end of stream",
                            ));
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
                        if is_path_open_for_pid(&self.path, pid)? {
                            continue;
                        } else {
                            return Err(std::io::Error::new(
                                ErrorKind::BrokenPipe,
                                "end of stream",
                            ));
                        }
                    }
                    Ok(nr) => return Ok(nr),
                    Err(err) => return Err(err.into()),
                }
            },
        }
    }
}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_with_policy(buf)
    }
}

/// A regular file that streams its contents as the writer (identified by `pid`) appends to it.
///
/// Busy-polls for file existence at open time, then reads with the same retry logic as
/// [`Pipe`] with [`RetryPolicy::IfOpenForPid`]: on EOF, checks whether the writer process
/// still has the file open. Returns `BrokenPipe` when the writer closes the file.
pub struct StreamingFile {
    path: PathBuf,
    inner: File,
    pid: u32,
}

impl StreamingFile {
    /// Polls until `path` exists (10 ms sleep between checks), then opens it.
    /// Returns `BrokenPipe` immediately if `pid` exits before the file appears.
    /// Path is canonicalized after open for accurate fd matching.
    pub fn open(path: PathBuf, pid: u32) -> io::Result<Self> {
        while !path.exists() {
            if !is_pid_alive(pid) {
                return Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "process exited before the file was created",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let inner = File::open(&path)?;
        let path = path.canonicalize()?;
        Ok(Self { path, inner, pid })
    }
}

impl Read for StreamingFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buf) {
            // Ok(0): at the current end of the file. If the writer still has it open,
            // return Ok(0) to signal "no data yet, try again later". If the writer
            // has closed the file, the stream is done — signal BrokenPipe.
            // Callers that cannot tolerate Ok(0) (e.g. a zstd Decoder) should wrap
            // this in a blocking retry adapter.
            Ok(0) => {
                if is_path_open_for_pid(&self.path, self.pid)? {
                    Ok(0)
                } else {
                    Err(std::io::Error::new(ErrorKind::BrokenPipe, "end of stream"))
                }
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::OpenOptions;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    const POLL: Duration = Duration::from_millis(5);

    /// A FIFO of our own, created and owned by the calling test.
    fn fifo() -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "galvanize-test-{}-{}.fifo",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Pipe::mkfifo(&path).expect("mkfifo");
        path
    }

    /// Nothing ever opens the write end, and the caller stops waiting: the open
    /// has to fail rather than park in the kernel the way `open` would.
    #[test]
    fn gives_up_when_no_writer_ever_opens() {
        let path = fifo();
        let started = Instant::now();
        let err = match Pipe::open_waiting_for_writer(path.clone(), RetryPolicy::Never, POLL, || {
            false
        }) {
            Ok(_) => panic!("must not hand back a pipe no writer will ever use"),
            Err(e) => e,
        };

        assert_eq!(err.kind(), ErrorKind::BrokenPipe);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "gave up after {:?}; it should take a couple of polls",
            started.elapsed()
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A writer that has attached without sending anything yet is still a
    /// writer, and `EAGAIN` is the only proof of one — so the wait ends there,
    /// whatever the guard says.
    ///
    /// The writer opens from another thread because `open(O_WRONLY)` on a FIFO
    /// blocks until a reader arrives: opening it up front would deadlock
    /// against the very open under test.
    #[test]
    fn ends_the_wait_when_a_writer_attaches_without_writing() {
        let path = fifo();
        let path_w = path.clone();
        let writer = std::thread::spawn(move || {
            OpenOptions::new()
                .write(true)
                .open(&path_w)
                .expect("open write end")
        });

        // Bounded rather than `|| true`, so a regression fails the test
        // instead of hanging it.
        let mut polls = 0;
        let pipe = Pipe::open_waiting_for_writer(path.clone(), RetryPolicy::Never, POLL, || {
            polls += 1;
            polls < 200
        });
        assert!(pipe.is_ok(), "an attached writer must end the wait");

        drop(writer.join().expect("writer thread"));
        let _ = std::fs::remove_file(&path);
    }

    /// The wait spends a read to learn where it stands, so whatever that read
    /// swallowed has to reach the caller's first `read`.
    #[test]
    fn hands_over_bytes_it_read_while_waiting() {
        let path = fifo();
        let path_w = path.clone();

        // Writing from inside the guard puts the bytes in the FIFO before the
        // loop's next read, which is what makes that read return data rather
        // than EAGAIN. Doing it from another thread would race that ordering.
        let mut wrote = false;
        let mut pipe = Pipe::open_waiting_for_writer(path.clone(), RetryPolicy::Never, POLL, || {
            if !wrote {
                wrote = true;
                let mut w = OpenOptions::new()
                    .write(true)
                    .open(&path_w)
                    .expect("open write end");
                w.write_all(b"hello world").expect("write");
            }
            true
        })
        .expect("data queued on the FIFO must end the wait");

        // Read in small chunks: the hand-over buffer has to survive being
        // drained across several reads, not just one big one.
        let mut got = Vec::new();
        let mut chunk = [0u8; 4];
        loop {
            match pipe.read(&mut chunk).expect("read") {
                0 => break,
                n => got.extend_from_slice(&chunk[..n]),
            }
        }
        assert_eq!(got, b"hello world");
        let _ = std::fs::remove_file(&path);
    }

    /// A writer can fill the FIFO and exit between two polls. Its bytes are
    /// still in the pipe, so noticing it left must not throw them away.
    #[test]
    fn delivers_bytes_from_a_writer_that_left_between_polls() {
        let path = fifo();
        let path_w = path.clone();

        // The whole writer lifetime — attach, write, leave — happens inside one
        // guard call, and the guard says "gone" every time. So the bytes can
        // only come back via the read taken after the writer is known gone.
        let mut wrote = false;
        let mut pipe = Pipe::open_waiting_for_writer(path.clone(), RetryPolicy::Never, POLL, || {
            if !wrote {
                wrote = true;
                let mut w = OpenOptions::new()
                    .write(true)
                    .open(&path_w)
                    .expect("open write end");
                w.write_all(b"late").expect("write");
            }
            false
        })
        .expect("bytes written before the last read must be delivered");

        let mut got = [0u8; 4];
        pipe.read_exact(&mut got).expect("read");
        assert_eq!(&got, b"late");
        let _ = std::fs::remove_file(&path);
    }
}
