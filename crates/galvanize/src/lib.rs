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

pub struct Pipe {
    path: PathBuf,
    inner: File,
    policy: RetryPolicy,
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

    /// Release a reader parked in [`Pipe::open`] by briefly becoming the writer
    /// it is waiting for: open the write end and close it without writing.
    /// The reader's open returns and its first read reports end-of-stream.
    ///
    /// For callers whose real writer may die before it ever opens the FIFO:
    /// that leaves the `open` parked in the kernel with no read for
    /// [`RetryPolicy`] to govern, and nothing short of a signal to call it off.
    ///
    /// Returns whether a reader was actually waiting. `O_NONBLOCK` keeps this
    /// from becoming the mirror image of the problem it solves: with no reader
    /// on the other side the open fails `ENXIO` rather than parking, reported
    /// here as `Ok(false)`.
    ///
    /// Harmless to call when the real writer did arrive: a FIFO reports
    /// end-of-stream only once *every* writer has closed, so a poke alongside
    /// a live writer is invisible to the reader.
    pub fn poke_writer(path: &Path) -> io::Result<bool> {
        use std::os::unix::fs::OpenOptionsExt;

        match std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
        {
            Ok(_) => Ok(true),
            Err(e) if e.raw_os_error() == Some(libc::ENXIO) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Open the read end of an existing FIFO at `path`. Blocks until a
    /// writer connects (POSIX FIFO semantics) unless one already has.
    /// Pair with `mkfifo` when the caller needs to control ordering
    /// between FIFO creation and writer spawn.
    ///
    /// A writer that never arrives parks this forever; see
    /// [`Pipe::poke_writer`] for the way out.
    pub fn open(path: PathBuf, policy: RetryPolicy) -> io::Result<Self> {
        let inner = File::open(&path)?;
        let path = path.canonicalize()?;
        Ok(Self {
            inner,
            policy,
            path,
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

    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

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

    /// The reason `poke_writer` exists: a reader parked in `open` with no writer
    /// coming has to be able to get out, and what it sees on the way out is an
    /// ended stream.
    #[test]
    fn poking_releases_a_parked_reader() {
        let path = fifo();
        let path_r = path.clone();
        let reader = std::thread::spawn(move || {
            let mut pipe = Pipe::open(path_r, RetryPolicy::Never).expect("open read end");
            let mut buf = [0u8; 8];
            pipe.read(&mut buf).expect("read")
        });

        // Poll rather than sleep-once: the poke only works while the reader is
        // parked, and `Ok(false)` says it wasn't there yet.
        let mut poked = false;
        for _ in 0..200 {
            if Pipe::poke_writer(&path).expect("poke") {
                poked = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            poked,
            "a reader was parked in open; the poke should have found it"
        );
        assert_eq!(
            reader.join().expect("reader thread"),
            0,
            "the released reader must see end-of-stream"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// With nobody parked the poke must report that and return, not park in the
    /// kernel itself waiting for a reader.
    #[test]
    fn poking_an_unattended_fifo_reports_no_reader() {
        let path = fifo();
        assert!(
            !Pipe::poke_writer(&path).expect("poke must not fail"),
            "no reader is waiting on this FIFO"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A poke is allowed to lose the race and land while the real writer holds
    /// the FIFO open. The reader must not see that as the end of anything.
    #[test]
    fn poking_alongside_a_live_writer_does_not_end_the_stream() {
        let path = fifo();
        let path_r = path.clone();
        let reader = std::thread::spawn(move || {
            let mut pipe = Pipe::open(path_r, RetryPolicy::Never).expect("open read end");
            let mut got = Vec::new();
            let mut buf = [0u8; 4];
            loop {
                match pipe.read(&mut buf).expect("read") {
                    0 => break,
                    n => got.extend_from_slice(&buf[..n]),
                }
            }
            got
        });

        let mut w = {
            use std::io::Write;
            let mut w = None;
            for _ in 0..200 {
                match std::fs::OpenOptions::new().write(true).open(&path) {
                    Ok(f) => {
                        w = Some(f);
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
            let mut f = w.expect("open write end");
            f.write_all(b"before").expect("write");
            f
        };

        // Poke while the real writer is still attached, then keep writing.
        let _ = Pipe::poke_writer(&path).expect("poke");
        {
            use std::io::Write;
            w.write_all(b"after").expect("write after the poke");
        }
        drop(w);

        assert_eq!(
            reader.join().expect("reader thread"),
            b"beforeafter".to_vec(),
            "a poke beside a live writer must not truncate the stream"
        );
        let _ = std::fs::remove_file(&path);
    }
}
