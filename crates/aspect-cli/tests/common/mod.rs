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
/// `ASPECT_*` would override a fixture's config, and the CI identity
/// variables steer the built-in tasks into their CI code paths (status checks,
/// runner detection) — the same filtering `tools/cache-diff-integration.py`
/// does for the same reason. A coding agent's variables would switch the
/// console to its `agent` profile, so they go too: the ones `under_agent` in
/// `private/lib/agent.axl` reads, kept in step with it by hand. `HOME` and
/// `PATH` stay: the CLI resolves its builtins cache under `HOME`. The task
/// history goes to a temp folder of the test process's own.
#[allow(dead_code)]
pub fn base_env() -> std::collections::HashMap<String, String> {
    const DROP_NAMES: &[&str] = &[
        "BUILDKITE",
        "CI",
        "AI_AGENT",
        "CLAUDECODE",
        "CLAUDE_CODE_SESSION_ID",
        "CODEX_THREAD_ID",
        "CURSOR_AGENT",
        "GEMINI_CLI",
        "GEMINI_SESSION_ID",
        "GROK_SESSION_ID",
    ];
    const DROP_PREFIXES: &[&str] = &[
        "ASPECT_",
        "BASIL_",
        "BUILDKITE_",
        "CIRCLE",
        "GITHUB_",
        "GITLAB_",
    ];
    let mut env: std::collections::HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| {
            !DROP_NAMES.contains(&k.as_str()) && !DROP_PREFIXES.iter().any(|p| k.starts_with(p))
        })
        .collect();
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

/// Locate basil, the fake `bazel` the CLI is pointed at.
///
/// Bazel sets `BASIL_BIN` from the `rust_test` rule's `env` via
/// `$(rootpath //crates/basil)`, relative to the runfiles root that is a
/// Bazel-run test's cwd. Under cargo, `CARGO_BIN_EXE_*` covers only this
/// crate's own binaries, so basil is built with cargo and found next to this
/// test executable, the same recursive-cargo pattern `axl_runtime::test` uses.
#[allow(dead_code)] // Not every test crate drives Bazel.
pub fn basil_bin() -> &'static str {
    static BIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(p) = std::env::var("BASIL_BIN") {
            return std::fs::canonicalize(&p)
                .unwrap_or_else(|e| panic!("BASIL_BIN={p:?} not found: {e}"))
                .to_string_lossy()
                .into_owned();
        }
        let test_exe = std::env::current_exe().expect("current_exe");
        let mut path: std::path::PathBuf =
            test_exe.parent().expect("test exe parent").to_path_buf();
        if path.ends_with("deps") {
            path.pop();
        }
        path.push("basil");
        let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root above crates/aspect-cli")
            .to_path_buf();
        // Cargo decides whether basil is stale, its dependencies included.
        // The variables cargo sets for this test binary are left out: cargo
        // reruns a build script that watches them (`ring`'s) when they
        // change, so inheriting them would rebuild basil here, then again on
        // the next `cargo build`.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let mut build = std::process::Command::new(&cargo);
        for (k, _) in std::env::vars() {
            if k.starts_with("CARGO_PKG_")
                || matches!(
                    k.as_str(),
                    "CARGO_MANIFEST_DIR"
                        | "CARGO_MANIFEST_PATH"
                        | "CARGO_CRATE_NAME"
                        | "CARGO_BIN_NAME"
                        | "CARGO_PRIMARY_PACKAGE"
                        | "CARGO_TARGET_TMPDIR"
                        | "CARGO_RUSTC_CURRENT_DIR"
                        | "OUT_DIR"
                )
            {
                build.env_remove(k);
            }
        }
        let status = build
            .args(["build", "--quiet", "-p", "basil"])
            .current_dir(&workspace_root)
            .status()
            .expect("invoking cargo to build basil");
        assert!(status.success(), "cargo build -p basil failed: {status}");
        assert!(path.exists(), "basil not found at {}", path.display());
        path.to_string_lossy().into_owned()
    })
}
