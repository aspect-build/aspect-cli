//! A child a task spawns has its output passed on through the CLI, whole and
//! in order: flushed as it comes, copied to the history's `output.log` a line
//! at a time, so it never lands inside the CLI's own lines, and waited on
//! until it is all passed on. With no terminal, a child marked interactive
//! runs as any other: piped and copied, with no marker lines. A stdout
//! nobody reads stalls neither stderr nor a screen taken meanwhile, and a
//! screen's dump of the task's output is never spliced with a child's.

mod common;

use common::{aspect_cli, base_env};
use std::process::{Command, Output, Stdio};

const TASKS: &str = r#"
load("@std//time.axl", "sleep")

def _partial(ctx):
    ctx.std.process.command("sh").args(["-c", "printf partial; sleep 0.3; printf ' rest\n'; printf tail"]).spawn().wait()
    print("AFTER")
    return 0

def _interactive(ctx):
    ctx.std.process.command("sh").args(["-c", "echo from-the-child"]).interactive(True).spawn().wait()
    return 0

def _takestall(ctx):
    ctx.std.process.command("sh").args(["-c", "head -c 400000 /dev/zero | tr '\\0' x | fold -w 100"]).spawn()
    ticks = ctx.std.process.command("sh").args(["-c", "for i in $(seq 1 40); do echo TICK$i >&2; sleep 0.1; done"]).spawn()
    sleep(500)
    ctx.std.io.stderr.write("TAKING\n")
    screen = ctx.std.term.take()
    screen.write("FRAME")
    sleep(300)
    screen.release()
    ctx.std.io.stderr.write("RELEASED\n")
    ticks.wait()
    return 0

def _dumpsplice(ctx):
    screen = ctx.std.term.take()
    for i in range(400):
        print("DUMPLINE-%d-abcdefghijklmnopqrstuvwxyz0123456789" % i)
    child = ctx.std.process.command("sh").args(["-c", "i=0; while [ $i -lt 20000 ]; do echo CHILDERR$i >&2; i=$((i+1)); done"]).spawn()
    sleep(50)
    screen.release()
    ctx.std.term.dump_output(max_lines = 500, older = "OLDER {n}")
    child.wait()
    return 0

partial = task(implementation = _partial, summary = "A child's partial lines.")
dumpsplice = task(implementation = _dumpsplice, summary = "A dump while a child writes stderr.")
takestall = task(implementation = _takestall, summary = "A screen taken while stdout takes nothing.")
interactive = task(implementation = _interactive, summary = "An interactive child.")
"#;

/// Run `aspect <task>` in a scratch workspace with stdin and stdout not a
/// terminal; its output and the history's `output.log`.
fn run(task: &str) -> (Output, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("tasks.axl"), TASKS).expect("tasks.axl");
    let history = dir.path().join("history");
    let output = Command::new(aspect_cli())
        .arg(task)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("ASPECT_HISTORY_DIR", &history)
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("running `aspect {task}`: {e}"));
    let log = std::fs::read_dir(&history)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .map(|e| e.path().join("output.log"))
                .find(|p| p.exists())
        })
        .map(|p| std::fs::read_to_string(p).expect("output.log"))
        .unwrap_or_default();
    (output, log)
}

/// The position of `needle` in `text`, failing with both when it is not there.
fn at(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("want {needle:?} in:\n{text}"))
}

#[test]
fn a_childs_partial_lines_are_passed_on_whole_and_in_order() {
    let (output, log) = run("partial");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    // Its stdout is the CLI's stdout (here not stderr's file), all of it.
    let shown = String::from_utf8_lossy(&output.stdout);
    assert_eq!(shown, "partial rest\ntail", "the child's stdout, all of it");
    assert!(
        at(&log, "partial rest\n") < at(&log, "tail\n") && at(&log, "tail\n") < at(&log, "AFTER"),
        "output.log keeps the child's lines whole, the CLI's after:\n{log}"
    );
}

#[test]
fn without_a_terminal_an_interactive_child_is_piped_and_unmarked() {
    let (output, log) = run("interactive");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(log.contains("from-the-child\n"), "copied:\n{log}");
    assert!(
        !log.contains("interactively"),
        "no marker lines without a terminal:\n{log}"
    );
}

/// `aspect <task>` in a scratch workspace, in a session of its own on a pty
/// of 120x40, its terminal and its stderr, with `stdout`; what it writes to
/// the terminal, a chunk at a time with when it came.
#[cfg(unix)]
fn on_a_pty(
    task: &str,
    stdout: Stdio,
) -> (
    std::process::Child,
    std::sync::mpsc::Receiver<(std::time::Duration, Vec<u8>)>,
    tempfile::TempDir,
) {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;

    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("MODULE.aspect"), "").expect("MODULE.aspect");
    let aspect = dir.path().join(".aspect");
    std::fs::create_dir(&aspect).expect(".aspect");
    std::fs::write(aspect.join("tasks.axl"), TASKS).expect("tasks.axl");

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
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ from a live winsize, on our pty.
    assert_eq!(
        unsafe { libc::ioctl(tty, libc::TIOCSWINSZ as _, &size) },
        0,
        "winsize"
    );
    let mut cmd = Command::new(aspect_cli());
    cmd.arg(task)
        .current_dir(dir.path())
        .env_clear()
        .envs(base_env())
        .env("ASPECT_CLI_CACHE", dir.path().join("cache"))
        .env("ASPECT_HISTORY_DIR", dir.path().join("history"))
        .env("HOME", dir.path().join("home"))
        .stdin(Stdio::from(slave.try_clone().expect("dup")))
        .stdout(stdout)
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
    let start = std::time::Instant::now();
    let (send, chunks) = std::sync::mpsc::channel();
    let mut screen = std::fs::File::from(master);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = screen.read(&mut buf) {
            let _ = send.send((start.elapsed(), buf[..n].to_vec()));
        }
    });
    (child, chunks, dir)
}

