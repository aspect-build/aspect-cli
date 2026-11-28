//! `aspect history browse` off a terminal: refused with the error that names
//! `aspect history list`, no verdict line after it, no run of its own kept
//! in the history, and no `--mouse` flag. On a terminal, a signal ends it
//! with the shell's code for that signal.

mod common;

use common::{aspect_cli, base_env};
use std::process::{Command, Output, Stdio};

/// Run `aspect history <args>` in a scratch workspace with piped stdio and
/// a history folder of its own, returned for a look at what it kept.
fn run(args: &[&str]) -> (Output, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    let output = Command::new(aspect_cli())
        .arg("history")
        .args(args)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("running `aspect history {}`: {e}", args.join(" ")));
    (output, dir)
}

#[test]
fn browse_off_a_terminal_is_refused_quietly_and_keeps_no_run() {
    let (output, dir) = run(&["browse"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(
            "aspect history browse needs a terminal; `aspect history list` prints the runs."
        ),
        "{stderr}"
    );
    assert!(
        !stderr.contains("history browse task"),
        "no verdict line after browse:\n{stderr}"
    );
    let kept: Vec<_> = std::fs::read_dir(dir.path().join("history"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("task.json").exists())
        .collect();
    assert!(
        kept.is_empty(),
        "aspect history keeps no run of its own: {kept:?}"
    );
}

#[test]
fn browse_has_no_mouse_flag() {
    let (output, _dir) = run(&["browse", "--mouse=false"]);
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--mouse"), "{stderr}");
}

/// `aspect history browse` on a pseudo-terminal of its own, its session's
/// controlling terminal, in a scratch workspace with an empty history; the
/// child, a channel of what it writes, and the folder.
#[cfg(unix)]
fn on_a_pty() -> (
    std::process::Child,
    std::sync::mpsc::Receiver<Vec<u8>>,
    tempfile::TempDir,
) {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;

    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    let (mut master, mut slave) = (0, 0);
    // SAFETY: openpty fills the two descriptors it returns.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty");
    // SAFETY: both are ours from openpty.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let tty = slave.as_raw_fd();
    let size = libc::winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ from a live winsize, on our pty.
    assert_eq!(unsafe { libc::ioctl(tty, libc::TIOCSWINSZ as _, &size) }, 0);
    let mut cmd = Command::new(aspect_cli());
    cmd.args(["history", "browse"])
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::from(slave.try_clone().expect("dup")))
        .stdout(Stdio::from(slave.try_clone().expect("dup")))
        .stderr(Stdio::from(slave.try_clone().expect("dup")));
    // SAFETY: setsid and ioctl are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 || libc::ioctl(tty, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn().expect("spawn");
    drop(cmd);
    drop(slave);
    let (send, got) = std::sync::mpsc::channel();
    let mut screen = std::fs::File::from(master);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = screen.read(&mut buf) {
            let _ = send.send(buf[..n].to_vec());
        }
    });
    (child, got, dir)
}

/// Wait at most `secs` for `child` to exit, killing it past them.
#[cfg(unix)]
fn exit_within(child: &mut std::process::Child, secs: u64) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("aspect history browse ran past {secs}s");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Signalled, the screen leaves with 128 plus the signal's number by the
/// shell's convention: 130 for SIGINT, 143 for SIGTERM and for a hangup.
#[cfg(unix)]
#[test]
fn a_signal_ends_browse_with_its_exit_code() {
    for (signal, want) in [
        (libc::SIGINT, 130),
        (libc::SIGTERM, 143),
        (libc::SIGHUP, 143),
    ] {
        let (mut child, got, _dir) = on_a_pty();
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !String::from_utf8_lossy(&seen).contains("aspect history") {
            assert!(
                std::time::Instant::now() < deadline,
                "no History screen:\n{}",
                String::from_utf8_lossy(&seen)
            );
            if let Ok(chunk) = got.recv_timeout(std::time::Duration::from_millis(200)) {
                seen.extend(chunk);
            }
        }
        // SAFETY: a signal to the child this test started.
        unsafe { libc::kill(child.id() as i32, signal) };
        let status = exit_within(&mut child, 30);
        assert_eq!(status.code(), Some(want), "signal {signal}");
    }
}
