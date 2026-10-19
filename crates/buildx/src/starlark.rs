//! The script surface: `buildx_main(impl = ..., cli_args = {})` and the
//! `Context` handed to `impl`, shaped after BXL.

use crate::bindings::v9_0_1 as bazel;
use crate::engine::Live;
use crate::engine::{Engine, Half, Spec};
use crate::route::RouteExecutor;
use crate::settings;
use crate::values::{
    BuildResult, Config, Execution, ExecutorValue, Failure, FrozenHeapHolder, Given, Outputs,
    Providers,
};
use crate::workspace;
use allocative::Allocative;
use anyhow::{Context as _, Result, anyhow, bail};
use starlark::environment::{
    GlobalsBuilder, LibraryExtension, Methods, MethodsBuilder, MethodsStatic, Module,
};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::dict::{AllocDict, DictRef, UnpackDictEntries};
use starlark::values::list::ListRef;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::structs::AllocStruct;
use starlark::values::tuple::TupleRef;
use starlark::values::tuple::UnpackTuple;
use starlark::values::{
    Heap, NoSerialize, ProvidesStaticType, StarlarkValue, UnpackValue, Value, ValueLike,
    starlark_value,
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
    // Frozen, so that top-level functions can be handed to Bazel's threads.
    let frozen = Module::with_temp_heap(|module| {
        {
            let mut eval = Evaluator::new(&module);
            eval.eval_module(ast, &globals).map_err(starlark_error)?;
        }
        module.freeze().map_err(|e| anyhow!("{e:?}"))
    })?;
    let main = frozen
        .get(name)
        .map_err(|_| anyhow!("{path} does not define `{name}`"))?;
    // SAFETY: `main` owns the frozen heap and outlives the call below.
    let main_value = unsafe { main.unchecked_frozen_value() };
    let heap = frozen.frozen_heap().clone();
    Module::with_temp_heap(|module| {
        let main = main_value.to_value();
        let mut eval = Evaluator::new(&module);
        let imp = main
            .get_attr("impl", eval.heap())
            .map_err(starlark_error)?
            .ok_or_else(|| anyhow!("`{name}` is not a buildx_main(...)"))?;
        let ctx = eval.heap().alloc(Context::new(workspace, heap, spec));
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
    const Config: StarlarkValueAsType<Config> = StarlarkValueAsType::new();

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

fn setting_value(v: Value) -> anyhow::Result<settings::Value> {
    if let Some(b) = v.unpack_bool() {
        return Ok(settings::Value::Bool(b));
    }
    if let Some(i) = v.unpack_i32() {
        return Ok(settings::Value::Int(i64::from(i)));
    }
    if let Some(s) = v.unpack_str() {
        return Ok(settings::Value::Str(s.to_string()));
    }
    if v.get_type() == "float" {
        let f: f64 = v
            .to_repr()
            .parse()
            .map_err(|_| anyhow!("bad float {}", v.to_repr()))?;
        return Ok(settings::Value::Float(f));
    }
    if let Some(d) = DictRef::from_value(v) {
        let mut entries = Vec::new();
        for (k, item) in d.iter() {
            let key = k.unpack_str().ok_or_else(|| {
                anyhow!(
                    "dict keys of a config setting must be strings, got {}",
                    k.get_type()
                )
            })?;
            entries.push((key.to_string(), setting_value(item)?));
        }
        return Ok(settings::Value::Dict(entries));
    }
    if let Some(items) = ListRef::from_value(v) {
        return Ok(settings::Value::List(
            items
                .iter()
                .map(setting_value)
                .collect::<anyhow::Result<_>>()?,
        ));
    }
    if let Some(items) = TupleRef::from_value(v) {
        return Ok(settings::Value::List(
            items
                .iter()
                .map(setting_value)
                .collect::<anyhow::Result<_>>()?,
        ));
    }
    bail!("a config setting cannot be a {}", v.get_type())
}

/// `starlark = {"//pkg:flag": value}`: bool, int, string or list of strings values.
fn starlark_settings(v: Value) -> anyhow::Result<Vec<(String, String)>> {
    let dict = UnpackDictEntries::<String, Value>::unpack_value(v)
        .map_err(starlark_error)?
        .ok_or_else(|| anyhow!("config setting `starlark` expects a dict of labels to values"))?;
    let mut out = Vec::new();
    for (label, value) in dict.entries {
        let text = if let Some(b) = value.unpack_bool() {
            b.to_string()
        } else if let Some(i) = value.unpack_i32() {
            i.to_string()
        } else if let Some(s) = value.unpack_str() {
            s.to_string()
        } else if let Some(l) = UnpackList::<String>::unpack_value(value).map_err(starlark_error)? {
            l.items.join(",")
        } else {
            bail!(
                "config setting `starlark` entry `{label}` must be a bool, int, string or list of strings, got {}",
                value.get_type()
            );
        };
        out.push((label, text));
    }
    Ok(out)
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Context {
    #[allocative(skip)]
    workspace: PathBuf,
    #[allocative(skip)]
    engine: Mutex<Option<Arc<Engine>>>,
    #[allocative(skip)]
    heap: FrozenHeapHolder,
    script: String,
}

impl Context {
    pub fn new(workspace: PathBuf, heap: starlark::values::FrozenHeapRef, script: &str) -> Context {
        Context {
            script: script.to_string(),
            workspace,
            engine: Mutex::new(None),
            heap: FrozenHeapHolder(heap),
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
        let _starting = crate::phases::start("engine start");
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

    /// A configuration from typed settings named like Bazel's options without
    /// the dashes: `compilation_mode = "opt"`, `copt = ["-O3"]`,
    /// `action_env = {"A": "b"}`, ..., plus `starlark = {"//pkg:flag": value}`
    /// for build settings. Every setting is checked against the release's
    /// option schema and parsed by the engine's own option parser;
    /// `ctx.config().describe(name)` says what a setting is.
    fn config<'v>(
        this: &Context,
        #[starlark(kwargs)] kwargs: UnpackDictEntries<&'v str, Value<'v>>,
    ) -> anyhow::Result<Config> {
        let mut given = Vec::new();
        let mut starlark = Vec::new();
        let mut arguments = Vec::new();
        let mut fragment_arguments = Vec::new();
        for (name, v) in kwargs.entries {
            if name == "starlark" {
                starlark = starlark_settings(v)?;
                continue;
            }
            let (def, renamed) = settings::lookup(name)?;
            for warning in renamed.into_iter().chain(settings::deprecation(def)) {
                eprintln!("WARNING: {warning}");
            }
            let value = setting_value(v)?;
            let args = settings::arguments(def, &value)?;
            if def.fragment {
                fragment_arguments.extend(args.iter().cloned());
            }
            arguments.extend(args);
            given.push(Given {
                name: def.name.to_string(),
                value,
                fragment: def.fragment,
            });
        }
        let repo_env: Vec<(String, String)> = given
            .iter()
            .find(|g| g.name == "repo_env")
            .map(|g| match &g.value {
                settings::Value::Dict(d) => d
                    .iter()
                    .filter_map(|(k, v)| match v {
                        settings::Value::Str(s) => Some((k.clone(), s.clone())),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
        let mut engine_half: Vec<String> = given
            .iter()
            .filter(|g| !g.fragment)
            .map(|g| format!("{}={}", g.name, g.value.repr()))
            .collect();
        engine_half.sort();
        let fingerprint = engine_half.join("\n");
        let starlark_arguments: Vec<String> =
            starlark.iter().map(|(k, v)| format!("--{k}={v}")).collect();
        let inner = this
            .engine()?
            .config(
                &arguments,
                &fragment_arguments,
                &starlark_arguments,
                Half {
                    fingerprint,
                    repo_env,
                },
            )
            .map_err(|e| anyhow!("{}", settings::strip_dashes(&format!("{:#}", flatten(e)))))?;
        Ok(Config::new(given, starlark, inner))
    }

    /// Builds the targets (label or pattern strings, live targets, or whole
    /// query results) in `config` and blocks until done. Returns a dict of
    /// label to `BuildResult`; a target that fails is data in its result,
    /// not an exception. Nothing is downloaded or uploaded.
    fn build<'v>(
        this: &Context,
        targets: Value<'v>,
        #[starlark(require = named)] config: Option<&Config>,
        #[starlark(require = named)] executor: Option<Value<'v>>,
        #[starlark(require = named)] events: Option<UnpackList<&'v crate::events::SinkValue>>,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let sinks: Vec<std::sync::Arc<crate::events::Sink>> = events
            .map(|l| l.items.iter().map(|s| s.0.clone()).collect())
            .unwrap_or_default();
        let router: std::sync::Arc<dyn crate::execution::Router> = match executor {
            None => crate::execution::default_router(),
            Some(e) if e.is_none() => crate::execution::default_router(),
            Some(e) => {
                if let Some(local) = e.downcast_ref::<ExecutorValue>() {
                    std::sync::Arc::new(crate::execution::Fixed(local.0.clone()))
                } else if let Some(route) = e.downcast_ref::<RouteExecutor>() {
                    route.router.0.clone()
                } else if e.get_type() == "function" {
                    Execution::new(this.heap.0.clone()).router_of(e, Vec::new())?
                } else {
                    bail!(
                        "executor {} is not implemented; use ctx.execution.sandbox(), ctx.execution.local(), ctx.execution.route(fn) or a function",
                        e.get_type()
                    );
                }
            }
        };
        let engine = this.engine()?;
        let config_settings = config.map(|c| c.settings());
        let config = match config {
            Some(c) => c.engine_config().clone(),
            None => engine.default_config().map_err(flatten)?,
        };
        let spec = targets_of(targets, heap)?;
        let mut used = crate::events::Used::default();
        router.describe(&mut used);
        let invocation = crate::events::Invocation {
            used,
            script: this.script.clone(),
            settings: config_settings.unwrap_or_default(),
            targets: spec
                .iter()
                .map(|s| match s {
                    Spec::Pattern(p) => p.clone(),
                    Spec::Live(_) => "<live target>".to_string(),
                })
                .collect(),
        };
        let batch = std::sync::Arc::new(
            engine
                .build(&spec, &config, router, &sinks, &invocation)
                .map_err(flatten)?,
        );
        let wrapping = crate::phases::start("script: BuildResult values");
        let mut entries = Vec::with_capacity(batch.len());
        let mut env = blazon_rt::vm::env()?;
        for (i, label) in batch.labels.iter().enumerate() {
            let label = label.get(&mut env)?.to_value(heap)?;
            entries.push((label, heap.alloc(BuildResult::new(batch.clone(), i))));
        }
        drop(wrapping);
        crate::phases::report();
        Ok(heap.alloc(AllocDict(entries)))
    }

    /// Materialization: `ensure`.
    #[starlark(attribute)]
    fn outputs(this: &Context) -> anyhow::Result<Outputs> {
        let _ = this;
        Ok(Outputs)
    }

    /// Event sinks for `build`: `bes`, `file`.
    #[starlark(attribute)]
    fn events(this: &Context) -> anyhow::Result<crate::events::Events> {
        let _ = this;
        Ok(crate::events::Events)
    }

    /// Executors: `local()`, `sandbox()`, `remote`, `cached`, `route`.
    #[starlark(attribute)]
    fn execution(this: &Context) -> anyhow::Result<Execution> {
        Ok(Execution::new(this.heap.0.clone()))
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
    /// `InputFile` Starlark value. With `config`, the graph is synced to its
    /// loading, repository and Starlark-semantics settings first.
    fn query<'v>(
        this: &Analysis,
        expr: &str,
        #[starlark(require = named)] config: Option<&Config>,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let set = match config {
            Some(c) => this.engine.query_in(expr, c.engine_config()),
            None => this.engine.query(expr),
        }
        .map_err(flatten)?;
        live(&set, heap)
    }

    /// Configures and analyzes the targets (label or pattern strings, live
    /// targets, or whole query results) in `config`, default settings when
    /// None. Returns a dict of label to the live configured target, or to a
    /// `Failure` for a target whose analysis failed; the others still analyze.
    fn analyze<'v>(
        this: &Analysis,
        targets: Value<'v>,
        #[starlark(require = named)] config: Option<&Config>,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let config = match config {
            Some(c) => c.engine_config().clone(),
            None => this.engine.default_config().map_err(flatten)?,
        };
        let spec = targets_of(targets, heap)?;
        let analyzed = this.engine.analyze(&spec, &config).map_err(flatten)?;
        let mut env = blazon_rt::vm::env()?;
        let mut entries = Vec::new();
        for a in analyzed {
            let label = a.label.get(&mut env)?.to_value(heap)?;
            let value = match (&a.target, a.failure) {
                (Some(t), _) => t.get(&mut env)?.to_value(heap)?,
                (None, message) => {
                    heap.alloc(Failure::new(a.label.clone(), message.unwrap_or_default()))
                }
            };
            entries.push((label, value));
        }
        Ok(heap.alloc(AllocDict(entries)))
    }

    /// Provider keys for indexing analyzed targets.
    #[starlark(attribute)]
    fn providers(this: &Analysis) -> anyhow::Result<Providers> {
        let _ = this;
        Ok(Providers)
    }

    /// The live `Package` for a label like `//lib`, loaded on demand.
    fn package<'v>(this: &Analysis, label: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let pkg = this.engine.package(label).map_err(flatten)?;
        live(&pkg, heap)
    }
}

/// The targets a script value names: a label or pattern string (run as a query
/// when the call has synced its graph), a generated target value, or an iterable
/// of either, such as a query result.
fn targets_of<'v>(v: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Vec<Spec>> {
    let mut out = Vec::new();
    collect_targets(v, heap, &mut out)?;
    if out.is_empty() {
        bail!("no targets given");
    }
    Ok(out)
}

fn collect_targets<'v>(v: Value<'v>, heap: Heap<'v>, out: &mut Vec<Spec>) -> anyhow::Result<()> {
    if let Some(expr) = v.unpack_str() {
        out.push(Spec::Pattern(expr.to_string()));
        return Ok(());
    }
    let single = {
        let mut env = blazon_rt::vm::env()?;
        if let Some(t) = v.downcast_ref::<bazel::RuleValue>() {
            Some(t.obj.get(&mut env)?.upcast::<bazel::Object>().to_global()?)
        } else if let Some(t) = v.downcast_ref::<bazel::InputFileValue>() {
            Some(t.obj.get(&mut env)?.upcast::<bazel::Object>().to_global()?)
        } else if let Some(t) = v.downcast_ref::<bazel::OutputFileValue>() {
            Some(t.obj.get(&mut env)?.upcast::<bazel::Object>().to_global()?)
        } else if let Some(t) = v.downcast_ref::<bazel::PackageGroupValue>() {
            Some(t.obj.get(&mut env)?.upcast::<bazel::Object>().to_global()?)
        } else {
            None
        }
    };
    if let Some(t) = single {
        out.push(Spec::Live(t));
        return Ok(());
    }
    let items = v.iterate(heap).map_err(|_| {
        anyhow!(
            "expected a target, a label string or an iterable of them, got {}",
            v.get_type()
        )
    })?;
    for item in items {
        collect_targets(item, heap, out)?;
    }
    Ok(())
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
