use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;

use allocative::Allocative;
use anyhow::Context;
use derive_more::Display;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

/// A Bazel aspect the runtime applies to a `build`/`test` invocation.
///
/// Created by `ctx.bazel.aspect(...)` and passed to
/// `ctx.bazel.build(aspects = [...])`. The runtime writes the aspect's source
/// into `@bazel_tools` — which is already in the repo graph, so applying the
/// aspect needs no `--inject_repository`/`--override_repository` and causes no
/// analysis-cache churn (the build reuses its warm analysis and just layers the
/// aspect nodes on top). See [`materialize`].
#[derive(Clone, Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.Aspect %{symbol}>")]
pub(crate) struct Aspect {
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

impl<'v> UnpackValue<'v> for Aspect {
    type Error = anyhow::Error;

    // `Ok(None)` on type mismatch so `UnpackList`/`Either` report a clean error.
    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.downcast_ref::<Aspect>().cloned())
    }
}

#[starlark_value(type = "bazel.Aspect")]
impl<'v> values::StarlarkValue<'v> for Aspect {}

starlark::starlark_simple_value!(Aspect);

/// Resolve `bazel info install_base` — the directory `@bazel_tools`
/// (`embedded_tools`) is materialized under. Honors the invocation's startup
/// flags (`--output_user_root` moves the install base).
fn install_base(startup_flags: &[String]) -> anyhow::Result<PathBuf> {
    let mut cmd = super::bazel_command();
    cmd.args(startup_flags);
    cmd.arg("info");
    cmd.arg("install_base");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());
    let (child, _guard) = super::live::spawn_registered(&mut cmd)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!(
            "`bazel info install_base` failed while materializing aspects: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
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
pub(crate) fn materialize(
    aspects: &[Aspect],
    startup_flags: &[String],
) -> anyhow::Result<Vec<String>> {
    if aspects.is_empty() {
        return Ok(vec![]);
    }
    let embedded = install_base(startup_flags)?.join("embedded_tools");
    let mut flags = Vec::new();
    for aspect in aspects {
        let hash = sha256::digest(aspect.implementation.as_str());
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
