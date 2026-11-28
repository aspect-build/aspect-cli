//! `aspect history browse` off a terminal: refused with the error that names
//! `aspect history list --output=json`, no verdict line after it, no run of
//! its own kept in the history, and no `--mouse` flag. On a terminal, a
//! signal ends it with the shell's code for that signal. `aspect history list`
//! and `show` print only what they show, and say when a newer aspect wrote a
//! run they cannot read, or its record does not read; `list --workspace`
//! takes a root however it is spelled.

mod common;

use common::{aspect_cli, base_env};
use std::process::{Command, Output, Stdio};

/// Run `aspect history <args>` in a scratch workspace with piped stdio and
/// a history folder of its own, returned for a look at what it kept.
fn run(args: &[&str]) -> (Output, tempfile::TempDir) {
    run_with(args, |_| {})
}

/// [`run`], `seed` first given the history folder to put entries in.
fn run_with(args: &[&str], seed: impl FnOnce(&std::path::Path)) -> (Output, tempfile::TempDir) {
    run_in(true, args, seed)
}

/// [`run_with`], in a workspace (a `MODULE.aspect`) or, without
/// `workspace`, in a folder that is in none.
fn run_in(
    workspace: bool,
    args: &[&str],
    seed: impl FnOnce(&std::path::Path),
) -> (Output, tempfile::TempDir) {
    let dir = scratch(workspace, seed);
    (history_in(dir.path(), args), dir)
}

/// A scratch folder, a workspace (a `MODULE.aspect`) or not, with a history
/// folder of its own, `seed` given it to put entries in.
fn scratch(workspace: bool, seed: impl FnOnce(&std::path::Path)) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    if workspace {
        std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    }
    std::fs::create_dir(dir.path().join("history")).expect("history");
    seed(&dir.path().join("history"));
    dir
}

/// `aspect history <args>` run in `dir` ([`scratch`]), with piped stdio.
fn history_in(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(aspect_cli())
        .arg("history")
        .args(args)
        .current_dir(dir)
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.join("history"))
        .env("HOME", dir.join("home"))
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("running `aspect history {}`: {e}", args.join(" ")))
}

#[test]
fn browse_off_a_terminal_is_refused_quietly_and_keeps_no_run() {
    let (output, dir) = run(&["browse"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(
            "aspect history browse needs a terminal on stdin and stdout; for a script: aspect history list --output=json"
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

/// An entry whose record a newer aspect wrote, of a schema this one skips.
const NEWER: &str = "20261101T000000000Z-00000000-0000-0000-0000-000000000005";

fn seed_newer(history: &std::path::Path) {
    let entry = history.join(NEWER);
    std::fs::create_dir(&entry).expect("entry");
    std::fs::write(
        entry.join("task.json"),
        r#"{"schema": 999, "cli_version": "2099.1.0", "status": "ended"}"#,
    )
    .expect("task.json");
}

#[test]
fn show_names_a_run_a_newer_aspect_wrote() {
    let (output, dir) = run_with(
        &["show", "00000000-0000-0000-0000-000000000005"],
        seed_newer,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    let want = format!(
        "run 00000000-0000-0000-0000-000000000005 was written by a newer aspect (2099.1.0); upgrade to read it · files: {}",
        dir.path().join("history").join(NEWER).display()
    );
    assert!(stderr.contains(&want), "{stderr}");
}

#[test]
fn list_counts_runs_a_newer_aspect_wrote_and_prints_only_its_list() {
    let (output, _dir) = run_with(&["list", "--builds", "--workspace=."], seed_newer);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert!(
        stderr.contains("1 run from a newer aspect not listed (upgrade to read it)"),
        "{stderr}"
    );
    assert!(
        stdout.starts_with("No runs match in the history at "),
        "{stdout}"
    );
    assert!(
        !stderr.contains("history list task"),
        "no verdict line after list:\n{stderr}"
    );
}

/// A run of `aspect build` in the scratch workspace (the history folder's
/// parent, its links resolved, as the CLI records its root).
fn seed_run(history: &std::path::Path) {
    let workspace = std::fs::canonicalize(history.parent().unwrap()).expect("workspace");
    let entry = history.join("20261102T000000000Z-00000000-0000-0000-0000-000000000006");
    std::fs::create_dir(&entry).expect("entry");
    let record = format!(
        r#"{{"schema": 1, "status": "ended", "cli_version": "", "task": {{"id": "00000000-0000-0000-0000-000000000006", "name": "build", "kind": "build"}}, "pid": 1, "argv": ["aspect", "build", "//..."], "argv_redacted": true, "cwd": {ws:?}, "workspace": {ws:?}, "started": "2026-11-02T00:00:00.000Z", "ended": "2026-11-02T00:00:01.000Z", "duration_ms": 1000, "exit_code": 0}}"#,
        ws = workspace.to_string_lossy()
    );
    std::fs::write(entry.join("task.json"), record).expect("task.json");
}

#[test]
fn list_takes_a_workspace_however_it_is_spelled() {
    let dir = scratch(true, seed_run);
    let links = tempfile::tempdir().expect("temp dir");
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut spelled = vec![
        ".".to_owned(),
        format!("{}/", dir.path().display()),
        format!("../{name}/"),
    ];
    #[cfg(unix)]
    {
        let link = links.path().join("ws");
        std::os::unix::fs::symlink(dir.path(), &link).expect("link");
        spelled.push(link.display().to_string());
    }
    for workspace in spelled {
        let output = history_in(dir.path(), &["list", &format!("--workspace={workspace}")]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains("00000000"),
            "--workspace={workspace}: {stdout}{stderr}"
        );
    }
    for filter in ["--workspace=/nowhere", "--kind=test"] {
        let output = history_in(dir.path(), &["list", filter]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.starts_with("No runs match in the history at "),
            "{filter}: {stdout}"
        );
    }
}

#[test]
fn list_counts_newer_runs_only_among_those_it_went_through() {
    let dir = scratch(true, |h| {
        seed_newer(h);
        seed_run(h);
    });
    let note = "1 run from a newer aspect not listed";
    let full = history_in(dir.path(), &["list", "--limit=1"]);
    let stderr = String::from_utf8_lossy(&full.stderr);
    assert!(
        !stderr.contains(note),
        "an older run past the limit: {stderr}"
    );
    let all = history_in(dir.path(), &["list"]);
    let stderr = String::from_utf8_lossy(&all.stderr);
    assert!(stderr.contains(note), "{stderr}");
}

#[test]
fn list_of_this_workspace_outside_one_is_an_error() {
    let (output, _dir) = run_in(false, &["list", "--workspace=."], seed_run);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("--workspace=. names the workspace you are in, and "),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn show_says_a_record_does_not_read() {
    let entry = "20261103T000000000Z-00000000-0000-0000-0000-000000000007";
    let (output, dir) = run_with(&["show", "00000000-0000-0000-0000-000000000007"], |h| {
        std::fs::create_dir(h.join(entry)).expect("entry");
        std::fs::write(h.join(entry).join("task.json"), "not json").expect("task.json");
    });
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    let want = format!(
        "run 00000000-0000-0000-0000-000000000007's record does not read (task.json damaged, or not written yet) · files: {}",
        dir.path().join("history").join(entry).display()
    );
    assert!(stderr.contains(&want), "{stderr}");
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
