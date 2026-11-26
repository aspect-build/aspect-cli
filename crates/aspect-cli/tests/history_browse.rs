//! `aspect history browse` off a terminal: refused with the error that names
//! `aspect history list`, no verdict line after it, no run of its own kept
//! in the history, and no `--mouse` flag. On a terminal, a signal ends it
//! with the shell's code for that signal. A run it starts again as its
//! bridged child keeps its output in order, stops once the browser is gone
//! or no longer reads it (or was before it started), is never asked twice,
//! and passes none of that on to what it starts.

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

/// A task run again by the History screen (`ASPECT_GROUP_SIGNALLER` naming
/// its parent, the browser) writes its stderr to its stdout, in order, and
/// stops on its own once the browser is gone, rather than run on unwatched.
#[cfg(unix)]
#[test]
fn a_bridged_child_keeps_its_order_and_stops_without_its_browser() {
    use std::os::unix::process::CommandExt;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    std::fs::write(
        dir.path().join(".aspect/sleeper.axl"),
        r#"load("@std//time.axl", "sleep")

def _impl(ctx):
    for i in range(3):
        print("err %d" % i)
        ctx.std.io.stdout.write("out %d\n" % i)
    for _ in range(600):
        if ctx.cancellation.root.cancelled:
            return 130
        sleep(100)
    return 0

sleeper = task(implementation = _impl, summary = "x")
"#,
    )
    .expect("sleeper.axl");
    // The browser stands in as a shell that starts the child, names itself
    // and exits once the child has written its lines; with job control on (`set -m`), so the child is in a group
    // of its own with SIGINT not ignored, as the browser's child is.
    let script = r#"set -m; export ASPECT_GROUP_SIGNALLER=$$; "$0" sleeper > out.txt 2> err.txt & echo $!; for _ in $(seq 300); do grep -q 'out 2' out.txt 2>/dev/null && break; sleep 0.1; done"#;
    let output = Command::new("/bin/sh")
        .args(["-c", script, &aspect_cli()])
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .process_group(0)
        .output()
        .expect("sh");
    let pid: i32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("the child's pid");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    // SAFETY: probing a pid this test started.
    while unsafe { libc::kill(pid, 0) } == 0 {
        if std::time::Instant::now() >= deadline {
            // SAFETY: a signal to the child this test started.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            panic!("the child ran on after its browser was gone");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap();
    let err = std::fs::read_to_string(dir.path().join("err.txt")).unwrap();
    let at = |s: &str| {
        out.find(s)
            .unwrap_or_else(|| panic!("{s:?} not in:\n{out}"))
    };
    assert!(
        at("err 0") < at("out 0") && at("out 0") < at("err 1") && at("err 1") < at("out 1"),
        "{out}"
    );
    assert!(!err.contains("err 0"), "stderr went to stdout: {err}");
    assert!(
        out.contains("interrupted"),
        "stopped as by an interrupt:\n{out}"
    );
    assert!(
        out.contains("■ Stopped sleeper task") && !out.contains("(by you)"),
        "a stop it asked itself is not the user's:\n{out}"
    );
}

/// What a bridged child starts (a nested CLI, or the credential helper Bazel
/// runs with the client's environment) is no bridged child of its own: the
/// browser's variables (its pid, its screen's size) are gone from its
/// environment, and a CLI that has it
/// with some other process as its parent ignores it, so neither stops itself
/// nor writes its stderr to its stdout.
#[cfg(unix)]
#[test]
fn what_a_bridged_child_starts_is_not_bridged() {
    use std::os::unix::process::CommandExt;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    std::fs::write(
        dir.path().join(".aspect/nested.axl"),
        r#"load("@std//time.axl", "sleep")

def _outer(ctx):
    script = 'echo "sig=$ASPECT_GROUP_SIGNALLER cols=$COLUMNS rows=$LINES" > inner.txt; "$1" inner 2> inner_err.txt; echo "inner=$?" >> inner.txt'
    return ctx.std.process.command("sh").args(["-c", script, "sh", ctx.std.env.current_exe()]).spawn().wait().code

def _inner(ctx):
    sleep(1000)
    print("inner err")
    return 0

outer = task(implementation = _outer, summary = "x")
inner = task(implementation = _inner, summary = "x")
"#,
    )
    .expect("nested.axl");
    let cli = || {
        let mut cmd = Command::new(aspect_cli());
        cmd.current_dir(dir.path())
            .env_clear()
            .envs(base_env())
            .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
            .env("HOME", dir.path().join("home"))
            .stdin(Stdio::null());
        cmd
    };

    let output = Command::new("/bin/sh")
        .args([
            "-c",
            r#"set -m; export ASPECT_GROUP_SIGNALLER=$$ COLUMNS=91 LINES=17; "$0" outer > out.txt 2>&1 & wait $!"#,
            &aspect_cli(),
        ])
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .process_group(0)
        .output()
        .expect("sh");
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap_or_default();
    let inner = std::fs::read_to_string(dir.path().join("inner.txt")).unwrap_or_default();
    assert_eq!(output.status.code(), Some(0), "{out}\n{inner}");
    assert!(
        inner.contains("sig= cols= rows=\n"),
        "the browser's variables reached it: {inner}"
    );
    assert!(inner.contains("inner=0"), "{inner}\n{out}");

    // The Bazel server keeps the environment of the client that started it,
    // so its credential helper can have the variable with another parent.
    let output = cli()
        .arg("inner")
        .env("ASPECT_GROUP_SIGNALLER", "1")
        .output()
        .expect("inner");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("inner err"), "{stderr}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("inner err"),
        "its stderr stays its own"
    );
}

/// A bridged child whose browser no longer reads it (its end of the pipe
/// closed, the browser itself still its parent) stops on its own too.
#[cfg(unix)]
#[test]
fn a_bridged_child_stops_once_nothing_reads_it() {
    use std::os::unix::process::CommandExt;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    std::fs::write(
        dir.path().join(".aspect/sleeper.axl"),
        r#"load("@std//time.axl", "sleep")

def _impl(ctx):
    for _ in range(600):
        if ctx.cancellation.root.cancelled:
            return 130
        sleep(100)
    return 0

sleeper = task(implementation = _impl, summary = "x")
"#,
    )
    .expect("sleeper.axl");
    let started = std::time::Instant::now();
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            r#"set -m; export ASPECT_GROUP_SIGNALLER=$$; "$0" sleeper | true; wait"#,
            &aspect_cli(),
        ])
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .process_group(0)
        .output()
        .expect("sh");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "the child ran on unread: {output:?}"
    );
}

