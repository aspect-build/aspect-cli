//! Applying a Bazel aspect to an invocation without touching its analysis
//! cache.
//!
//! The aspect's source is written into `@bazel_tools` — which is already in
//! the repo graph, so applying it needs no `--inject_repository` or
//! `--override_repository` and causes no analysis-cache churn (the build
//! reuses its warm analysis and just layers the aspect nodes on top).

use std::path::Path;

use anyhow::Context;
use sha2::{Digest, Sha256};

use crate::server::Server;

/// A Bazel aspect to apply to a `build`/`test` invocation.
#[derive(Clone, Debug)]
pub struct Aspect {
    /// The `.bzl` source defining the aspect.
    pub implementation: String,
    /// The aspect symbol within `implementation` (the `%name`).
    pub symbol: String,
    /// `--aspects_parameters=<k>=<v>` pairs. A value that varies per run (e.g. a
    /// nonce) mints a fresh `AspectKey`, forcing the aspect to re-evaluate.
    pub parameters: Vec<(String, String)>,
    /// Output groups to request so the aspect's outputs build
    /// (`--output_groups=+<g>`).
    pub output_groups: Vec<String>,
}

/// Write `contents` to `path` atomically (temp file + rename), so a concurrent
/// build never observes a half-written aspect file.
fn write_atomic(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} into place", path.display()))?;
    Ok(())
}

/// Materialize `aspects` into `@bazel_tools` and return the Bazel flags to
/// append to the invocation (`--aspects` / `--aspects_parameters` /
/// `--output_groups`). No-op (empty vec) when there are no aspects.
///
/// Each aspect is written to `<install_base>/embedded_tools/aspect_<hash>/`,
/// keyed by a content hash of its source, so the write is idempotent and
/// concurrency-safe, and reused across runs. It self-heals if the install base
/// is re-extracted (e.g. on a Bazel upgrade) since it's rewritten when absent.
pub fn materialize(server: &Server, aspects: &[Aspect]) -> anyhow::Result<Vec<String>> {
    if aspects.is_empty() {
        return Ok(vec![]);
    }
    let embedded = server.install_base()?.join("embedded_tools");
    let mut flags = Vec::new();
    for aspect in aspects {
        let hash = format!("{:x}", Sha256::digest(aspect.implementation.as_bytes()));
        let pkg = format!("aspect_{}", &hash[..16]);
        let dir = embedded.join(&pkg);
        let bzl = dir.join("aspect.bzl");
        if !bzl.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("creating aspect package {}", dir.display()))?;
            // Empty BUILD.bazel: makes the dir a package so the label resolves,
            // with no targets — so `//...` / attribute queries never pick it up.
            write_atomic(&dir.join("BUILD.bazel"), b"")?;
            write_atomic(&bzl, aspect.implementation.as_bytes())?;
        }
        flags.push(format!(
            "--aspects=@bazel_tools//{pkg}:aspect.bzl%{}",
            aspect.symbol
        ));
        for (key, value) in &aspect.parameters {
            flags.push(format!("--aspects_parameters={key}={value}"));
        }
        for group in &aspect.output_groups {
            flags.push(format!("--output_groups=+{group}"));
        }
    }
    Ok(flags)
}
