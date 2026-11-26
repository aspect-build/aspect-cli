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
    child = ctx.std.process.command("sh").args(["-c", "i=0; while [ $i -lt 20000 ]; do echo CHILDERR$i-abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz >&2; i=$((i+1)); done"]).spawn()
    sleep(50)
    screen.release()
    ctx.std.term.dump_output(max_lines = 500, older = "OLDER {n}")
    child.wait()
    return 0

partial = task(implementation = _partial, summary = "A child's partial lines.")
dumpsplice = task(implementation = _dumpsplice, summary = "A dump while a child writes stderr.")

def _tstpdump(ctx):
    screen = ctx.std.term.take()
    screen.write("FRAME")
    for i in range(400):
        print("DUMPLINE-%d-abcdefghijklmnopqrstuvwxyz0123456789" % i)
    for i in range(60000):
        print("CLILINE-%d-abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz" % i)
    screen.release()
    return 0

tstpdump = task(implementation = _tstpdump, summary = "Ctrl+Z's dump while the task prints.")

def _swapstall(ctx):
    big = ctx.std.process.command("sh").args(["-c", "awk 'BEGIN{for(i=0;i<4000;i++) printf \"N%06d-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n\", i}'"]).spawn()
    sleep(300)
    screen = ctx.std.term.take()
    screen.write("FRAME")
    sleep(300)
    screen.release()
    ctx.std.io.stderr.write("RELEASED\n")
    st = big.wait()
    ctx.std.io.stderr.write("WAITED-%s\n" % st.code)
    return 0

swapstall = task(implementation = _swapstall, summary = "A screen taken and given back while stdout takes nothing.")

def _inhtstp(ctx):
    screen = ctx.std.term.take()
    screen.write("FRAME")
    child = ctx.std.process.command("sh").args(["-c", "i=0; while [ $i -lt 30 ]; do echo INH$i; i=$((i+1)); sleep 0.1; done"]).stdout("inherit").spawn()
    sleep(4000)
    screen.release()
    child.wait()
    return 0

inhtstp = task(implementation = _inhtstp, summary = "A child given the CLI's stdout under a screen, across Ctrl+Z.")

def _tstpstall(ctx):
    screen = ctx.std.term.take()
    screen.write("FRAME")
    for i in range(20000):
        ctx.std.io.stdout.write("OUTLINE-%d-abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz\n" % i)
    screen.release()
    return 0

tstpstall = task(implementation = _tstpstall, summary = "Ctrl+Z while a release replays to a stdout nobody reads.")

def _batches(ctx):
    return ctx.std.process.command("sh").args(["-c", "i=0; while [ $i -lt 100 ]; do awk -v b=$i 'BEGIN{for(j=0;j<30;j++) printf \"N%06d-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n\", b*30+j}'; i=$((i+1)); sleep 0.05; done"]).stdout("inherit").spawn()

def _retake(ctx):
    a = ctx.std.term.take()
    a.write("FRAME-A")
    child = _batches(ctx)
    sleep(300)
    a.release()
    sleep(1500)
    b = ctx.std.term.take()
    b.write("FRAME-B")
    sleep(2500)
    b.release()
    ctx.std.io.stderr.write("WAITED-%s\n" % child.wait().code)
    return 0

retake = task(implementation = _retake, summary = "A child given the CLI's stdout across two screens.")

def _fgslow(ctx):
    a = ctx.std.term.take()
    a.write("FRAME-A")
    child = _batches(ctx)
    sleep(7000)
    a.release()
    ctx.std.io.stderr.write("WAITED-%s\n" % child.wait().code)
    return 0

