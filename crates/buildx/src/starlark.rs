//! The script surface: `buildx_main(impl = ..., cli_args = {})` and the
//! `Context` handed to `impl`, shaped after BXL.

use crate::bindings::v9_0_1 as bazel;
use crate::engine::Engine;
use crate::engine::Live;
use crate::workspace;
use allocative::Allocative;
use anyhow::{Context as _, Result, anyhow, bail};
use starlark::environment::{
    GlobalsBuilder, LibraryExtension, Methods, MethodsBuilder, MethodsStatic, Module,
};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::dict::UnpackDictEntries;
use starlark::values::none::NoneType;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::structs::AllocStruct;
use starlark::values::tuple::UnpackTuple;
use starlark::values::{
    Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Value, ValueLike, starlark_value,
};
use starlark::{starlark_module, starlark_simple_value};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Runs `path/to/file.star:name` and returns the exit code.
pub fn run(spec: &str) -> Result<i32> {
    let Some((path, name)) = spec.rsplit_once(':') else {
        bail!("expected path/to/file.star:name, got {spec:?}");
    };
    let src = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let ast = AstModule::parse(path, src, &Dialect::Extended).map_err(starlark_error)?;
    let mut builder = GlobalsBuilder::extended_by(&[
        LibraryExtension::Print,
        LibraryExtension::Pprint,
        LibraryExtension::StructType,
        LibraryExtension::RecordType,
        LibraryExtension::EnumType,
        LibraryExtension::Typing,
        LibraryExtension::Json,
    ]);
    register_globals(&mut builder);
    let globals = builder.build();
    bazel::register();
    let workspace = workspace::find(&std::env::current_dir()?)?;
    Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.eval_module(ast, &globals).map_err(starlark_error)?;
        let main = module
            .get(name)
            .ok_or_else(|| anyhow!("{path} does not define `{name}`"))?;
        let imp = main
            .get_attr("impl", eval.heap())
            .map_err(starlark_error)?
            .ok_or_else(|| anyhow!("`{name}` is not a buildx_main(...)"))?;
        let ctx = eval.heap().alloc(Context::new(workspace));
        let ret = eval
            .eval_function(imp, &[ctx], &[])
            .map_err(starlark_error)?;
        if ret.is_none() {
            return Ok(0);
        }
        ret.unpack_i32()
            .ok_or_else(|| anyhow!("impl returned {}, expected an int or None", ret.to_repr()))
    })
}

fn starlark_error(e: starlark::Error) -> anyhow::Error {
    anyhow!("{e}")
}

#[starlark_module]
fn register_globals(builder: &mut GlobalsBuilder) {
    const Context: StarlarkValueAsType<Context> = StarlarkValueAsType::new();
    const Analysis: StarlarkValueAsType<Analysis> = StarlarkValueAsType::new();
    const Java: StarlarkValueAsType<Java> = StarlarkValueAsType::new();

    /// Declares a script entry point: `main = buildx_main(impl = _impl, cli_args = {})`.
    fn buildx_main<'v>(
        #[starlark(kwargs)] kwargs: UnpackDictEntries<&'v str, Value<'v>>,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let mut imp = None;
        let mut cli_args = None;
        for (k, v) in kwargs.entries {
            match k {
                "impl" => imp = Some(v),
                "cli_args" => cli_args = Some(v),
                other => bail!("buildx_main: unexpected argument `{other}`"),
            }
        }
        let imp = imp.ok_or_else(|| anyhow!("buildx_main: `impl` is required"))?;
        let cli_args = cli_args.ok_or_else(|| anyhow!("buildx_main: `cli_args` is required"))?;
        let n = cli_args.length().map_err(starlark_error)?;
        if n != 0 {
            bail!("buildx_main: script arguments are not supported yet; pass cli_args = {{}}");
        }
        Ok(heap.alloc(AllocStruct(vec![
            ("impl".to_string(), imp),
            ("cli_args".to_string(), cli_args),
        ])))
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Context {
    #[allocative(skip)]
    workspace: PathBuf,
    #[allocative(skip)]
    engine: Mutex<Option<Arc<Engine>>>,
}

impl Context {
    pub fn new(workspace: PathBuf) -> Context {
        Context {
            workspace,
            engine: Mutex::new(None),
        }
    }

    /// The engine starts on first use and lives for the rest of the script.
    pub fn engine(&self) -> Result<Arc<Engine>> {
        let mut slot = self
            .engine
            .lock()
            .map_err(|_| anyhow!("engine lock poisoned"))?;
        if let Some(e) = slot.as_ref() {
            return Ok(e.clone());
        }
        let engine = Arc::new(workspace::start(&self.workspace).map_err(flatten)?);
        *slot = Some(engine.clone());
        Ok(engine)
    }
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Context({})", self.workspace.display())
    }
}

