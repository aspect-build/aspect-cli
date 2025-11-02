use std::cell::RefCell;
use std::env::temp_dir;
use std::fs;
use std::fs::File;

use std::io::Read;
use std::process::Command;
use std::process::Stdio;
use std::rc::Rc;

use allocative::Allocative;
use anyhow::anyhow;
use derive_more::Display;
use dupe::Dupe;
use prost::bytes::Bytes;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;

use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::typing::Ty;
use starlark::values;
use starlark::values::starlark_value;
use starlark::values::type_repr::StarlarkTypeRepr;
use starlark::values::AllocValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;

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
    fn alloc_value(self, heap: &'v starlark::values::Heap) -> starlark::values::Value<'v> {
        match self {
            Target::SourceFile(source_file) => heap.alloc_simple(source_file),
            Target::GeneratedFile(generated_file) => heap.alloc_simple(generated_file),
            Target::Rule(rule) => heap.alloc_simple(rule),
            Target::PackageGroup(package_group) => heap.alloc_simple(package_group),
        }
    }
}

#[derive(Debug, ProvidesStaticType, Display, Trace, NoSerialize, Allocative)]
#[display("<target_set {targets:?}>")]
pub struct TargetSet {
    #[allocative(skip)]
    targets: Vec<Target>,
}

impl<'v> AllocValue<'v> for TargetSet {
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_simple(self)
    }
}

#[starlark_value(type = "target_set")]
impl<'v> values::StarlarkValue<'v> for TargetSet {
    fn get_type_starlark_repr() -> Ty {
        Ty::iter(Target::starlark_type_repr())
    }

    fn length(&self) -> starlark::Result<i32> {
        Ok(self.targets.len() as i32)
    }

    fn at(&self, index: values::Value<'v>, heap: &'v Heap) -> starlark::Result<values::Value<'v>> {
        let idx = index.unpack_i32().ok_or(anyhow!("pass an int"))?;
        let target = self
            .targets
            .get(idx as usize)
            .map(|t| heap.alloc(t.clone()))
            .ok_or(anyhow!("no target at index {idx}"))?;
        Ok(target)
    }

    fn iterate_collect(&self, heap: &'v Heap) -> starlark::Result<Vec<values::Value<'v>>> {
        Ok(self
            .targets
            .clone()
            .into_iter()
            .map(|f| heap.alloc(f))
            .collect())
    }
}

#[derive(Dupe, Clone, Debug, Display, ProvidesStaticType, Trace, NoSerialize, Allocative)]
#[display("<query>")]
pub struct Query {
    #[allocative(skip)]
    // Expr here has to be mutable
    expr: Rc<RefCell<String>>,
}

impl Query {
    pub fn new() -> Self {
        Self {
            expr: Rc::new(RefCell::new(String::new())),
        }
    }

    pub fn query(expr: &str) -> anyhow::Result<TargetSet> {
        let mut cmd = Command::new("bazel");
        cmd.arg("query");
        cmd.arg(expr);
        cmd.arg("--output=streamed_proto");
        let out = temp_dir().join("query.bin");
        let _ = fs::remove_file(&out);

        // TODO; make it efficient
        // match nix::unistd::mkfifo(&out, Mode::S_IRWXO | Mode::S_IRWXU | Mode::S_IRWXG) {
        //     Ok(_) => {}
        //     Err(_) => todo!("failed to create pipe, implement the fallback mechanism"),
        // };

        let outfile = File::create(&out)?;

        cmd.stderr(Stdio::null());
        cmd.stdout(outfile);
        cmd.stdin(Stdio::null());
        cmd.spawn()?.wait()?;

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
    fn alloc_value(self, heap: &'v Heap) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "query")]
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
    /// # Complex intersection query
    /// complex = ctx.bazel.query().raw("deps(//foo) intersect kind('test', //bar:*)")
    ///
    /// # Path-based query
    /// path_query = ctx.bazel.query().raw("somepath(//start, //end)")
    ///
    /// # Chaining after raw
    /// filtered = complex.kind("source file")
    /// ```
    fn raw<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] expr: values::StringValue,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Query> {
        use dupe::Dupe;
        let query = this.downcast_ref_err::<Query>()?;
        query.expr.replace(expr.as_str().to_string());
        Ok(query.dupe())
    }

    /// The query system provides a programmatic interface for analyzing build dependencies
    /// and target relationships. Queries are constructed using a chain API and are lazily
    /// evaluated only when `.eval()` is explicitly called.
    ///
    /// The entry point is `ctx.bazel.query()`, which returns a `query` for creating initial
    /// query expressions. Most operations operate on `query` objects, which represent
    /// sets of targets that can be filtered, transformed, and combined.
    ///
    /// # Example
    ///
    /// ```starlark
    /// # Query dependencies of a target
    /// deps = ctx.bazel.query().targets("//myapp:main").deps()
    /// all_deps: target_set = deps.eval()
    ///
    /// # Chain multiple operations
    /// sources = ctx.bazel.query().targets("//myapp:main")
    ///     .deps()
    ///     .kind("source file")
    ///     .eval()
    /// ```
    fn eval<'v>(this: values::Value<'v>) -> anyhow::Result<TargetSet> {
        let this = this.downcast_ref_err::<Query>()?;
        let expr = this.expr.borrow();
        Query::query(expr.as_str())
    }
}
