use std::env::temp_dir;
use std::fs;
use std::fs::File;

use std::cell::RefCell;
use std::io::Read;
use std::process::Stdio;

use anyhow::Context;

use either::Either;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;

use prost::bytes::Bytes;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::StarlarkResultExt;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::list::UnpackList;
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

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<bazel.query.TargetSet {targets:?}>")]
pub struct TargetSet {
    #[allocative(skip)]
    targets: Vec<Target>,
}

impl<'v> AllocValue<'v> for TargetSet {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "bazel.query.TargetSet")]
impl<'v> values::StarlarkValue<'v> for TargetSet {
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

#[derive(Debug, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<bazel.query.Query>")]
pub struct Query {
    #[allocative(skip)]
    expr: RefCell<String>,
    #[allocative(skip)]
    startup_flags: Vec<String>,
    #[allocative(skip)]
    flags: Vec<String>,
}

impl Query {
    pub fn new(startup_flags: Vec<String>) -> Self {
        Self {
            expr: RefCell::new(String::new()),
            startup_flags,
            flags: vec![],
        }
    }

    pub fn query(
        expr: &str,
        startup_flags: &[String],
        flags: &[String],
        announce: super::build::AnnounceSpawn,
    ) -> anyhow::Result<TargetSet> {
        let mut cmd = super::bazel_command();
        cmd.args(startup_flags);
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
                super::info::server_info_with_startup_flags(startup_flags)
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
        Ok(TargetSet { targets })
    }
}

impl<'v> AllocValue<'v> for Query {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.query.Query")]
/// The entry point for programmatic queries, providing methods to construct initial target sets.
///
/// This builder allows creating starting points for queries, such as target patterns or explicit
/// label sets. All operations return a `TargetSet` which can be further composed.
impl<'v> values::StarlarkValue<'v> for Query {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(query_methods)
    }
}

#[starlark_module]
pub(crate) fn query_methods(registry: &mut MethodsBuilder) {
    /// Replaces the query `expression` with a raw query expression string.
    ///
    /// This escape hatch allows direct use of the underlying query language for complex cases,
    /// while still supporting further chaining.
    ///
    /// ```starlark
    /// **Complex** intersection query
    /// complex = ctx.bazel.query().raw("deps(//foo) intersect kind('test', //bar:*)")
    ///
    /// **Path**-based query
    /// path_query = ctx.bazel.query().raw("somepath(//start, //end)")
    ///
    /// **Chaining** after raw
    /// filtered = complex.kind("source file")
    /// ```
    fn raw<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] expr: values::StringValue,
    ) -> anyhow::Result<Query> {
        let query = this.downcast_ref_err::<Query>().into_anyhow_result()?;
        Ok(Query {
            expr: RefCell::new(expr.as_str().to_string()),
            startup_flags: query.startup_flags.clone(),
            flags: query.flags.clone(),
        })
    }

    /// The query system provides a programmatic interface for analyzing build dependencies
    /// and target relationships. Queries are constructed using a chain API and are lazily
    /// evaluated only when `.eval()` is explicitly called.
    ///
    /// The entry point is `ctx.bazel.query()`, which returns a `query` for creating initial
    /// query expressions. Most operations operate on `query` objects, which represent
    /// sets of targets that can be filtered, transformed, and combined.
    ///
    /// **Example**
    ///
    /// ```starlark
    /// **Query** dependencies of a target
    /// deps = ctx.bazel.query().targets("//myapp:main").deps()
    /// all_deps: target_set = deps.eval()
    ///
    /// **Chain** multiple operations
    /// sources = ctx.bazel.query().targets("//myapp:main")
    ///     .deps()
    ///     .kind("source file")
    ///     .eval()
    /// ```
    ///
    /// Fails with Bazel's own stderr if the query exits non-zero (bad
    /// expression, BUILD evaluation error, …), rather than returning an
    /// empty target set — a failed query is not the same as one that
    /// matched nothing.
    ///
    /// # Arguments
    /// * `flags` - Command flags to pass to `bazel query` (between the
    ///   expression and `--output`). Callers that run `query` alongside
    ///   `build` / `test` under `--ignore_all_rc_files` MUST forward the
    ///   rc-expanded command flags here, so the query resolves external
    ///   repositories the same way the build does (e.g. an rc-set
    ///   `--noenable_bzlmod` / `--enable_workspace`). Omitting them lets the
    ///   query diverge from the build and fail on repos the build can see.
    ///   Accepts the same `str | (str, version-constraint)` shape as
    ///   `ctx.bazel.build` / `.test`: a conditional `(flag, constraint)`
    ///   tuple is resolved against the running Bazel version and dropped
    ///   when the constraint does not hold — so a flag gated to a different
    ///   Bazel version is filtered out here exactly as it would be on the
    ///   build, rather than being forced onto the query.
    /// * `announce_version` - Print an `INFO: Bazel <version>` line before
    ///   spawning. Resolved from the `--announce-bazel-version` task flag.
    /// * `announce_command` - Print an `INFO: Spawning: <command>` line
    ///   before spawning. Resolved from the `--announce-bazel-command` task
    ///   flag. Both mirror the `ctx.bazel.build` / `.test` disclosure.
    fn eval<'v>(
        this: values::Value<'v>,
        #[starlark(require = named, default = UnpackList::default())] flags: UnpackList<
            Either<values::StringValue<'v>, (values::StringValue<'v>, values::StringValue<'v>)>,
        >,
        #[starlark(require = named, default = false)] announce_version: bool,
        #[starlark(require = named, default = false)] announce_command: bool,
    ) -> anyhow::Result<TargetSet> {
        let this = this.downcast_ref_err::<Query>().into_anyhow_result()?;
        let expr = this.expr.borrow();
        // Flags passed to `.eval()` win over any carried on the builder, but
        // fall back to the builder's so the common case still works. Resolve
        // conditional `(flag, constraint)` tuples against the running Bazel
        // version the same way `ctx.bazel.build` does (see `resolve_flags`),
        // so a version-gated flag is filtered out — not force-applied — on a
        // mismatch. Only pay for the `bazel info` version probe when a
        // conditional flag is actually present.
        let flags = if flags.items.is_empty() {
            this.flags.clone()
        } else {
            let has_conditional = flags.items.iter().any(|f| f.is_right());
            let bazel_version = if has_conditional {
                let (_, version) = super::info::server_info()
                    .map_err(|e| anyhow!("failed to get Bazel server info: {}", e))?;
                version
            } else {
                None
            };
            super::resolve_flags(&flags.items, bazel_version.as_ref())?
        };
        let announce = super::build::AnnounceSpawn {
            version: announce_version,
            command: announce_command,
        };
        Query::query(&expr, &this.startup_flags, &flags, announce)
    }
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
