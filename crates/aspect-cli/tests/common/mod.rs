//! Shared by the integration tests under `tests/`, each of which spawns the
//! built CLI. Compiled into every test crate that declares `mod common;`.

/// Locate the CLI under test.
///
/// Bazel sets `ASPECT_CLI_BIN` from the `rust_test` rule's `env` via
/// `$(rootpath :aspect-cli)`, relative to the runfiles root that is a Bazel-run
/// test's cwd. Under cargo, `CARGO_BIN_EXE_*` points at the binary cargo already
/// built for this test; `option_env!` because that variable does not exist in a
/// Bazel build, where `env!` would fail to compile.
///
/// The first call also points the task history and the CLI's cache at the
/// test binary's own folders ([`isolate_writes`]): every CLI a test spawns
/// inherits them, and one whose command clears its environment
/// (`env_clear()`) gets them back through [`base_env`].
pub fn aspect_cli() -> String {
    isolate_writes();
    match std::env::var("ASPECT_CLI_BIN") {
        Ok(p) => std::fs::canonicalize(&p)
            .unwrap_or_else(|e| panic!("ASPECT_CLI_BIN={p:?} not found: {e}"))
            .to_string_lossy()
            .into_owned(),
        Err(_) => option_env!("CARGO_BIN_EXE_aspect-cli")
            .expect("set ASPECT_CLI_BIN or run under cargo")
            .to_string(),
    }
}

/// The test binary's own task history folder, which every CLI a test spawns
/// writes to unless the test names its own: one per binary (its name
/// without cargo's hash), reused run to run, so they never pile up.
#[allow(dead_code)]
pub fn test_history_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("aspect-test-history-{}", test_binary()))
}

/// The test binary's name, without the `-<hash>` cargo adds.
fn test_binary() -> String {
    let exe = std::env::current_exe().ok();
    let stem = exe
        .as_deref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tests".into());
    match stem.rsplit_once('-') {
        Some((name, hash)) if hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit()) => {
            name.to_string()
        }
        _ => stem,
    }
}

/// Once per test process: `ASPECT_HISTORY_DIR` and `ASPECT_CLI_CACHE` set in
/// its own environment, which every CLI it spawns inherits, to the test
/// binary's own folders under the temp dir ([`test_history_dir`]), so no
/// test writes the developer's task history or cache. A command that clears
/// its environment gets them back through [`base_env`]; a test that names
/// its own still wins. Before then the process may not have
/// started a thread that reads the environment outside std's lock.
pub fn isolate_writes() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let cache = std::env::temp_dir().join(format!("aspect-test-cache-{}", test_binary()));
        // SAFETY: std's `set_var` and every std read of the environment
        // (a `Command`'s spawn among them) take its lock; the test harness's
        // threads read it only through std.
        unsafe {
            std::env::set_var("ASPECT_HISTORY_DIR", test_history_dir());
            std::env::set_var("ASPECT_CLI_CACHE", cache);
        }
    });
}

/// The environment a spawned CLI runs under: the developer's own, minus everything
/// that would make the result depend on where the test ran.
///
/// `ASPECT_*` would override a fixture's config, the CI identity variables
/// steer the built-in tasks into their CI code paths (status checks, runner
/// detection) - the same filtering `tools/cache-diff-integration.py` does for
/// the same reason - a coding agent's variables switch the console to its
/// `agent` layout, `CLICOLOR_FORCE` would color piped output, and `OTEL_*`
/// would register exporters that print to stderr or ship to a collector.
/// The colour theme gets a known dark background (`COLORFGBG`), 256 colours
/// (`TERM`), and none of the developer's `COLORTERM`, `NO_COLOR`, or the
/// variables that say which terminal or session it is (hyperlinks, SSH).
/// `HOME` and `PATH` stay. The task history and the CLI's cache go to the
/// test binary's own temp folders ([`isolate_writes`]).
#[allow(dead_code)]
pub fn base_env() -> std::collections::HashMap<String, String> {
    watch_for_hangs();
    const DROP_NAMES: &[&str] = &[
        "BUILDKITE",
        "CI",
        "CLICOLOR_FORCE",
        "COLORFGBG",
        "COLORTERM",
        "NO_COLOR",
        "TERM_PROGRAM",
        "TMUX",
        "KITTY_WINDOW_ID",
        "WT_SESSION",
        "VTE_VERSION",
        "SSH_CONNECTION",
        "SSH_TTY",
    ];
    const DROP_PREFIXES: &[&str] = &[
        "ASPECT_",
        "BASIL_",
        "BUILDKITE_",
        "CIRCLE",
        "GITHUB_",
        "GITLAB_",
        "OTEL_",
    ];
    isolate_writes();
    let mut env: std::collections::HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| {
            !DROP_NAMES.contains(&k.as_str())
                && !axl_runtime::agent::ENV_VARS.contains(&k.as_str())
                && !DROP_PREFIXES.iter().any(|p| k.starts_with(p))
        })
        .collect();
    // A known dark background: the theme's fixed colours, at 256 without
    // `COLORTERM`, and no system appearance asked. A test of another
    // background sets its own.
    env.insert("COLORFGBG".into(), "15;0".into());
    env.insert("TERM".into(), "xterm-256color".into());
    // Every task keeps a record in the task history: the test's own, not the
    // user's. A test that reads the history names its own after this.
    env.insert(
        "ASPECT_HISTORY_DIR".into(),
        test_history_dir().to_string_lossy().into_owned(),
    );
    if let Some(cache) = std::env::var_os("ASPECT_CLI_CACHE") {
        env.insert(
            "ASPECT_CLI_CACHE".into(),
            cache.to_string_lossy().into_owned(),
        );
    }
    env
}

