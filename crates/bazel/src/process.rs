//! Questions about other processes: is one alive, does one hold a path open.
//! Everything that differs per OS (procfs, `proc_pidinfo`) is here.

use std::io;
use std::path::Path;

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
pub fn is_path_open_for_pid(path: &Path, pid: u32) -> io::Result<bool> {
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
pub fn is_path_open_for_pid(path: &Path, pid: u32) -> io::Result<bool> {
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
