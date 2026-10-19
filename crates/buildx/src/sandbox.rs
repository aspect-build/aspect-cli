//! The sandboxed local executor: what `ctx.execution.sandbox()` returns, the
//! host probe that decides whether one can exist, and the execution
//! requirement rules Bazel applies before it sandboxes a spawn.
//!
//! The runner itself (Bazel's `SymlinkedSandboxedSpawn` under a Rust
//! `SpawnRunner`) is `engine/sandboxed.rs`; this file is what does not need the
//! JVM, so the rules can be unit tested.

use crate::execution::{ExecOutcome, Executor, SpawnView};
use anyhow::{Result, bail};
use std::sync::{Arc, OnceLock};

/// The sandbox this host can run, named as Bazel names its runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// macOS: `sandbox-exec` over `process-wrapper`.
    Darwin,
    /// Any other POSIX host: a symlink forest run under `process-wrapper`.
    ProcessWrapper,
}

impl Kind {
    /// The runner name in `SpawnResult` and in the build metrics' runner counts.
    pub fn runner(self) -> &'static str {
        match self {
            Kind::Darwin => "darwin-sandbox",
            Kind::ProcessWrapper => "processwrapper-sandbox",
        }
    }
}

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Runs `/usr/bin/true` in a sandbox that allows everything, the probe
/// `DarwinSandboxedSpawnRunner.computeIsSupported` runs.
pub fn probe_sandbox_exec() -> Result<(), String> {
    let out = std::process::Command::new(SANDBOX_EXEC)
        .args(["-p", "(version 1) (allow default)", "/usr/bin/true"])
        .current_dir("/usr/bin")
        .output()
        .map_err(|e| format!("{SANDBOX_EXEC} could not run: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{SANDBOX_EXEC} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Which sandbox this host can run, probed once per process. The order is
/// `SandboxModule`'s preference: the platform's own sandbox, else
/// `process-wrapper`. Linux's `linux-sandbox` helper is not constructed (see the
/// README), so Linux hosts get `process-wrapper`.
pub fn host() -> Result<Kind, String> {
    static HOST: OnceLock<Result<Kind, String>> = OnceLock::new();
    HOST.get_or_init(|| {
        if cfg!(target_os = "macos") {
            probe_sandbox_exec().map(|()| Kind::Darwin)
        } else if cfg!(unix) {
            Ok(Kind::ProcessWrapper)
        } else {
            Err("sandboxed execution is not supported on this platform".to_string())
        }
    })
    .clone()
}

/// `Spawns.mayBeSandboxed`: a spawn tagged `no-sandbox`, `nosandbox` or `local`
/// is not sandboxed.
pub fn may_be_sandboxed(execution_info: &[(String, String)]) -> bool {
    !execution_info
        .iter()
        .any(|(k, _)| matches!(k.as_str(), "no-sandbox" | "nosandbox" | "local"))
}

/// `Spawns.requiresNetwork`: `block-network` denies, `requires-network` allows,
/// and otherwise Bazel's `--sandbox_default_allow_network` decides.
pub fn requires_network(execution_info: &[(String, String)], default_allow: bool) -> bool {
    let has = |key: &str| execution_info.iter().any(|(k, _)| k == key);
    if has("block-network") {
        return false;
    }
    if has("requires-network") {
        return true;
    }
    default_allow
}

/// Whether a build that was not given an executor can have the sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sandboxing {
    /// No executor of the build is sandboxed.
    None,
    /// The default: when the sandbox cannot be set up, the build runs unsandboxed.
    Optional,
    /// Asked for with `ctx.execution.sandbox()`: when it cannot be set up, the build fails.
    Required,
}

/// Actions run in Bazel's sandbox, except those whose execution requirements
/// say otherwise, which run in Bazel's standalone strategy like `local()`.
pub struct SandboxExecutor {
    pub name: String,
    pub sandboxing: Sandboxing,
}

impl SandboxExecutor {
    /// `ctx.execution.sandbox()`: fails now when the host cannot sandbox.
    pub fn required(name: Option<&str>) -> Result<Arc<dyn Executor>> {
        if let Err(why) = host() {
            bail!("ctx.execution.sandbox(): no sandbox works on this host: {why}");
        }
        Ok(Arc::new(SandboxExecutor {
            name: name.unwrap_or("sandbox").to_string(),
            sandboxing: Sandboxing::Required,
        }))
    }

    /// What `ctx.build` runs with when it is given no executor.
    pub fn optional() -> Arc<dyn Executor> {
        Arc::new(SandboxExecutor {
            name: "sandbox".to_string(),
            sandboxing: Sandboxing::Optional,
        })
    }
}

impl Executor for SandboxExecutor {
    fn name(&self) -> &str {
        &self.name
    }

    fn sandboxing(&self) -> Sandboxing {
        self.sandboxing
    }

    /// Runs in the Rust `exec`, which calls Bazel's strategies as delegates:
    /// the sandboxed one, or the standalone one for a spawn that may not be
    /// sandboxed. Falling through to Bazel's registered strategies would not
    /// do, because those cannot tell `local()` from `sandbox()`.
    fn runs_in_bazel(&self) -> bool {
        false
    }

    fn exec(&self, spawn: &Arc<SpawnView>) -> Result<ExecOutcome> {
        if may_be_sandboxed(&spawn.data.execution_info) {
            Ok(ExecOutcome::Bazel(spawn.run_in_sandbox()?))
        } else {
            Ok(ExecOutcome::Bazel(spawn.run_in_bazel()?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(keys: &[&str]) -> Vec<(String, String)> {
        keys.iter()
            .map(|k| (k.to_string(), String::new()))
            .collect()
    }

    #[test]
    fn untagged_spawns_are_sandboxed() {
        assert!(may_be_sandboxed(&[]));
        assert!(may_be_sandboxed(&tags(&["requires-network", "cpu:2"])));
    }

    #[test]
    fn the_three_opt_outs_run_unsandboxed() {
        for tag in ["no-sandbox", "nosandbox", "local"] {
            assert!(!may_be_sandboxed(&tags(&[tag])), "{tag}");
        }
        assert!(!may_be_sandboxed(&tags(&["requires-network", "local"])));
    }

    #[test]
    fn network_follows_the_tags_then_the_default() {
        assert!(requires_network(&[], true));
        assert!(!requires_network(&[], false));
        assert!(requires_network(&tags(&["requires-network"]), false));
        assert!(!requires_network(&tags(&["block-network"]), true));
        assert!(!requires_network(
            &tags(&["block-network", "requires-network"]),
            true
        ));
    }

    #[test]
    fn runner_names_are_bazels() {
        assert_eq!(Kind::Darwin.runner(), "darwin-sandbox");
        assert_eq!(Kind::ProcessWrapper.runner(), "processwrapper-sandbox");
    }

    #[test]
    fn sandboxing_orders_by_strength() {
        assert!(Sandboxing::None < Sandboxing::Optional);
        assert!(Sandboxing::Optional < Sandboxing::Required);
    }
}