/// Taking the screen, and giving it back, while a child's stdout fills a
/// FIFO nobody reads: neither may wait on stdout, or stderr's lines, fed to
/// the screen, stop with it (macOS's `dup2` over a FIFO waits out a write in
/// flight on it, and a forwarder's write to one nobody reads blocks). Stdout
/// is read only once stderr's last tick is in: a tick every 100ms, none of
/// them held up, and the screen given back before then.
#[cfg(unix)]
#[test]
fn a_stdout_nobody_reads_stalls_no_take_nor_stderr() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::Duration;

    let fifos = tempfile::tempdir().expect("temp dir");
    let fifo = fifos.path().join("stdout.fifo");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: mkfifo on a path of our own temp dir.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0, "mkfifo");
    // The read end first, not blocking, so the write end opens at once.
    let mut reader = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo)
        .expect("fifo reader");
    let writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&fifo)
        .expect("fifo writer");
    let (mut child, chunks, _dir) = on_a_pty("takestall", Stdio::from(writer));

    // When each of `TICK1`..`TICK40`, `TAKING` and `RELEASED` first shows on
    // the terminal, the screen's frames around them.
    let (mut text, mut got) = (String::new(), Vec::<(Duration, String)>::new());
    // A tick with its line end, so `TICK1` is not found in `TICK10`.
    let names: Vec<(String, String)> = (1..=40)
        .map(|i| (format!("TICK{i}"), format!("TICK{i}\r")))
        .chain(["TAKING", "RELEASED"].map(|n| (n.to_string(), n.to_string())))
        .collect();
    let mut see = |at: Duration, bytes: &[u8], got: &mut Vec<(Duration, String)>| {
        text.push_str(&String::from_utf8_lossy(bytes));
        for (name, needle) in &names {
            if !got.iter().any(|(_, l)| l == name) && text.contains(needle.as_str()) {
                got.push((at, name.clone()));
            }
        }
    };
    let start = std::time::Instant::now();
    while !got.iter().any(|(_, l)| l == "TICK40") {
        match chunks.recv_timeout(Duration::from_secs(20).saturating_sub(start.elapsed())) {
            Ok((at, bytes)) => see(at, &bytes, &mut got),
            Err(_) => break,
        }
    }
    let read_from = start.elapsed();
    // SAFETY: clearing O_NONBLOCK on our own descriptor.
    unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, 0) };
    let drain = std::thread::spawn(move || std::io::copy(&mut reader, &mut std::io::sink()));
    let status = child.wait().expect("wait");
    let _ = drain.join();
    std::thread::sleep(Duration::from_millis(200));
    for (at, bytes) in chunks.try_iter() {
        see(at, &bytes, &mut got);
    }
    assert!(status.success(), "{status:?}: {got:?}");
    let mut ticks: Vec<(usize, Duration)> = got
        .iter()
        .filter_map(|(t, l)| Some((l.strip_prefix("TICK")?.parse().ok()?, *t)))
        .collect();
    ticks.sort();
    assert_eq!(
        ticks.len(),
        40,
        "every tick before stdout was read: {got:?}"
    );
    let gap = ticks
        .windows(2)
        .map(|w| w[1].1.saturating_sub(w[0].1))
        .max()
        .unwrap();
    assert!(
        gap < Duration::from_secs(1),
        "a tick held up {gap:?}: {got:?}"
    );
    let at = |want: &str| got.iter().find(|(_, l)| l == want).map(|(t, _)| *t);
    assert!(
        at("TAKING") < at("RELEASED") && at("RELEASED").is_some_and(|t| t < read_from),
        "taken and given back before stdout was read at {read_from:?}: {got:?}"
    );
}

/// A screen's dump of the task's output, written while a child floods
/// stderr, on the terminal and with stdout redirected (the exit's path):
/// every line whole, the dump's and the child's never spliced.
#[cfg(unix)]
#[test]
fn a_dump_is_never_spliced_with_a_childs_stderr() {
    use std::time::Duration;
    for stdout in [Stdio::inherit(), Stdio::null()] {
        let (mut child, chunks, _dir) = on_a_pty("dumpsplice", stdout);
        let status = child.wait().expect("wait");
        std::thread::sleep(Duration::from_millis(300));
        let mut text = String::new();
        for (_, bytes) in chunks.try_iter() {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
        assert!(status.success(), "{status:?}");
        let mut torn = vec![];
        for line in plain(&text).split('\n') {
            let line = line.trim_end_matches('\r');
            let line = line.rsplit('\r').next().unwrap_or(line);
            let dump = line
                .strip_prefix("DUMPLINE-")
                .and_then(|r| r.split_once('-'))
                .is_some_and(|(n, rest)| {
                    n.parse::<u32>().is_ok() && rest == "abcdefghijklmnopqrstuvwxyz0123456789"
                });
            let child = line
                .strip_prefix("CHILDERR")
                .is_some_and(|n| n.parse::<u32>().is_ok());
            if (line.contains("DUMPLINE") || line.contains("CHILDERR")) && !(dump || child) {
                torn.push(line.to_string());
            }
        }
        assert!(
            text.contains("CHILDERR19999") && torn.is_empty(),
            "{} torn lines, the first: {:?}",
            torn.len(),
            torn.iter().take(3).collect::<Vec<_>>()
        );
    }
}

/// `text` without its CSI (`ESC [ ... final`) and OSC (`ESC ] ... BEL or ST`)
/// sequences.
#[cfg(unix)]
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || c == '\x1b' && chars.next_if_eq(&'\\').is_some() {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}
