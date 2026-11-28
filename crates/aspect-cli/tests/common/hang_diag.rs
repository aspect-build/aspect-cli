//! What a test that hangs shows of what it was waiting on: [`describe`],
//! for a test that gave up on a process, and, opted into with
//! `HANG_DIAG_AFTER_SECS` (CI's `--config=ci` sets it), a watcher that
//! reports every process a test left running past that many seconds before
//! the test's own timeout ([`watch_for_hangs`]). Nothing else in the test
//! helpers attaches to or reads another process.

#![allow(dead_code)]

/// `HANG_DIAG_AFTER_SECS`, when set: the watcher's opt-in, and after how
/// long a process counts as hung (45s when it is no number).
fn after_secs() -> Option<u64> {
    std::env::var("HANG_DIAG_AFTER_SECS")
        .ok()
        .map(|s| s.trim().parse().unwrap_or(45))
}

/// On Linux, once per test process, when `HANG_DIAG_AFTER_SECS` is set:
/// every 10s, each of this process's descendants still alive after that
/// many seconds (a CLI under test, or a child of one) has its threads'
/// `comm`, `wchan`, `syscall`, `stat` and CPU used over one second (a busy
/// thread is slow, not stuck), every thread's stack for the first two
/// `aspect-cli` processes a tool attaches to (`gdb`, else `eu-stack`; 40s at
/// most a process, 60s in all; none tried again once all have failed to
/// attach to a live process, not for time), its open descriptors and its
/// children printed to stderr, once, under `HANG-DIAG`. For a CI timeout,
/// which otherwise says only which tests ran long.
pub fn watch_for_hangs() {
    #[cfg(target_os = "linux")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        if after_secs().is_some() {
            ONCE.call_once(|| {
                let _ = std::thread::Builder::new()
                    .name("hang-diag".into())
                    .spawn(linux::watch);
            });
        }
    }
}

