use std::env::temp_dir;
use std::fs;
use std::fs::File;

use std::io::Read;
use std::process::Stdio;

use anyhow::Context;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;

use prost::bytes::Bytes;

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
use prost::Message;

#[derive(Debug, Clone)]
pub enum Target {
    // We leave environment_group out as its undocumented.
    // https://github.com/bazelbuild/bazel/issues/10849
    SourceFile(query::SourceFile),
    GeneratedFile(query::GeneratedFile),
    Rule(query::Rule),
    PackageGroup(query::PackageGroup),
}

impl TryFrom<query::Target> for Target {
    type Error = anyhow::Error;

    fn try_from(value: query::Target) -> Result<Self, Self::Error> {
        match value.r#type() {
            query::target::Discriminator::Rule => Ok(Self::Rule(
                value
                    .rule
                    .ok_or(anyhow::anyhow!("rule field is not set."))?,
            )),
            query::target::Discriminator::SourceFile => Ok(Self::SourceFile(
                value
                    .source_file
                    .ok_or(anyhow::anyhow!("source_file field is not set."))?,
            )),
            query::target::Discriminator::GeneratedFile => {
                Ok(Self::GeneratedFile(value.generated_file.ok_or(
                    anyhow::anyhow!("generated_file field is not set."),
                )?))
            }
            query::target::Discriminator::PackageGroup => {
                Ok(Self::PackageGroup(value.package_group.ok_or(
                    anyhow::anyhow!("package_group field is not set."),
                )?))
            }
            query::target::Discriminator::EnvironmentGroup => Err(anyhow::anyhow!("not supported")),
        }
    }
}

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
        match self {
            Target::SourceFile(source_file) => heap.alloc_simple(source_file),
            Target::GeneratedFile(generated_file) => heap.alloc_simple(generated_file),
            Target::Rule(rule) => heap.alloc_simple(rule),
            Target::PackageGroup(package_group) => heap.alloc_simple(package_group),
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

/// Build the error for a `bazel query` that exited non-zero.
///
/// A failed query (bad expression, BUILD evaluation error, …) must not
/// be mistaken for one that legitimately matched nothing, so the error
/// carries Bazel's own `stderr` when it's available. `stderr` is
/// best-effort: it's trimmed and appended only when non-empty, so a
/// query that died without writing diagnostics still yields a usable
/// exit-code message.
fn query_failure_error(expr: &str, exit_code: Option<i32>, stderr: &str) -> anyhow::Error {
    let stderr = stderr.trim();
    let detail = if stderr.is_empty() {
        String::new()
    } else {
        format!("\n{stderr}")
    };
    anyhow!("bazel query failed with exit code {exit_code:?}: {expr}{detail}")
}

/// Run `bazel query <expr>` with the given startup + command flags and decode
/// the streamed-proto result into a `Query`. Fails with Bazel's own stderr
/// on a non-zero exit — a failed query is not the same as one that matched
/// nothing. Used by `ctx.bazel.query(expr, rc=…)`. Forking goes through the
/// `BazelBackend` so the fake backend can answer queries in tests.
pub fn run(
    backend: &super::backend::BazelBackend,
    expr: &str,
    startup_flags: &[String],
    flags: &[String],
    announce: super::build::AnnounceSpawn,
) -> anyhow::Result<Query> {
    let mut cmd = backend.base_command(startup_flags);
    cmd.arg("query");
    cmd.arg(expr);
    cmd.args(flags);
    cmd.arg("--output=streamed_proto");
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
            backend
                .server_info(startup_flags)
                .ok()
                .and_then(|(_pid, version)| version)
        } else {
            None
        };
        super::build::announce_spawn(announce, version.as_ref(), &cmd);
    }

    // Register with the live-bazel registry so a CI cancel
    // (SIGINT/SIGTERM to aspect-cli) escalates to the bazel
    // client. Large queries can run for many seconds; without
    // registration they'd outlive an aborted aspect-cli.
    let (mut child, _guard) =
        super::live::spawn_registered(&mut cmd).with_context(|| "failed to spawn bazel")?;
    let status = child.wait()?;

    if !status.success() {
        let mut stderr = String::new();
        if let Ok(mut f) = File::open(&errfile_path) {
            let _ = f.read_to_string(&mut stderr);
        }
        return Err(query_failure_error(expr, status.code(), &stderr));
    }

    let mut buf = vec![];
    File::open(&out)?.read_to_end(&mut buf)?;

    let mut buf2 = Bytes::from(buf);
    let mut targets = vec![];
    loop {
        let target = query::Target::decode_length_delimited(&mut buf2);
        match target {
            Ok(target) => {
                let target = target.try_into();
                if target.is_ok() {
                    targets.push(target.unwrap());
                }
            }
            // TODO: only break if error is EOF
            Err(_) => break,
        }
    }
    Ok(Query { targets })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_failure_error_includes_stderr_when_present() {
        let err = query_failure_error("kind('x', //...)", Some(2), "ERROR: no such package\n");
        let msg = format!("{err}");
        assert!(msg.contains("exit code Some(2)"), "{msg}");
        assert!(msg.contains("kind('x', //...)"), "{msg}");
        assert!(msg.contains("ERROR: no such package"), "{msg}");
    }

    #[test]
    fn query_failure_error_omits_empty_stderr() {
        let err = query_failure_error("//...", Some(1), "   \n  ");
        let msg = format!("{err}");
        // No dangling separator/newline when there are no diagnostics.
        assert!(msg.ends_with("//..."), "{msg}");
    }

    #[test]
    fn query_failure_error_handles_unknown_exit_code() {
        // A signal-killed child reports no exit code.
        let err = query_failure_error("//...", None, "");
        assert!(format!("{err}").contains("exit code None"));
    }
}
