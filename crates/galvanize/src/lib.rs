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
