//! Bazel's own output tree, shared with the `bazel` command line. The engine
//! writes exactly what a Bazel server would, so the on-disk caches are reused:
//! external repositories, the repository cache, the action cache, lockfiles.
//! In-memory Skyframe state is per process and is never shared.

use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// `<output root>/_bazel_<user>`, as the Bazel client derives it when no
/// `--output_user_root` is given.
pub fn output_user_root() -> Result<PathBuf> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".into());
    Ok(output_root()?.join(format!("_bazel_{user}")))
}

/// `<output_user_root>/<md5(workspace path)>`, Bazel's default output base.
pub fn output_base(output_user_root: &Path, workspace: &Path) -> PathBuf {
    let digest = Md5::digest(workspace.as_os_str().as_encoded_bytes());
    output_user_root.join(format!("{digest:x}"))
}

/// macOS: `$XDG_CACHE_HOME/bazel`, else `~/Library/Caches/bazel`.
/// Linux: `$XDG_CACHE_HOME/bazel`, else `~/.cache/bazel`.
fn output_root() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME")
        && !xdg.is_empty()
    {
        return Ok(PathBuf::from(xdg).join("bazel"));
    }
    let home = dirs::home_dir().context("no home directory")?;
    Ok(if cfg!(target_os = "macos") {
        home.join("Library").join("Caches").join("bazel")
    } else {
        home.join(".cache").join("bazel")
    })
}

/// The exclusive lock the Bazel client holds on `<output_base>/lock` for the
/// length of a command. Held for the engine's lifetime, released on drop.
pub struct Lock {
    _file: File,
}

impl Lock {
    pub fn acquire(output_base: &Path, workspace: &Path) -> Result<Lock> {
        let path = output_base.join("lock");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        if !try_lock(&file)? {
            let mut owner = String::new();
            file.seek(SeekFrom::Start(0))?;
            file.read_to_string(&mut owner).ok();
            bail!(
                "another command holds the lock on {}:\n{}",
                output_base.display(),
                owner.trim_end()
            );
        }
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "pid={}", std::process::id())?;
        writeln!(file, "owner=buildx")?;
        writeln!(file, "cwd={}", workspace.display())?;
        Ok(Lock { _file: file })
    }
}

/// The same lock the client takes: a write lock on byte 0, open-file-description
/// style where the kernel offers it so a stray close in this process cannot
/// drop it, POSIX record lock otherwise.
fn try_lock(file: &File) -> Result<bool> {
    let mut lock = libc::flock {
        l_type: libc::F_WRLCK as _,
        l_whence: libc::SEEK_SET as _,
        l_start: 0,
        l_len: 1,
        l_pid: 0,
    };
    let fd = file.as_raw_fd();
    for cmd in [libc::F_OFD_SETLK, libc::F_SETLK] {
        if unsafe { libc::fcntl(fd, cmd, &mut lock) } == 0 {
            return Ok(true);
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EACCES) | Some(libc::EAGAIN) => return Ok(false),
            Some(libc::EINVAL) if cmd == libc::F_OFD_SETLK => continue,
            _ => return Err(err).context("fcntl on output base lock"),
        }
    }
    Ok(false)
}

/// A live Bazel server trusts its in-memory graph to match this output base;
/// writing underneath it would desynchronise the two, so refuse.
pub fn ensure_no_server(output_base: &Path, workspace: &Path) -> Result<()> {
    let pid_file = output_base.join("server").join("server.pid.txt");
    let Ok(text) = fs::read_to_string(&pid_file) else {
        return Ok(());
    };
    let Ok(pid) = text.trim().parse::<i32>() else {
        return Ok(());
    };
    if unsafe { libc::kill(pid, 0) } == 0 {
        bail!(
            "a Bazel server (pid {pid}) owns {}; run `bazel shutdown` in {} first",
            output_base.display(),
            workspace.display()
        );
    }
    Ok(())
}