starlark_simple_value!(Context);

#[starlark_value(type = "Context")]
impl<'v> StarlarkValue<'v> for Context {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("context_methods", context_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn context_methods(registry: &mut MethodsBuilder) {
    /// Loading-phase questions about the workspace; stands in for uquery, cquery and analysis.
    fn analysis(this: &Context) -> anyhow::Result<Analysis> {
        Ok(Analysis {
            engine: this.engine()?,
        })
    }

    /// Where a script writes its results; `print` goes to stderr.
    #[starlark(attribute)]
    fn output(this: &Context) -> anyhow::Result<Output> {
        let _ = this;
        Ok(Output)
    }

    /// Live handles into the hosted JVM: `runtime`.
    #[starlark(attribute)]
    fn java(this: &Context) -> anyhow::Result<Java> {
        Ok(Java {
            engine: this.engine()?,
        })
    }

    /// Script arguments. Empty until `cli_args` is supported.
    #[starlark(attribute)]
    fn cli_args<'v>(this: &Context, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        Ok(heap.alloc(AllocStruct::EMPTY))
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Output;

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Output")
    }
}

starlark_simple_value!(Output);

#[starlark_value(type = "Output")]
impl<'v> StarlarkValue<'v> for Output {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("output_methods", output_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn output_methods(registry: &mut MethodsBuilder) {
    /// Writes the arguments to stdout, space separated, with a newline.
    fn print<'v>(
        this: &Output,
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
    ) -> anyhow::Result<NoneType> {
        let _ = this;
        let mut line = String::new();
        for (i, v) in args.items.iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            v.collect_str(&mut line);
        }
        println!("{line}");
        Ok(NoneType)
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Analysis {
    #[allocative(skip)]
    engine: Arc<Engine>,
}

impl fmt::Display for Analysis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Analysis")
    }
}

starlark_simple_value!(Analysis);

#[starlark_value(type = "Analysis")]
impl<'v> StarlarkValue<'v> for Analysis {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("analysis_methods", analysis_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn analysis_methods(registry: &mut MethodsBuilder) {
    /// Evaluates a Bazel query expression on the heap and returns the live
    /// result set, in label order. Each element is a generated `Rule` or
    /// `InputFile` Starlark value.
    fn query<'v>(this: &Analysis, expr: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let set = this.engine.query(expr).map_err(flatten)?;
        live(&set, heap)
    }

    /// The live `Package` for a label like `//lib`, loaded on demand.
    fn package<'v>(this: &Analysis, label: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let pkg = this.engine.package(label).map_err(flatten)?;
        live(&pkg, heap)
    }
}

/// A live object as the generated Starlark type for its class.
fn live<'v>(obj: &Live, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
    let mut env = blazon_rt::vm::env()?;
    let obj = obj.get(&mut env)?;
    Ok(blazon_starlark::wrap(obj.as_ref(), heap)?)
}

/// `ctx.java`: entry points into the hosted JVM.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Java {
    #[allocative(skip)]
    engine: Arc<Engine>,
}

impl fmt::Display for Java {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Java")
    }
}

starlark_simple_value!(Java);

#[starlark_value(type = "Java")]
impl<'v> StarlarkValue<'v> for Java {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("java_methods", java_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn java_methods(registry: &mut MethodsBuilder) {
    /// The live `BlazeRuntime`.
    #[starlark(attribute)]
    fn runtime<'v>(this: &Java, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let mut env = blazon_rt::vm::env()?;
        let runtime = this.engine.runtime().get(&mut env)?;
        Ok(runtime.to_value(heap)?)
    }
}

pub fn serr(e: anyhow::Error) -> starlark::Error {
    starlark::Error::new_kind(starlark::ErrorKind::Function(flatten(e)))
}

/// One message carrying the whole cause chain, since Starlark shows only the
/// outermost error. A Java stack trace at the root is cut to its first frames.
pub fn flatten(e: anyhow::Error) -> anyhow::Error {
    let chain: Vec<String> = e.chain().map(|c| c.to_string()).collect();
    if std::env::var_os("BUILDX_TRACE").is_some() {
        if let Some(root) = chain.last() {
            eprintln!("{root}");
        }
    }
    let n = chain.len();
    let mut out = String::new();
    for (i, msg) in chain.iter().enumerate() {
        let lines: Vec<&str> = msg
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let msg = if i + 1 == n && lines.len() > 6 {
            format!("{} ...", lines[..6].join(" | "))
        } else {
            lines.join(" | ")
        };
        if i > 0 && n > 1 && i + 1 == n && chain[i - 1].lines().next() == msg.lines().next() {
            continue;
        }
        if !out.is_empty() {
            out.push_str(" <- ");
        }
        out.push_str(&msg);
    }
    anyhow!("{out}")
}