fgslow = task(implementation = _fgslow, summary = "A child given the CLI's stdout across Ctrl+Z.")
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
/// of 120x40, its terminal and its stderr, with `stdout` (the pty too when
/// None); what it writes to
/// the terminal, a chunk at a time with when it came.
#[cfg(unix)]
fn on_a_pty(
    task: &str,
    stdout: Option<Stdio>,
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
        .stdout(stdout.unwrap_or_else(|| Stdio::from(slave.try_clone().expect("dup"))))
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

/// Wait at most `secs` for `child`; on a timeout, fail with what it and its
/// children are doing ([`common::describe`], Linux) and what it wrote to the
/// terminal, `chunks` drained, after killing it and every process under it.
#[cfg(unix)]
fn wait_bounded(
    child: &mut std::process::Child,
    secs: u64,
    chunks: &std::sync::mpsc::Receiver<(std::time::Duration, Vec<u8>)>,
) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        if std::time::Instant::now() >= deadline {
            let doing = common::describe(child.id());
            for pid in common::descendants(child.id()) {
                // SAFETY: SIGKILL to processes this test started.
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
            let _ = child.kill();
            let _ = child.wait();
            let wrote: Vec<u8> = chunks.try_iter().flat_map(|(_, b)| b).collect();
            let wrote = String::from_utf8_lossy(&wrote);
            let tail: String = wrote
                .chars()
                .rev()
                .take(2000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            panic!("the CLI ran past {secs}s, killed:\n{doing}\nits last output:\n{tail}");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A FIFO in `dir`: its read end (not read until the caller does) and its
/// write end, for a CLI's stdout nobody reads yet.
#[cfg(unix)]
fn unread_fifo(dir: &std::path::Path) -> (std::fs::File, std::fs::File) {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let fifo = dir.join("stdout.fifo");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: mkfifo on a path of the caller's temp dir.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0, "mkfifo");
    // The read end first, not blocking, so the write end opens at once.
    let reader = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo)
        .expect("fifo reader");
    let writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&fifo)
        .expect("fifo writer");
    // SAFETY: clearing O_NONBLOCK on our own descriptor, for the read later.
    unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, 0) };
    (reader, writer)
}

