use std::env::temp_dir;
use std::fs;
use std::fs::File;

use std::io::Read;
use std::process::Stdio;

use anyhow::Context;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;

use starlark::typing::Ty;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::starlark_value;
use starlark::values::type_repr::StarlarkTypeRepr;

use axl_proto::blaze_query as query;
use bazel::command::query as bazel_query;

use std::sync::Arc;

use super::runner::SignalRunner;
use crate::engine::cancellation::Signals;
use crate::engine::children;

/// A target as a script sees it: the proto message of its kind.
#[derive(Debug, Clone)]
pub struct Target(bazel_query::Target);

impl<'v> StarlarkTypeRepr for Target {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        Ty::unions(vec![
            Ty::starlark_value::<query::Rule>(),
            Ty::starlark_value::<query::SourceFile>(),
            Ty::starlark_value::<query::GeneratedFile>(),
            Ty::starlark_value::<query::PackageGroup>(),
        ])
    }
}

impl<'v> starlark::values::AllocValue<'v> for Target {
    fn alloc_value(self, heap: starlark::values::Heap<'v>) -> starlark::values::Value<'v> {
        match self.0 {
            bazel_query::Target::SourceFile(source_file) => heap.alloc_simple(source_file),
            bazel_query::Target::GeneratedFile(generated_file) => heap.alloc_simple(generated_file),
            bazel_query::Target::Rule(rule) => heap.alloc_simple(rule),
            bazel_query::Target::PackageGroup(package_group) => heap.alloc_simple(package_group),
        }
    }
}

/// The result of `ctx.bazel.query(expr, rc=…)`: an iterable set of `Target`s.
/// Supports `len()`, indexing, and iteration (`for t in result`).
#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.query.Query {targets:?}>")]
pub struct Query {
    #[allocative(skip)]
    targets: Vec<Target>,
}

impl<'v> AllocValue<'v> for Query {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "bazel.query.Query")]
impl<'v> values::StarlarkValue<'v> for Query {
    fn get_type_starlark_repr() -> Ty {
        Ty::iter(Target::starlark_type_repr())
    }

    fn length(&self) -> starlark::Result<i32> {
        Ok(self.targets.len() as i32)
    }

    fn at(&self, index: values::Value<'v>, heap: Heap<'v>) -> starlark::Result<values::Value<'v>> {
        let idx = index.unpack_i32().ok_or(anyhow!("pass an int"))?;
        let target = self
            .targets
            .get(idx as usize)
            .map(|t| heap.alloc(t.clone()))
            .ok_or(anyhow!("no target at index {idx}"))?;
        Ok(target)
    }

    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<values::Value<'v>>> {
        Ok(self
            .targets
            .clone()
            .into_iter()
            .map(|f| heap.alloc(f))
            .collect())
    }
}

/// Run `bazel query <expr>` with the given startup + command flags and decode
/// the streamed-proto result into a `TargetSet`. Fails with Bazel's own stderr
/// on a non-zero exit — a failed query is not the same as one that matched
/// nothing. Used by `ctx.bazel.query(expr, rc=…)`.
pub fn run(
    signals: &Arc<Signals>,
    expr: &str,
    startup_flags: &[String],
    flags: &[String],
    directory: Option<String>,
    announce: super::build::AnnounceSpawn,
) -> anyhow::Result<Query> {
    let runner = SignalRunner(signals);
    let mut cmd = bazel_query::command(&runner, expr, startup_flags, flags, directory);
    let out = temp_dir().join("query.bin");
    let _ = fs::remove_file(&out);
    let errfile_path = temp_dir().join("query.err");
    let _ = fs::remove_file(&errfile_path);

    let outfile = File::create(&out)?;
    // Capture stderr to a file so a failed query's diagnostics can be
    // surfaced below. A file (rather than a pipe read after wait)
    // avoids a pipe-buffer deadlock on a chatty query.
    let errfile = File::create(&errfile_path)?;

    cmd.stderr(errfile);
    cmd.stdout(outfile);
    cmd.stdin(Stdio::null());

    // Mirror the build/test spawn disclosure: the version line costs an
    // extra `bazel info`, so only pay for it when actually announcing.
    if announce.version || announce.command {
        let version = if announce.version {
            bazel::server::Server::new(&runner, startup_flags)
                .info()
                .ok()
                .and_then(|(_pid, version)| version)
        } else {
            None
        };
        super::build::announce_spawn(announce, version.as_ref(), &cmd);
    }

    // Bound to the root so a cancel reaches the client: large queries can
    // run for many seconds, and the wait is a safe point.
    let status = children::spawn_bazel(&mut cmd, signals)
        .with_context(|| "failed to spawn bazel")?
        .wait()?;

    if !status.success() {
        let mut stderr = String::new();
        if let Ok(mut f) = File::open(&errfile_path) {
            let _ = f.read_to_string(&mut stderr);
        }
        return Err(anyhow!(bazel_query::failure_message(
            expr,
            status.code(),
            &stderr
        )));
    }

    let mut buf = vec![];
    File::open(&out)?.read_to_end(&mut buf)?;

    let targets = bazel_query::decode(&buf).into_iter().map(Target).collect();
    Ok(Query { targets })
}
