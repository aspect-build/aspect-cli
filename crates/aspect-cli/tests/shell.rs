//! The `aspect` shell, entered only by a bare `aspect` on a terminal: left
//! on purpose (Ctrl+D, Ctrl+C twice at an idle prompt) it exits 0; `"shell":
//! false` in `~/.aspect/config.json` has a bare `aspect` print the help
//! instead; its internal command cannot be typed, and it takes no name from
//! a repository's tasks.

#![cfg(unix)]

mod common;

use common::{aspect_cli, base_env};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A scratch workspace and its `$HOME`, `config` the user's `config.json`
/// when given, `tasks` the repository's `.aspect/tasks.axl` when given.
fn workspace(config: Option<&str>, tasks: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir_all(dir.path().join("home/.aspect")).expect("home");
    if let Some(config) = config {
        std::fs::write(dir.path().join("home/.aspect/config.json"), config).expect("config.json");
    }
    if let Some(tasks) = tasks {
        std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
        std::fs::write(dir.path().join(".aspect/tasks.axl"), tasks).expect("tasks.axl");
    }
    dir
}

/// `aspect <args>` in `dir`, on a pty of its own, its controlling terminal;
/// the child, the pty's master and what the screen got so far.
fn on_a_pty(
    dir: &std::path::Path,
    args: &[&str],
) -> (
    std::process::Child,
    std::fs::File,
    std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
) {
    on_a_pty_sized(dir, args, (100, 30), None)
}

/// [`on_a_pty`] on a pty `size` (columns, rows), stderr to `stderr` when
/// given.
fn on_a_pty_sized(
    dir: &std::path::Path,
    args: &[&str],
    size: (u16, u16),
    stderr: Option<&std::path::Path>,
) -> (
    std::process::Child,
    std::fs::File,
    std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
) {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
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
        ws_row: size.1,
        ws_col: size.0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ from a live winsize, on our pty.
    assert_eq!(unsafe { libc::ioctl(tty, libc::TIOCSWINSZ as _, &size) }, 0);
    let mut cmd = Command::new(aspect_cli());
    cmd.args(args)
        .current_dir(dir)
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.join("history"))
        .env("ASPECT_CLI_CACHE", dir.join("cache"))
        .env("HOME", dir.join("home"))
        .stdin(Stdio::from(slave.try_clone().expect("dup")))
        .stdout(Stdio::from(slave.try_clone().expect("dup")))
        .stderr(match stderr {
            Some(path) => Stdio::from(std::fs::File::create(path).expect("stderr file")),
            None => Stdio::from(slave.try_clone().expect("dup")),
        });
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
    let screen = std::fs::File::from(master);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (mut reader, into) = (screen.try_clone().expect("dup"), seen.clone());
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            into.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    (child, screen, seen)
}

/// Wait at most `secs` for `seen` to hold `text`.
fn wait_for(seen: &std::sync::Mutex<Vec<u8>>, text: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if String::from_utf8_lossy(&seen.lock().unwrap()).contains(text) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// `child`'s exit code, waited for at most `secs`, else killed and failed.
fn exit_within(
    child: &mut std::process::Child,
    secs: u64,
    seen: &std::sync::Mutex<Vec<u8>>,
) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status.code();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "still running:\n{}",
                String::from_utf8_lossy(&seen.lock().unwrap())
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_bare_aspect_opens_the_shell_and_ctrl_d_leaves_it_with_0() {
    let dir = workspace(None, None);
    let (mut child, mut keys, seen) = on_a_pty(dir.path(), &[]);
    assert!(
        wait_for(&seen, "help lists them", 60),
        "{}",
        String::from_utf8_lossy(&seen.lock().unwrap())
    );
    keys.write_all(b"\x04").unwrap();
    assert_eq!(exit_within(&mut child, 30, &seen), Some(0));
}

#[test]
fn ctrl_c_twice_at_an_idle_prompt_leaves_with_0() {
    let dir = workspace(None, None);
    let (mut child, mut keys, seen) = on_a_pty(dir.path(), &[]);
    assert!(wait_for(&seen, "help lists them", 60));
    keys.write_all(b"\x03").unwrap();
    assert!(wait_for(&seen, "Ctrl+C again", 10));
    keys.write_all(b"\x03").unwrap();
    assert_eq!(exit_within(&mut child, 30, &seen), Some(0));
}

#[test]
fn shell_false_in_the_users_config_prints_the_help() {
    let dir = workspace(Some(r#"{"shell": false}"#), None);
    let (mut child, _keys, seen) = on_a_pty(dir.path(), &[]);
    exit_within(&mut child, 60, &seen);
    let out = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
    assert!(out.contains("programmable task runner"), "{out}");
    assert!(!out.contains("help lists them"), "{out}");
}

#[test]
fn its_internal_command_cannot_be_typed() {
    let dir = workspace(None, None);
    let out = Command::new(aspect_cli())
        .arg("__shell")
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .output()
        .expect("aspect __shell");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unrecognized subcommand '__shell'"),
        "{out:?}"
    );
}

#[test]
fn a_repositorys_own_shell_task_is_its_own() {
    let tasks = r#"
def _impl(ctx):
    print("their shell")
    return 0

shell = task(implementation = _impl, summary = "The repository's own shell.")
"#;
    let dir = workspace(None, Some(tasks));
    let out = Command::new(aspect_cli())
        .arg("shell")
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .output()
        .expect("aspect shell");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    assert!(err.contains("their shell"), "{err}");
}

#[test]
fn where_the_shell_would_refuse_a_bare_aspect_prints_the_help() {
    let dir = workspace(None, None);
    let (mut child, _keys, seen) = on_a_pty_sized(dir.path(), &[], (39, 30), None);
    exit_within(&mut child, 60, &seen);
    let out = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
    assert!(
        out.contains("programmable task runner"),
        "too narrow: {out}"
    );
    let (mut child, _keys, seen) = on_a_pty_sized(dir.path(), &[], (100, 11), None);
    exit_within(&mut child, 60, &seen);
    let out = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
    assert!(out.contains("programmable task runner"), "too short: {out}");
    let log = dir.path().join("stderr.log");
    let (mut child, _keys, seen) = on_a_pty_sized(dir.path(), &[], (100, 30), Some(&log));
    let code = exit_within(&mut child, 60, &seen);
    let help = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        help.contains("programmable task runner") && !help.contains("needs a terminal"),
        "stderr not a terminal: {code:?} {help}"
    );
}

#[test]
fn no_suggestion_names_the_shells_internal_command() {
    let dir = workspace(None, None);
    for typed in ["shel", "shell", "_shell"] {
        let out = Command::new(aspect_cli())
            .arg(typed)
            .current_dir(dir.path())
            .env_clear()
            .envs(base_env())
            .env("HOME", dir.path().join("home"))
            .stdin(Stdio::null())
            .output()
            .expect("aspect");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("__shell"), "{typed}: {err}");
    }
}