/// The task's `output.log` under a scratch workspace's history.
#[cfg(unix)]
fn output_log(dir: &std::path::Path) -> String {
    std::fs::read_dir(dir.join("history"))
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .map(|e| e.path().join("output.log"))
                .find(|p| p.exists())
        })
        .map(|p| String::from_utf8_lossy(&std::fs::read(p).expect("output.log")).into_owned())
        .unwrap_or_default()
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
    use std::time::Duration;

    let fifos = tempfile::tempdir().expect("temp dir");
    let (mut reader, writer) = unread_fifo(fifos.path());
    let (mut child, chunks, _dir) = on_a_pty("takestall", Some(Stdio::from(writer)));

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
    let drain = std::thread::spawn(move || std::io::copy(&mut reader, &mut std::io::sink()));
    let status = wait_bounded(&mut child, 60, &chunks);
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
    for stdout in [None, Some(Stdio::null())] {
        let (mut child, chunks, _dir) = on_a_pty("dumpsplice", stdout);
        let status = wait_bounded(&mut child, 60, &chunks);
        std::thread::sleep(Duration::from_millis(300));
        let mut text = String::new();
        for (_, bytes) in chunks.try_iter() {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
        assert!(status.success(), "{status:?}");
        let torn = torn_lines(&text);
        assert!(
            text.contains(&format!("CHILDERR19999-{LONG}")) && torn.is_empty(),
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

/// The tail of every long line the tests write: long enough that a line
/// takes more than one of a forwarder's reads now and then.
#[cfg(unix)]
const LONG: &str = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";

/// The lines of `text` (its escapes and the terminal's `^Z` echo left out)
/// that hold a dump's, a child's or the CLI's test line, but not one whole.
#[cfg(unix)]
fn torn_lines(text: &str) -> Vec<String> {
    let numbered = |line: &str, prefix: &str, tail: &str| {
        line.strip_prefix(prefix)
            .and_then(|r| r.split_once('-'))
            .is_some_and(|(n, rest)| n.parse::<u32>().is_ok() && rest == tail)
    };
    let mut torn = vec![];
    for line in plain(text).replace("^Z", "").split('\n') {
        let line = line.trim_end_matches('\r');
        let line = line.rsplit('\r').next().unwrap_or(line);
        let line = line.strip_prefix("FRAME").unwrap_or(line);
        let whole = numbered(line, "DUMPLINE-", "abcdefghijklmnopqrstuvwxyz0123456789")
            || numbered(line, "CHILDERR", LONG)
            || numbered(line, "CLILINE-", LONG);
        let ours = ["DUMPLINE", "CHILDERR", "CLILINE"]
            .iter()
            .any(|p| line.contains(p));
        if ours && !whole {
            torn.push(line.to_string());
        }
    }
    torn
}

/// Ctrl+Z under a screen gives the terminal back with the task's output
/// dumped, while the task goes on printing: the dump and the CLI's own lines
/// are never spliced, either way round.
#[cfg(unix)]
#[test]
fn a_ctrl_z_dump_is_never_spliced_with_the_clis_lines() {
    use std::time::Duration;
    let (mut child, chunks, _dir) = on_a_pty("tstpdump", None);
    let mut text = String::new();
    let start = std::time::Instant::now();
    while !text.contains("FRAME") && start.elapsed() < Duration::from_secs(30) {
        if let Ok((_, bytes)) = chunks.recv_timeout(Duration::from_millis(100)) {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert!(text.contains("FRAME"), "the screen was taken: {text:?}");
    // A Ctrl+Z on the terminal, then the shell's `fg`, three times: each
    // gives the terminal back with a dump while the task prints.
    // `fg` only once it has stopped, as a shell does: it stops itself after
    // its dump, and a SIGCONT before that would leave it stopped for good.
    let stopped = |pid: u32| {
        std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .is_ok_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim_start()
                    .starts_with('T')
            })
    };
    for _ in 0..3 {
        // SAFETY: signals to our own child.
        unsafe { libc::kill(child.id() as i32, libc::SIGTSTP) };
        let asked = std::time::Instant::now();
        while !stopped(child.id()) && asked.elapsed() < Duration::from_secs(20) {
            if child.try_wait().expect("try_wait").is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if child.try_wait().expect("try_wait").is_some() {
            break;
        }
        unsafe { libc::kill(child.id() as i32, libc::SIGCONT) };
        std::thread::sleep(Duration::from_millis(300));
    }
    let status = wait_bounded(&mut child, 60, &chunks);
    std::thread::sleep(Duration::from_millis(300));
    for (_, bytes) in chunks.try_iter() {
        text.push_str(&String::from_utf8_lossy(&bytes));
    }
    assert!(status.success(), "{status:?}");
    let torn = torn_lines(&text);
    assert!(
        text.contains(&format!("CLILINE-59999-{LONG}")) && torn.is_empty(),
        "{} torn lines, the first: {:?}",
        torn.len(),
        torn.iter().take(3).collect::<Vec<_>>()
    );
}

/// A screen taken and given back while a child fills a stdout nobody reads:
/// moving fds 1 and 2 neither fails the forwarder's write (the child would
/// die of SIGPIPE, its output lost) nor has the capture copy what the
/// forwarder copies too. Every line reaches stdout and output.log once.
#[cfg(unix)]
#[test]
fn a_screen_switch_over_a_stalled_stdout_loses_and_doubles_nothing() {
    use std::time::Duration;
    let fifos = tempfile::tempdir().expect("temp dir");
    let (mut reader, writer) = unread_fifo(fifos.path());
    let (mut child, chunks, dir) = on_a_pty("swapstall", Some(Stdio::from(writer)));
    let mut text = String::new();
    let start = std::time::Instant::now();
    while !text.contains("RELEASED") && start.elapsed() < Duration::from_secs(30) {
        if let Ok((_, bytes)) = chunks.recv_timeout(Duration::from_millis(100)) {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let drain = std::thread::spawn(move || {
        let mut got = Vec::new();
        let _ = std::io::Read::read_to_end(&mut reader, &mut got);
        got
    });
    let status = wait_bounded(&mut child, 60, &chunks);
    std::thread::sleep(Duration::from_millis(200));
    for (_, bytes) in chunks.try_iter() {
        text.push_str(&String::from_utf8_lossy(&bytes));
    }
    let stdout = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    assert!(status.success(), "{status:?}");
    assert!(
        plain(&text).contains("WAITED-0"),
        "the child exited 0: {:?}",
        plain(&text)
    );
    let numbered = |text: &str| {
        text.lines()
            .filter(|l| {
                l.strip_prefix('N')
                    .and_then(|r| r.split_once('-'))
                    .is_some_and(|(n, x)| {
                        n.len() == 6 && x.len() > 80 && x.bytes().all(|b| b == b'x')
                    })
            })
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let shown = numbered(&stdout);
    assert_eq!(shown.len(), 4000, "every line on stdout once");
    let log = numbered(&output_log(dir.path()));
    let unique: std::collections::HashSet<_> = log.iter().collect();
    assert_eq!(
        (log.len(), unique.len()),
        (4000, 4000),
        "every line in output.log once"
    );
}

/// A child given the CLI's own stdout while a screen holds the terminal,
/// across Ctrl+Z and `fg`: none of its lines is written over the screen
/// once it is back, and output.log has each once.
#[cfg(unix)]
#[test]
fn an_inherited_stdout_goes_back_into_the_screen_after_fg() {
    use std::time::Duration;
    let (mut child, chunks, dir) = on_a_pty("inhtstp", None);
    let mut raw = Vec::new();
    let start = std::time::Instant::now();
    while !raw.windows(5).any(|w| w == b"FRAME") && start.elapsed() < Duration::from_secs(30) {
        if let Ok((_, bytes)) = chunks.recv_timeout(Duration::from_millis(100)) {
            raw.extend(bytes);
        }
    }
    std::thread::sleep(Duration::from_millis(1000));
    let stopped = |pid: u32| {
        std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .is_ok_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim_start()
                    .starts_with('T')
            })
    };
    // SAFETY: signals to our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGTSTP) };
    let asked = std::time::Instant::now();
    while !stopped(child.id()) && asked.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    unsafe { libc::kill(child.id() as i32, libc::SIGCONT) };
    let status = wait_bounded(&mut child, 60, &chunks);
    std::thread::sleep(Duration::from_millis(200));
    for (_, bytes) in chunks.try_iter() {
        raw.extend(bytes);
    }
    assert!(status.success(), "{status:?}");
    // Walk the screen's enter/leave: no INH line while it is up.
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let mut up = false;
    let mut over = vec![];
    let mut rest = raw.as_str();
    while let Some(at) = rest.find(|c| c == '\x1b' || c == 'I') {
        rest = &rest[at..];
        if let Some(r) = rest.strip_prefix("\x1b[?1049h") {
            up = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("\x1b[?1049l") {
            up = false;
            rest = r;
        } else if rest.starts_with("INH") {
            let end = rest
                .find(|c: char| c == '\r' || c == '\n')
                .unwrap_or(rest.len());
            if up {
                over.push(rest[..end].to_string());
            }
            rest = &rest[end..];
        } else {
            rest = &rest[1..];
        }
    }
    assert!(over.is_empty(), "written over the screen: {over:?}");
    let log = output_log(dir.path());
    let inh: Vec<&str> = log.lines().filter(|l| l.starts_with("INH")).collect();
    let unique: std::collections::HashSet<_> = inh.iter().collect();
    assert_eq!(
        (inh.len(), unique.len()),
        (30, 30),
        "each line in output.log once: {inh:?}"
    );
}

/// Ctrl+Z while a release replays to a stdout nobody reads: the stop waits
/// on the release's order locks for a bounded while, not the replay's.
#[cfg(unix)]
#[test]
fn ctrl_z_during_a_stalled_release_stops_at_once() {
    use std::time::Duration;
    let fifos = tempfile::tempdir().expect("temp dir");
    let (mut reader, writer) = unread_fifo(fifos.path());
    let (mut child, chunks, _dir) = on_a_pty("tstpstall", Some(Stdio::from(writer)));
    let mut raw = Vec::new();
    let start = std::time::Instant::now();
    // The release has begun once the screen is left.
    while !raw.windows(8).any(|w| w == b"\x1b[?1049l") && start.elapsed() < Duration::from_secs(30)
    {
        if let Ok((_, bytes)) = chunks.recv_timeout(Duration::from_millis(10)) {
            raw.extend(bytes);
        }
    }
    let stopped = |pid: u32| {
        std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .is_ok_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim_start()
                    .starts_with('T')
            })
    };
    // SAFETY: signals to our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGTSTP) };
    let asked = std::time::Instant::now();
    while !stopped(child.id()) && asked.elapsed() < Duration::from_secs(10) {
        // The terminal read meanwhile, so a dump is never what it waits on.
        while chunks.try_recv().is_ok() {}
        std::thread::sleep(Duration::from_millis(10));
    }
    let took = asked.elapsed();
    unsafe { libc::kill(child.id() as i32, libc::SIGCONT) };
    let drain = std::thread::spawn(move || std::io::copy(&mut reader, &mut std::io::sink()));
    let status = wait_bounded(&mut child, 60, &chunks);
    let _ = drain.join();
    assert!(status.success(), "{status:?}");
    assert!(
        took < Duration::from_millis(1500),
        "stopped {took:?} after Ctrl+Z"
    );
}

/// The numbered lines a batch child writes, in `text`: whole ones and torn.
#[cfg(unix)]
fn batch_lines(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut whole, mut torn) = (vec![], vec![]);
    for l in plain(text).replace('\r', "").lines() {
        let ok = l
            .strip_prefix('N')
            .and_then(|r| r.split_once('-'))
            .is_some_and(|(n, x)| n.len() == 6 && x.len() >= 95 && x.bytes().all(|b| b == b'x'));
        if ok {
            whole.push(l.to_string());
        } else if l.contains("N0") {
            torn.push(l.to_string());
        }
    }
    (whole, torn)
}

/// Each of a batch child's 3000 lines, once and whole, in `text`.
#[cfg(unix)]
fn assert_every_batch_line_once(what: &str, text: &str) {
    let (whole, torn) = batch_lines(text);
    let unique: std::collections::HashSet<_> = whole.iter().collect();
    assert!(
        whole.len() == 3000 && unique.len() == 3000 && torn.is_empty(),
        "{what}: {} lines, {} distinct, {} torn: {:?}",
        whole.len(),
        unique.len(),
        torn.len(),
        torn.iter().take(3).collect::<Vec<_>>()
    );
}

/// A child given the CLI's stdout across two screens while stdout is read
/// slowly: what it still writes into the first screen's stopped capture
/// goes on through the second's, and is never cut short when a screen is
/// taken (a `select` the move cuts short), nor written alongside the second
/// screen's replay. Stdout and output.log get each line once.
#[cfg(unix)]
#[test]
fn a_child_on_the_clis_stdout_across_two_screens_loses_and_tears_nothing() {
    use std::time::{Duration, Instant};
    let fifos = tempfile::tempdir().expect("temp dir");
    let (mut reader, writer) = unread_fifo(fifos.path());
    let (mut child, chunks, dir) = on_a_pty("retake", Some(Stdio::from(writer)));
    // 20KB/s for 8s, then as fast as it comes.
    let drain = std::thread::spawn(move || {
        let start = Instant::now();
        let mut got = Vec::new();
        let mut buf = vec![0u8; 65536];
        loop {
            let slow = start.elapsed() < Duration::from_secs(8);
            let want = if slow { 1000 } else { buf.len() };
            match std::io::Read::read(&mut reader, &mut buf[..want]) {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
            if slow {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        got
    });
    let status = wait_bounded(&mut child, 90, &chunks);
    let stdout = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    assert!(status.success(), "{status:?}");
    assert_every_batch_line_once("stdout", &stdout);
    assert_every_batch_line_once("output.log", &output_log(dir.path()));
}

/// Ctrl+Z while a child writes to the CLI's stdout, a FIFO: what the screen
/// captured of stdout before the stop goes to stdout then, as a release's
/// does, not only into the dump on the terminal.
#[cfg(unix)]
#[test]
fn ctrl_z_passes_stdouts_captured_lines_to_a_stdout_that_is_no_terminal() {
    use std::time::Duration;
    let fifos = tempfile::tempdir().expect("temp dir");
    let (mut reader, writer) = unread_fifo(fifos.path());
    let drain = std::thread::spawn(move || {
        let mut got = Vec::new();
        let _ = std::io::Read::read_to_end(&mut reader, &mut got);
        got
    });
    let (mut child, chunks, _dir) = on_a_pty("fgslow", Some(Stdio::from(writer)));
    // Once the screen is up, and some of the child's lines with it.
    let mut raw = Vec::new();
    let start = std::time::Instant::now();
    while !raw.windows(7).any(|w| w == b"FRAME-A") && start.elapsed() < Duration::from_secs(30) {
        if let Ok((_, bytes)) = chunks.recv_timeout(Duration::from_millis(50)) {
            raw.extend(bytes);
        }
    }
    std::thread::sleep(Duration::from_millis(1500));
    let stopped = |pid: u32| {
        std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .is_ok_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim_start()
                    .starts_with('T')
            })
    };
    // SAFETY: signals to our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGTSTP) };
    let asked = std::time::Instant::now();
    while !stopped(child.id()) && asked.elapsed() < Duration::from_secs(20) {
        while chunks.try_recv().is_ok() {}
        if child.try_wait().expect("try_wait").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(stopped(child.id()), "stopped on Ctrl+Z");
    std::thread::sleep(Duration::from_millis(300));
    unsafe { libc::kill(child.id() as i32, libc::SIGCONT) };
    let status = wait_bounded(&mut child, 60, &chunks);
    let stdout = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    assert!(status.success(), "{status:?}");
    assert_every_batch_line_once("stdout", &stdout);
}