/// A bridged child already asked to stop (the browser's `q`, one SIGINT to
/// its group) whose browser then goes is not asked again: a second would be
/// the hard stop, ending the run before it is done stopping.
#[cfg(unix)]
#[test]
fn a_bridged_child_asked_to_stop_is_not_asked_again() {
    use std::os::unix::process::CommandExt;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    std::fs::write(
        dir.path().join(".aspect/slow_stop.axl"),
        r#"def _impl(ctx):
    script = 'trap "sleep 1; exit 0" INT; echo up > up.txt; while :; do sleep 0.1; done'
    ctx.std.process.command("sh").args(["-c", script]).spawn().wait()
    ctx.std.fs.write("done.txt", "done")
    return 130

slow_stop = task(implementation = _impl, summary = "x")
"#,
    )
    .expect("slow_stop.axl");
    let script = r#"set -m; export ASPECT_GROUP_SIGNALLER=$$; "$0" slow-stop > out.txt 2>&1 & p=$!; echo $p; for _ in $(seq 300); do [ -f up.txt ] && break; sleep 0.1; done; kill -INT -$p; sleep 0.3"#;
    let output = Command::new("/bin/sh")
        .args(["-c", script, &aspect_cli()])
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .process_group(0)
        .output()
        .expect("sh");
    let pid: i32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("the child's pid");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    // SAFETY: probing a pid this test started.
    while unsafe { libc::kill(pid, 0) } == 0 {
        if std::time::Instant::now() >= deadline {
            // SAFETY: a signal to the group this test started.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            panic!("the child did not stop");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap_or_default();
    assert!(
        dir.path().join("done.txt").exists(),
        "it was stopped hard:\n{out}"
    );
    assert!(
        out.contains("■ Stopped slow-stop task (by you)"),
        "the browser's stop is the user's:\n{out}"
    );
}

/// A bridged child whose browser was gone before it started (its parent
/// already init) stops at once rather than run to its end unwatched.
#[cfg(unix)]
#[test]
fn a_bridged_child_whose_browser_is_already_gone_stops() {
    use std::os::unix::process::CommandExt;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    std::fs::write(
        dir.path().join(".aspect/sleeper.axl"),
        r#"load("@std//time.axl", "sleep")

def _impl(ctx):
    ctx.std.fs.write("started.txt", "")
    for _ in range(600):
        if ctx.cancellation.root.cancelled:
            return 130
        sleep(100)
    return 0

sleeper = task(implementation = _impl, summary = "x")
"#,
    )
    .expect("sleeper.axl");
    // The child execs the CLI only after the browser, its parent, exited.
    let script = r#"set -m; export ASPECT_GROUP_SIGNALLER=$$; sh -c 'sleep 0.5; exec "$0" sleeper' "$0" > out.txt 2>&1 & echo $!"#;
    let output = Command::new("/bin/sh")
        .args(["-c", script, &aspect_cli()])
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .process_group(0)
        .output()
        .expect("sh");
    let pid: i32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("the child's pid");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    // SAFETY: probing a pid this test started.
    while unsafe { libc::kill(pid, 0) } == 0 {
        if std::time::Instant::now() >= deadline {
            // SAFETY: a signal to the group this test started.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap_or_default();
            panic!("the child ran on unwatched:\n{out}");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// A run given a `--task:id` (one `R` made, or one typed, in either form)
/// runs again with `r` and with `R`, each time as a new task of its own:
/// the old ID is left out, as the CLI refuses an ID already used.
#[cfg(unix)]
#[test]
fn a_run_with_a_task_id_runs_again_with_a_new_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    std::fs::write(
        dir.path().join(".aspect/again.axl"),
        r#"load("@aspect//private/lib/history_child.axl", "again_command", "new_task_id")

def _hi(ctx):
    return 0

def _again(ctx):
    history = ctx.std.history
    state = {"browser": {"history": history}}
    for n, old in enumerate(["11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222"]):
        record = history.get(old)
        if record == None:
            fail("no record " + old)
        before = len(history.list(limit = 100))
        r = again_command(state, record, True).stdout("null").stderr("null").spawn().wait()
        if r.code != 0:
            fail("r of %s exited %r" % (old, r.code))
        given = new_task_id(ctx.std.process.id(), 1795365737634, n)
        big_r = again_command(state, record, False, task_id = given).stdout("null").stderr("null").spawn().wait()
        if big_r.code != 0:
            fail("R of %s exited %r" % (old, big_r.code))
        if history.get(given) == None:
            fail("R's run not found by its ID")
        if len(history.list(limit = 100)) != before + 2:
            fail("not two new runs")
    return 0

hi = task(implementation = _hi, summary = "x")
again = task(implementation = _again, summary = "x")
"#,
    )
    .expect("again.axl");
    let cli = |args: &[&str]| {
        let out = Command::new(aspect_cli())
            .args(args)
            .current_dir(dir.path())
            .env_clear()
            .envs(base_env())
            .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
            .env("HOME", dir.path().join("home"))
            .stdin(Stdio::null())
            .output()
            .expect("aspect");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.status.code(), Some(0), "aspect {args:?}:\n{stderr}");
    };
    cli(&["hi", "--task:id=11111111-1111-4111-8111-111111111111"]);
    cli(&["hi", "--task:id", "22222222-2222-4222-8222-222222222222"]);
    cli(&["again"]);
}

/// A run the browser stopped that then ends in an error is the error's:
/// its record says so, and never that the user stopped it.
#[cfg(unix)]
#[test]
fn a_stopped_run_that_ends_in_an_error_is_not_recorded_as_stopped_by_the_user() {
    use std::os::unix::process::CommandExt;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    std::fs::create_dir_all(dir.path().join(".aspect")).expect(".aspect");
    std::fs::create_dir(dir.path().join("history")).expect("history");
    std::fs::write(
        dir.path().join(".aspect/errer.axl"),
        r#"load("@std//time.axl", "sleep")

def _impl(ctx):
    ctx.std.fs.write("up.txt", "")
    for _ in range(600):
        if ctx.cancellation.root.cancelled:
            fail("the build it waited on was interrupted")
        sleep(50)
    return 0

errer = task(implementation = _impl, summary = "x")
"#,
    )
    .expect("errer.axl");
    let script = r#"set -m; export ASPECT_GROUP_SIGNALLER=$$; "$0" errer > out.txt 2>&1 & p=$!; for _ in $(seq 300); do [ -f up.txt ] && break; sleep 0.1; done; kill -INT -$p; wait $p"#;
    let output = Command::new("/bin/sh")
        .args(["-c", script, &aspect_cli()])
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .process_group(0)
        .output()
        .expect("sh");
    let out = std::fs::read_to_string(dir.path().join("out.txt")).unwrap_or_default();
    let record = std::fs::read_dir(dir.path().join("history"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("task.json"))
        .find(|p| p.exists())
        .map(|p| std::fs::read_to_string(p).unwrap())
        .unwrap_or_default();
    assert!(record.contains("\"error\""), "{output:?}\n{out}\n{record}");
    assert!(!record.contains("stopped_by_user"), "{out}\n{record}");
}
