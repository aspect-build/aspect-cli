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
/// does for the same reason. `HOME` and `PATH` stay: the CLI resolves its
/// builtins cache under `HOME`.
#[allow(dead_code)]
pub fn base_env() -> std::collections::HashMap<String, String> {
    const DROP_NAMES: &[&str] = &["BUILDKITE", "CI"];
    const DROP_PREFIXES: &[&str] = &[
        "ASPECT_",
        "BASIL_",
        "BUILDKITE_",
        "CIRCLE",
        "GITHUB_",
        "GITLAB_",
    ];
    std::env::vars()
        .filter(|(k, _)| {
            !DROP_NAMES.contains(&k.as_str()) && !DROP_PREFIXES.iter().any(|p| k.starts_with(p))
        })
        .collect()
}