/// What process `pid` and its descendants are doing, for a test that gave
/// up on one: on Linux as [`watch_for_hangs`] reports one; elsewhere their `ps`
/// rows (state, CPU time, elapsed), and `pid`'s stacks from `sample` over a
/// second where it can attach (macOS), cut off after 10s.
pub fn describe(pid: u32) -> String {
    #[cfg(target_os = "linux")]
    {
        linux::describe(pid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut pids = vec![pid];
        pids.extend(super::descendants(pid));
        let list = pids
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let ps = std::process::Command::new("ps")
            .args(["-o", "pid,ppid,stat,%cpu,time,etime,command", "-p", &list])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        // `sample` over a second, killed if it has not ended in ten.
        let sampled = match std::process::Command::new("sample")
            .args([pid.to_string().as_str(), "1"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut sample) => {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sample.try_wait().ok().flatten().is_none()
                    && std::time::Instant::now() < deadline
                {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                let _ = sample.kill();
                sample
                    .wait_with_output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .unwrap_or_default()
            }
            Err(_) => String::new(),
        };
        format!("ps:\n{ps}\nsample:\n{sampled}")
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;

    const CLOCK_TICKS: u64 = 100;

    /// Straight to stderr: the test harness captures `eprint!` from a
    /// thread a test started, and drops it when the test times out.
    fn say(text: &str) {
        use std::io::Write;
        let _ = std::io::stderr().write_all(text.as_bytes());
    }

    /// `path` read on a thread of its own, given up on after 500ms: a read of
    /// a process stuck in the kernel (D state) can block.
    fn read(path: &str) -> String {
        let path = path.to_string();
        let (send, got) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("hang-diag-read".into())
            .spawn(move || {
                let _ = send.send(std::fs::read_to_string(path).unwrap_or_default());
            });
        got.recv_timeout(Duration::from_millis(500))
            .unwrap_or_default()
    }

    /// Whether `pid` descends from `root`, by the parents in `parents`.
    fn descends(pid: u32, root: u32, parents: &HashMap<u32, u32>) -> bool {
        let mut at = pid;
        for _ in 0..64 {
            match parents.get(&at) {
                Some(&p) if p == root => return true,
                Some(&p) if p > 1 => at = p,
                _ => return false,
            }
        }
        false
    }

    /// `(ppid, starttime in ticks)` from `/proc/<pid>/stat`.
    fn stat(pid: u32) -> Option<(u32, u64)> {
        let text = read(&format!("/proc/{pid}/stat"));
        let rest = text.rsplit_once(')')?.1;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        Some((fields.get(1)?.parse().ok()?, fields.get(19)?.parse().ok()?))
    }

    fn pids() -> Vec<u32> {
        std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect()
    }

    fn uptime_ticks() -> u64 {
        let up: f64 = read("/proc/uptime")
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        (up * CLOCK_TICKS as f64) as u64
    }

    fn cmdline(pid: u32) -> String {
        read(&format!("/proc/{pid}/cmdline")).replace('\0', " ")
    }

    /// [`super::describe`] for `pid`, on Linux.
    pub fn describe(pid: u32) -> String {
        let parents: HashMap<u32, u32> = pids()
            .into_iter()
            .filter_map(|p| stat(p).map(|(pp, _)| (p, pp)))
            .collect();
        let mut out = String::new();
        let mut todo = vec![pid];
        while let Some(p) = todo.pop() {
            out.push_str(&report(p, &parents));
            todo.extend(parents.iter().filter(|(_, pp)| **pp == p).map(|(c, _)| *c));
        }
        out
    }

    fn dump(pid: u32, parents: &HashMap<u32, u32>) {
        say(&report(pid, parents));
    }

    /// A thread's user and system time so far, in ticks, from its `stat`.
    fn cpu_ticks(stat: &str) -> Option<u64> {
        let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
    }

    /// What `program args` prints to stdout, given up on (and killed) after
    /// `secs`: `Err(true)` for that, `Err(false)` where it can't be run or
    /// says nothing useful. Stdout is read as it comes, so a long report
    /// never blocks the child on a full pipe; stderr is dropped.
    fn bounded(program: &str, args: &[&str], secs: u64) -> Result<String, bool> {
        use std::io::Read;
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| false)?;
        let mut stdout = child.stdout.take().ok_or(false)?;
        let (send, got) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("hang-diag-drain".into())
            .spawn(move || {
                let mut text = Vec::new();
                let _ = stdout.read_to_end(&mut text);
                let _ = send.send(text);
            });
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        let status = loop {
            match child.try_wait().map_err(|_| false)? {
                Some(status) => break status,
                None if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(true);
                }
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        // A grandchild still holding the pipe ends the read no later.
        let text = got.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        let text = String::from_utf8_lossy(&text).into_owned();
        // gdb exits 0 having attached to nothing; a frame says it did.
        let useful = text.contains("#0") || (status.success() && !text.trim().is_empty());
        if useful { Ok(text) } else { Err(false) }
    }

    /// The `aspect-cli` processes, per test process, whose stacks are still
    /// printed: a capture that worked takes one, each up to
    /// [`STACKS_PER_PROCESS`], so a test's own timeout comes after the
    /// diagnosis, not during it.
    static STACKS_LEFT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(2);

    /// Set once every tool has failed to attach: none is tried again.
    static NO_TOOL_ATTACHES: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    /// Whether `pid` is a CLI under test (by its executable's name), whose
    /// stacks are worth the budget; a fixture's `sleep` is not.
    fn is_cli(pid: u32) -> bool {
        std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .is_some_and(|n| n == "aspect-cli")
            || read(&format!("/proc/{pid}/comm")).trim() == "aspect-cli"
    }

    /// The most one process's stacks take, every tool tried.
    const STACKS_PER_PROCESS: Duration = Duration::from_secs(40);

    /// The most time every process's stacks take in all: a tool that hangs
    /// attaching never pushes the dump past a CI timeout.
    const STACKS_IN_ALL: Duration = Duration::from_secs(60);

    /// The time spent capturing stacks so far, in milliseconds, against
    /// [`STACKS_IN_ALL`]: time spent, not since the first, so an early
    /// capture of a slow CLI leaves the budget for a later hang.
    static STACKS_SPENT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Whether `pid` is gone: no longer there, or a zombie (`Z` in its
    /// `stat`), which no tool attaches to.
    fn gone(pid: u32) -> bool {
        let stat = read(&format!("/proc/{pid}/stat"));
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_none_or(|state| state == "Z")
    }

    /// Every thread's stack in `pid`: `gdb`, else `eu-stack`. Else
    /// `Err(true)` where none could attach to a process still there, none
    /// running out of time; `Err(false)` where the process ended (a zombie
    /// too), or a tool, [`STACKS_PER_PROCESS`] or [`STACKS_IN_ALL`] ran out
    /// of time, so another may yet work.
    fn stacks(pid: u32) -> Result<String, bool> {
        use std::sync::atomic::Ordering::SeqCst;
        let began = std::time::Instant::now();
        let spent = Duration::from_millis(STACKS_SPENT_MS.load(SeqCst));
        let until = began + STACKS_PER_PROCESS.min(STACKS_IN_ALL.saturating_sub(spent));
        let result = stacks_until(pid, until);
        STACKS_SPENT_MS.fetch_add(began.elapsed().as_millis() as u64, SeqCst);
        result
    }

    /// [`stacks`], every tool given up on at `until`.
    fn stacks_until(pid: u32, until: std::time::Instant) -> Result<String, bool> {
        let p = pid.to_string();
        let gdb = ["-batch", "-nx", "-p", &p, "-ex", "thread apply all bt"];
        let eu = ["-p", &p];
        let left = || {
            until
                .saturating_duration_since(std::time::Instant::now())
                .as_secs()
                .min(15)
        };
        let mut timed_out = false;
        for (program, args) in [("gdb", &gdb[..]), ("eu-stack", &eu[..])] {
            if left() == 0 {
                return Err(false);
            }
            match bounded(program, args, left()) {
                Ok(text) => return Ok(text),
                Err(late) => timed_out |= late,
            }
        }
        Err(!timed_out && !gone(pid))
    }

    fn report(pid: u32, parents: &HashMap<u32, u32>) -> String {
        let mut out = format!("HANG-DIAG pid {pid}: {}\n", cmdline(pid));
        let tasks = format!("/proc/{pid}/task");
        let mut tids: Vec<u32> = std::fs::read_dir(&tasks)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect();
        tids.sort();
        let before: HashMap<u32, u64> = tids
            .iter()
            .filter_map(|tid| Some((*tid, cpu_ticks(&read(&format!("{tasks}/{tid}/stat")))?)))
            .collect();
        std::thread::sleep(Duration::from_secs(1));
        for tid in tids {
            let t = format!("{tasks}/{tid}");
            let stat = read(&format!("{t}/stat"));
            let busy = match (before.get(&tid), cpu_ticks(&stat)) {
                (Some(a), Some(b)) => format!("{}ms", b.saturating_sub(*a) * 1000 / CLOCK_TICKS),
                _ => "?".into(),
            };
            out.push_str(&format!(
                "HANG-DIAG   task {tid} comm={} wchan={} syscall={} cpu_last_1s={busy} stat={}\n",
                read(&format!("{t}/comm")).trim(),
                read(&format!("{t}/wchan")).trim(),
                read(&format!("{t}/syscall")).trim(),
                stat.trim(),
            ));
        }
        use std::sync::atomic::Ordering::SeqCst;
        if !is_cli(pid) {
            // Not an aspect-cli process: no stacks, nor budget spent.
        } else if NO_TOOL_ATTACHES.load(SeqCst) {
            out.push_str("HANG-DIAG   no stacks: no tool could attach (see above)\n");
        } else if STACKS_LEFT
            .fetch_update(SeqCst, SeqCst, |n| n.checked_sub(1))
            .is_err()
        {
            out.push_str("HANG-DIAG   no stacks: those of two processes were printed already\n");
        } else {
            // Reserved above; given back unless the capture worked.
            match stacks(pid) {
                Ok(text) => {
                    for line in text.lines() {
                        out.push_str(&format!("HANG-DIAG   | {line}\n"));
                    }
                }
                Err(cannot_attach) => {
                    STACKS_LEFT.fetch_add(1, SeqCst);
                    if cannot_attach {
                        NO_TOOL_ATTACHES.store(true, SeqCst);
                        out.push_str("HANG-DIAG   no stacks: neither gdb nor eu-stack could attach; not tried again\n");
                    } else {
                        out.push_str("HANG-DIAG   no stacks: the process ended, or the tools ran out of time\n");
                    }
                }
            }
        }
        let fds = format!("/proc/{pid}/fd");
        let mut fdl: Vec<(u32, String)> = std::fs::read_dir(&fds)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_str()?.parse().ok()?;
                Some((n, std::fs::read_link(e.path()).ok()?.display().to_string()))
            })
            .collect();
        fdl.sort();
        for (n, to) in fdl {
            out.push_str(&format!("HANG-DIAG   fd {n} -> {to}\n"));
        }
        let mut kids: Vec<u32> = parents
            .iter()
            .filter(|(_, p)| **p == pid)
            .map(|(c, _)| *c)
            .collect();
        kids.sort();
        for kid in kids {
            out.push_str(&format!("HANG-DIAG   child {kid}: {}\n", cmdline(kid)));
        }
        out
    }

    pub fn watch() {
        let me = std::process::id();
        let Some((_, born)) = stat(me) else { return };
        let after = Duration::from_secs(super::after_secs().unwrap_or(45));
        let mut told = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_secs(10));
            let now = uptime_ticks();
            let all: Vec<(u32, u32, u64)> = pids()
                .into_iter()
                .filter_map(|p| stat(p).map(|(pp, st)| (p, pp, st)))
                .collect();
            let parents: HashMap<u32, u32> = all.iter().map(|(p, pp, _)| (*p, *pp)).collect();
            let mut own = std::fs::read_dir("/proc/self/fd")
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| std::fs::read_link(e.path()).ok())
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>();
            let mut reported = false;
            for (pid, _, start) in &all {
                let age = Duration::from_millis(now.saturating_sub(*start) * 1000 / CLOCK_TICKS);
                if *start > born && age > after && descends(*pid, me, &parents) && told.insert(*pid)
                {
                    dump(*pid, &parents);
                    reported = true;
                }
            }
            if reported {
                own.sort();
                say(&format!(
                    "HANG-DIAG test process {me} fds: {}\n",
                    own.join(" ")
                ));
            }
        }
    }
}