/// `cmd` without a coding agent's variables, which would switch the console
/// to its `agent` layout, for a test that otherwise keeps the developer's
/// environment.
#[allow(dead_code)]
pub fn without_coding_agent(cmd: &mut std::process::Command) -> &mut std::process::Command {
    for var in axl_runtime::agent::ENV_VARS {
        cmd.env_remove(var);
    }
    cmd
}

/// Locate basil, the fake `bazel` the CLI is pointed at.
///
/// Bazel sets `BASIL_BIN` from the `rust_test` rule's `env` via
/// `$(rootpath //crates/basil)`, relative to the runfiles root that is a
/// Bazel-run test's cwd. Under cargo, `CARGO_BIN_EXE_*` covers only this
/// crate's own binaries, so basil is built on demand and found next to this
/// test executable — the same recursive-cargo pattern `axl_runtime::test` uses.
#[allow(dead_code)]
pub fn basil_bin() -> &'static str {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::OnceLock;

    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(p) = std::env::var("BASIL_BIN") {
            return std::fs::canonicalize(&p)
                .unwrap_or_else(|e| panic!("BASIL_BIN={p:?} not found: {e}"))
                .to_string_lossy()
                .into_owned();
        }
        let test_exe = std::env::current_exe().expect("current_exe");
        let mut path: PathBuf = test_exe.parent().expect("test exe parent").to_path_buf();
        if path.ends_with("deps") {
            path.pop();
        }
        path.push("basil");
        if !path.exists() {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            // Only reached under cargo, which sets CARGO_MANIFEST_DIR for test
            // binaries. Read at run time: env! would embed the build path.
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
            let workspace_root = PathBuf::from(manifest_dir)
                .parent()
                .and_then(Path::parent)
                .expect("workspace root above crates/aspect-cli")
                .to_path_buf();
            let status = Command::new(&cargo)
                .args(["build", "--quiet", "-p", "basil"])
                .current_dir(&workspace_root)
                .status()
                .expect("invoking cargo to build basil");
            assert!(status.success(), "cargo build -p basil failed: {status}");
        }
        assert!(path.exists(), "basil not found at {}", path.display());
        path.to_string_lossy().into_owned()
    })
}

/// On Linux, once per test process: every 10s, each of this process's
/// descendants still alive after 45s (a CLI under test, or a child of one)
/// has its threads' `comm`, `wchan`, `syscall`, `stat` and CPU used over
/// one second (a busy thread is slow, not stuck), every thread's stack
/// for the first two `aspect-cli` processes a tool attaches to (`gdb`,
/// else `eu-stack`, each tried under `sudo -n timeout` too, as Ubuntu lets
/// no non-ancestor attach; 40s at most a process, 60s in all; none tried again once
/// all have failed to attach to a live process, not for time), its open
/// descriptors and its
/// children printed to stderr, once, under `HANG-DIAG`
/// (`HANG_DIAG_AFTER_SECS` overrides the 45s). For a CI timeout, which
/// otherwise says only which tests ran long. Nothing elsewhere.
#[allow(dead_code)]
fn watch_for_hangs() {
    #[cfg(target_os = "linux")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("hang-diag".into())
                .spawn(hang_diag::watch);
        });
    }
}

/// The pids of every process under `pid`, its children's children too,
/// from `ps` (Linux and macOS alike); none when `ps` can't be run.
#[allow(dead_code)]
pub fn descendants(pid: u32) -> Vec<u32> {
    // `ps` names every process's parent on Linux and macOS alike.
    let table = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let pairs: Vec<(u32, u32)> = table
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect();
    let mut out = vec![];
    let mut todo = vec![pid];
    while let Some(p) = todo.pop() {
        for &(c, pp) in &pairs {
            if pp == p && !out.contains(&c) {
                out.push(c);
                todo.push(c);
            }
        }
    }
    out
}

/// What process `pid` and its descendants are doing, for a test that gave
/// up on one (see [`watch_for_hangs`]); Linux only, empty elsewhere.
#[allow(dead_code)]
pub fn describe(pid: u32) -> String {
    #[cfg(target_os = "linux")]
    {
        hang_diag::describe(pid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        String::new()
    }
}

#[cfg(target_os = "linux")]
mod hang_diag {
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

    /// [`super::describe`] for `pid`.
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

    /// Whether `timeout` (coreutils) can be run: what bounds a stack tool
    /// under `sudo`, which a kill of `sudo` itself would leave attached.
    fn have_timeout() -> bool {
        static HAVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *HAVE.get_or_init(|| bounded("timeout", &["--version"], 5).is_ok())
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

    /// Every thread's stack in `pid`: `gdb`, else `eu-stack`, each plain and
    /// under `sudo -n timeout` (only where `timeout` is there). Else
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
            let secs = left();
            if secs == 0 || !have_timeout() {
                continue;
            }
            let bound = secs.to_string();
            let mut sudo = vec!["-n", "timeout", "-s", "KILL", &bound, program];
            sudo.extend_from_slice(args);
            match bounded("sudo", &sudo, secs + 2) {
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
        let after = Duration::from_secs(
            std::env::var("HANG_DIAG_AFTER_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(45),
        );
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
