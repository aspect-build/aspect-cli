//! Shared by the integration tests under `tests/`, each of which spawns the
//! built CLI. Compiled into every test crate that declares `mod common;`.

/// Locate the CLI under test.
///
/// Bazel sets `ASPECT_CLI_BIN` from the `rust_test` rule's `env` via
/// `$(rootpath :aspect-cli)`, relative to the runfiles root that is a Bazel-run
/// test's cwd. Under cargo, `CARGO_BIN_EXE_*` points at the binary cargo already
/// built for this test; `option_env!` because that variable does not exist in a
/// Bazel build, where `env!` would fail to compile.
pub fn aspect_cli() -> String {
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
/// `HOME` and `PATH` stay: the CLI resolves its builtins cache under `HOME`.
/// The task history goes to a temp folder of the test process's own.
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
        std::env::temp_dir()
            .join(format!("aspect-test-history-{}", std::process::id()))
            .to_string_lossy()
            .into_owned(),
    );
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

/// On Linux, once per test process: every 10s, each process started since
/// this one that is still alive after 45s (a CLI under test, a child of
/// one, or one left behind and reparented) has its threads' `comm`,
/// `wchan`, `syscall` and `stat`, its open descriptors and its children
/// printed to stderr, once, under `HANG-DIAG` (`HANG_DIAG_AFTER_SECS`
/// overrides the 45s). For a CI timeout, which
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

    fn read(path: &str) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
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

    fn dump(pid: u32, parents: &HashMap<u32, u32>) {
        let mut out = format!("HANG-DIAG pid {pid}: {}\n", cmdline(pid));
        let tasks = format!("/proc/{pid}/task");
        let mut tids: Vec<u32> = std::fs::read_dir(&tasks)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect();
        tids.sort();
        for tid in tids {
            let t = format!("{tasks}/{tid}");
            out.push_str(&format!(
                "HANG-DIAG   task {tid} comm={} wchan={} syscall={} stat={}\n",
                read(&format!("{t}/comm")).trim(),
                read(&format!("{t}/wchan")).trim(),
                read(&format!("{t}/syscall")).trim(),
                read(&format!("{t}/stat")).trim(),
            ));
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
        say(&out);
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
                if *pid != me && *start > born && age > after && told.insert(*pid) {
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
