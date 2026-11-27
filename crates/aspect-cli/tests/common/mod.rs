//! Shared by the integration tests under `tests/`, each of which spawns the
//! built CLI. Compiled into every test crate that declares `mod common;`.

mod hang_diag;

#[allow(unused_imports)]
pub use hang_diag::describe;

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

/// This test run's own folder, which every CLI a test spawns writes under:
/// Bazel's `$TEST_TMPDIR` for the test, else one per test process under the
/// temp dir (`aspect-tests-<binary>-<pid>`), the folders of runs no longer
/// running removed when it is made, so they never pile up.
pub fn run_dir() -> &'static std::path::Path {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = match std::env::var_os("TEST_TMPDIR") {
            Some(dir) => std::path::PathBuf::from(dir).join("aspect-tests"),
            None => {
                let prefix = format!("aspect-tests-{}-", test_binary());
                prune_runs(&prefix);
                std::env::temp_dir().join(format!("{prefix}{}", std::process::id()))
            }
        };
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("cannot make {}: {e}", dir.display()));
        dir
    })
}

/// Remove the temp dir's `<prefix><pid>` folders of processes no longer
/// running: earlier runs of this test binary. Best effort.
fn prune_runs(prefix: &str) {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_prefix(prefix)
            .filter(|p| p.parse::<u32>().is_ok())
        else {
            continue;
        };
        // `ps -p` fails for a pid no process has, whoever's it would be.
        let gone = std::process::Command::new("ps")
            .args(["-p", pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| !s.success());
        if gone {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The test run's task history folder ([`run_dir`]), which every CLI a test
/// spawns writes to unless the test names its own.
#[allow(dead_code)]
pub fn test_history_dir() -> std::path::PathBuf {
    run_dir().join("history")
}

/// `HOME` as the test process had it before [`isolate_writes`] gave the
/// tests a scratch one: for the tools a test runs itself (cargo), not for a
/// CLI under test.
#[allow(dead_code)]
pub fn real_home() -> Option<std::ffi::OsString> {
    isolate_writes();
    REAL_HOME.get().cloned().flatten()
}

static REAL_HOME: std::sync::OnceLock<Option<std::ffi::OsString>> = std::sync::OnceLock::new();

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

/// Once per test process: `ASPECT_HISTORY_DIR`, `ASPECT_CLI_CACHE` and
/// `HOME` set in its own environment, which every CLI it spawns inherits,
/// to folders of the test run's own ([`run_dir`]): `history`, `cache` and
/// an empty `home`, so no test writes the developer's task history or
/// cache, nor reads their user-level config. A command that clears its
/// environment gets them back through [`base_env`]; a test that names its
/// own still wins. Before then the process may not have started a thread
/// that reads the environment outside std's lock.
pub fn isolate_writes() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let home = run_dir().join("home");
        std::fs::create_dir_all(&home)
            .unwrap_or_else(|e| panic!("cannot make {}: {e}", home.display()));
        let _ = REAL_HOME.set(std::env::var_os("HOME"));
        // SAFETY: std's `set_var` and every std read of the environment
        // (a `Command`'s spawn among them) take its lock; the test harness's
        // threads read it only through std.
        unsafe {
            std::env::set_var("ASPECT_HISTORY_DIR", test_history_dir());
            std::env::set_var("ASPECT_CLI_CACHE", run_dir().join("cache"));
            std::env::set_var("HOME", home);
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
/// `PATH` stays. `HOME`, the task history and the CLI's cache are the test
/// run's own folders ([`isolate_writes`]).
#[allow(dead_code)]
pub fn base_env() -> std::collections::HashMap<String, String> {
    hang_diag::watch_for_hangs();
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
            let mut build = Command::new(&cargo);
            // Cargo's own registry and toolchain are under the real HOME.
            if let Some(home) = real_home() {
                build.env("HOME", home);
            }
            let status = build
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
