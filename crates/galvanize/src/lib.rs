use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::path::PathBuf;

use nix::sys::stat::Mode;
use nix::unistd::mkfifo;
use proc_pidinfo::*;

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
    pub fn new(path: PathBuf, policy: RetryPolicy) -> io::Result<Self> {
        mkfifo(&path, Mode::S_IRWXO | Mode::S_IRWXU | Mode::S_IRWXG)?;
        let inner = File::open(&path)?;
        let path = path.canonicalize()?;
        Ok(Self {
            inner,
            policy,
            path,
        })
    }

    #[cfg(target_os = "linux")]
    fn is_path_open(&self, pid: u32) -> io::Result<bool> {
        use procfs::process::{FDTarget, Process};
        let proc = Process::new(pid as i32).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        for fd in proc.fd()? {
            let fd = fd?;
            if let FDTarget::Path(fd_path) = fd.target {
                // Resolve the path exactly as the kernel reports it
                if fd_path == *path {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    #[cfg(target_os = "macos")]
    pub fn is_path_open(&self, pid: u32) -> io::Result<bool> {
        let pid = Pid(pid);
        for fd in proc_pidinfo_list::<ProcFDInfo>(pid)? {
            match proc_pidfdinfo::<VnodeFdInfoWithPath>(pid, fd.proc_fd)? {
                Some(vnode) => match vnode.path() {
                    Ok(p) if self.path == p => {
                        return Ok(true);
                    }
                    // ignore vnode entries without a path
                    _ => continue,
                },
                None => continue,
            }
        }
        Ok(false)
    }

    fn read_with_policy(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.policy {
            RetryPolicy::Never => self.inner.read(buf).map_err(|err| err.into()),
            RetryPolicy::IfOpenForPid(pid) => loop {
                match self.inner.read(buf) {
                    Ok(nr) if nr == 0 => {
                        // it is okay to return an empty buffer if the FD is still open.
                        if self.is_path_open(pid)? {
                            return Ok(nr);
                        } else {
                            return Err(std::io::Error::new(
                                ErrorKind::BrokenPipe,
                                "end of stream",
                            ));
                        }
                    }
                    // If EOF error was encountered and the path is still open by the PID
                    // then retry this stream.
                    Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
                        if self.is_path_open(pid)? {
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