#[test]
fn a_repository_that_loads_the_shell_task_has_it_as_its_own() {
    let tasks = r#"
load("@aspect//private/task/shell.axl", builtin = "shell")

shell = builtin
"#;
    let dir = workspace(None, Some(tasks));
    let out = Command::new(aspect_cli())
        .arg("version")
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .output()
        .expect("aspect version");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    let help = Command::new(aspect_cli())
        .arg("--help")
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .output()
        .expect("aspect --help");
    let out = String::from_utf8_lossy(&help.stdout);
    assert!(out.contains("shell"), "theirs is listed: {out}");
}

#[test]
fn a_console_config_that_cannot_be_used_does_not_keep_the_shell_from_opening() {
    let dir = workspace(None, None);
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    std::fs::write(
        dir.path().join(".aspect/config.axl"),
        "load(\"@aspect//feature/console.axl\", \"Console\")\n\ndef config(ctx):\n    ctx.features[Console].args.task_presets = {\"build\": \"no-such-preset\"}\n",
    )
    .expect("config.axl");
    let (mut child, mut keys, seen) = on_a_pty(dir.path(), &[]);
    assert!(
        wait_for(&seen, "help lists them", 60),
        "{}",
        String::from_utf8_lossy(&seen.lock().unwrap())
    );
    keys.write_all(b"\x04").unwrap();
    assert_eq!(exit_within(&mut child, 30, &seen), Some(0));
}

/// Left before the `aspect describe` it asked for as it started answers (a
/// slow `config.axl`), the shell takes it down with it.
#[test]
fn leaving_the_shell_stops_the_describe_it_asked_for() {
    let dir = workspace(None, None);
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    let pids = dir.path().join("config-pids");
    std::fs::create_dir_all(&pids).expect("config-pids");
    std::fs::write(
        dir.path().join(".aspect/config.axl"),
        format!(
            "load(\"@std//time.axl\", \"sleep\")\n\ndef config(ctx):\n    ctx.std.fs.write(\"{}/%d\" % ctx.std.process.id(), \"\")\n    if not ctx.std.io.stdout.is_tty:\n        sleep(30000)\n",
            pids.display(),
        ),
    )
    .expect("config.axl");
    let (mut child, mut keys, seen) = on_a_pty(dir.path(), &[]);
    assert!(
        wait_for(&seen, "help lists them", 60),
        "{}",
        String::from_utf8_lossy(&seen.lock().unwrap())
    );
    let shell = child.id() as i32;
    // The describe asked for as the shell started (its stdout a pipe) is
    // still in its slow config.
    keys.write_all(b"\x04").unwrap();
    assert_eq!(exit_within(&mut child, 30, &seen), Some(0));
    std::thread::sleep(Duration::from_millis(300));
    let others: Vec<i32> = std::fs::read_dir(&pids)
        .expect("config-pids")
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().parse().ok())
        .filter(|&pid| pid != shell)
        .collect();
    for pid in &others {
        // SAFETY: probing pids this test's CLI started.
        let alive = unsafe { libc::kill(*pid, 0) } == 0;
        if alive {
            // SAFETY: a signal to a pid this test's CLI started.
            unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
        assert!(!alive, "describe {pid} outlived the shell");
    }
}
