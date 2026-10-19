//! Hand-written Starlark values of the script surface that are not Bazel's
//! own classes: `Config`, `Providers`, `BuildResult`, `Outputs` and
//! `Execution`. Names visible to scripts are snake_case.

use crate::bindings::v9_0_1 as bazel;
use crate::engine::{ArtifactInfo, Batch, EngineConfig};
use crate::execution::{Executor, Router};
use crate::route::{RouteExecutor, RouteFn, RouterRef, StarlarkRouter};
use crate::settings;
use crate::store::Store;
use allocative::Allocative;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::values::dict::{AllocDict, DictRef, UnpackDictEntries};
use starlark::values::list::AllocList;
use starlark::values::structs::AllocStruct;
use starlark::values::{
    Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Value, ValueLike, starlark_value,
};
use starlark::{starlark_module, starlark_simple_value};
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

/// One setting a script gave to `ctx.config`.
#[derive(Debug, Clone)]
pub struct Given {
    pub name: String,
    pub value: settings::Value,
    /// Declared by a configuration fragment: part of the `BuildOptions` side.
    /// The rest is the engine half (Starlark semantics, loading, repositories).
    pub fragment: bool,
}

impl Allocative for Given {
    fn visit<'a, 'b: 'a>(&self, visitor: &'a mut allocative::Visitor<'b>) {
        let _ = visitor.enter_self_sized::<Self>();
    }
}

/// `ctx.config(compilation_mode = "opt", ...)`: typed settings validated
/// against the release's option schema and parsed by the engine. Two
/// configurations holding the same settings are equal.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Config {
    given: Vec<Given>,
    starlark: Vec<(String, String)>,
    #[allocative(skip)]
    inner: Arc<EngineConfig>,
}

impl Config {
    /// The settings given, as `name=value`, sorted by name.
    pub fn settings(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .given
            .iter()
            .map(|g| format!("{}={}", g.name, g.value.repr()))
            .chain(self.starlark.iter().map(|(k, v)| format!("{k}={v}")))
            .collect();
        out.sort();
        out
    }
}

impl fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EngineConfig")
    }
}

impl Config {
    pub fn new(given: Vec<Given>, starlark: Vec<(String, String)>, inner: EngineConfig) -> Config {
        Config {
            given,
            starlark,
            inner: Arc::new(inner),
        }
    }

    pub fn engine_config(&self) -> &Arc<EngineConfig> {
        &self.inner
    }

    /// The engine half, order-independent, for equality and hashing.
    fn engine_half(&self) -> Vec<(String, String)> {
        let mut half: Vec<(String, String)> = self
            .given
            .iter()
            .filter(|g| !g.fragment)
            .map(|g| (g.name.clone(), g.value.repr()))
            .collect();
        half.sort();
        half
    }

    fn sorted_starlark(&self) -> Vec<(String, String)> {
        let mut s = self.starlark.clone();
        s.sort();
        s
    }
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Config(")?;
        let mut parts: Vec<String> = self
            .given
            .iter()
            .map(|g| format!("{} = {}", g.name, g.value.repr()))
            .collect();
        if !self.starlark.is_empty() {
            parts.push(format!(
                "starlark = {{{}}}",
                self.starlark
                    .iter()
                    .map(|(k, v)| format!("{k:?}: {v:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        f.write_str(&parts.join(", "))?;
        f.write_str(")")
    }
}

starlark_simple_value!(Config);

#[starlark_value(type = "Config")]
impl<'v> StarlarkValue<'v> for Config {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("config_methods", config_methods);
        Some(RES.methods())
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        let Some(other) = other.downcast_ref::<Config>() else {
            return Ok(false);
        };
        Ok(self
            .inner
            .same_as(&other.inner)
            .map_err(crate::starlark::serr)?
            && self.engine_half() == other.engine_half()
            && self.sorted_starlark() == other.sorted_starlark())
    }

    fn write_hash(
        &self,
        hasher: &mut starlark::collections::StarlarkHasher,
    ) -> starlark::Result<()> {
        let checksum = self.inner.checksum().map_err(crate::starlark::serr)?;
        checksum.hash(hasher);
        self.engine_half().hash(hasher);
        Ok(())
    }
}

#[starlark_module]
fn config_methods(registry: &mut MethodsBuilder) {
    /// The settings the configuration was created with, as a dict.
    #[starlark(attribute)]
    fn settings<'v>(this: &Config, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let mut entries: Vec<(Value<'v>, Value<'v>)> = Vec::new();
        for g in &this.given {
            entries.push((
                heap.alloc(g.name.as_str()),
                setting_to_starlark(&g.value, heap),
            ));
        }
        if !this.starlark.is_empty() {
            entries.push((
                heap.alloc("starlark"),
                heap.alloc(AllocDict(
                    this.starlark.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                )),
            ));
        }
        Ok(heap.alloc(AllocDict(entries)))
    }

    /// Bazel's checksum of the configuration's options; equal configurations share it.
    #[starlark(attribute)]
    fn checksum(this: &Config) -> anyhow::Result<String> {
        this.inner.checksum()
    }

    /// What a setting is: its `kind`, `default`, `help` and `effect_tags`.
    fn describe<'v>(this: &Config, name: &str, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        let (def, _) = settings::lookup(name)?;
        let default = match def.default {
            "null" => Value::new_none(),
            d => heap.alloc(settings::strip_dashes(d)),
        };
        Ok(heap.alloc(AllocStruct([
            ("name", heap.alloc(def.name)),
            ("kind", heap.alloc(settings::kind_name(&def.kind))),
            ("default", default),
            ("help", heap.alloc(settings::strip_dashes(def.help.trim()))),
            (
                "effect_tags",
                heap.alloc(AllocList(def.effect_tags.iter().map(|t| t.to_lowercase()))),
            ),
            ("deprecated", Value::new_bool(def.deprecated)),
        ])))
    }
}

fn setting_to_starlark<'v>(v: &settings::Value, heap: Heap<'v>) -> Value<'v> {
    match v {
        settings::Value::Bool(b) => Value::new_bool(*b),
        settings::Value::Int(i) => heap.alloc(*i),
        settings::Value::Float(f) => heap.alloc(*f),
        settings::Value::Str(s) => heap.alloc(s.as_str()),
        settings::Value::List(l) => {
            heap.alloc(AllocList(l.iter().map(|i| setting_to_starlark(i, heap))))
        }
        settings::Value::Dict(d) => heap.alloc(AllocDict(
            d.iter()
                .map(|(k, v)| (k.as_str(), setting_to_starlark(v, heap))),
        )),
    }
}

/// `ctx.analysis().providers`: the provider keys Bazel's Starlark names
/// `DefaultInfo`, `OutputGroupInfo` and `RunEnvironmentInfo`, to index an
/// analyzed target with: `target[providers.DefaultInfo]`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Providers;

impl fmt::Display for Providers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Providers")
    }
}

starlark_simple_value!(Providers);

#[starlark_value(type = "Providers")]
impl<'v> StarlarkValue<'v> for Providers {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("providers_methods", providers_methods);
        Some(RES.methods())
    }
}

#[allow(non_snake_case)]
#[starlark_module]
fn providers_methods(registry: &mut MethodsBuilder) {
    /// The `DefaultInfo` provider key.
    #[starlark(attribute)]
    fn DefaultInfo<'v>(this: &Providers, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        let mut env = blazon_rt::vm::env()?;
        Ok(bazel::DefaultInfo::provider(&mut env)?.to_value(heap)?)
    }

    /// The `OutputGroupInfo` provider key.
    #[starlark(attribute)]
    fn OutputGroupInfo<'v>(this: &Providers, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        let mut env = blazon_rt::vm::env()?;
        Ok(bazel::OutputGroupInfo::provider(&mut env)?.to_value(heap)?)
    }

    /// The `RunEnvironmentInfo` provider key.
    #[starlark(attribute)]
    fn RunEnvironmentInfo<'v>(this: &Providers, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let _ = this;
        let mut env = blazon_rt::vm::env()?;
        Ok(bazel::RunEnvironmentInfo::provider(&mut env)?.to_value(heap)?)
    }
}

/// An artifact a build produced, with the metadata Skyframe recorded: `path`,
/// `digest`, `size` and `is_local` are free, they read no file.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuiltArtifact {
    #[allocative(skip)]
    info: Arc<ArtifactInfo>,
}

impl fmt::Debug for ArtifactInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ArtifactInfo({})", self.path)
    }
}

impl BuiltArtifact {
    pub fn new(info: Arc<ArtifactInfo>) -> BuiltArtifact {
        BuiltArtifact { info }
    }

    /// The paths of the files this artifact stands for: itself, or a tree's files.
    pub fn files(&self) -> Vec<String> {
        match &self.info.children {
            Some(children) => children
                .iter()
                .flat_map(|c| BuiltArtifact::new(c.clone()).files())
                .collect(),
            None => vec![self.info.path.clone()],
        }
    }
}

impl fmt::Display for BuiltArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<built artifact {}>", self.info.path)
    }
}

starlark_simple_value!(BuiltArtifact);

#[starlark_value(type = "BuiltArtifact")]
impl<'v> StarlarkValue<'v> for BuiltArtifact {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("built_artifact_methods", built_artifact_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn built_artifact_methods(registry: &mut MethodsBuilder) {
    /// The absolute path of the file, or of the directory for a tree artifact.
    #[starlark(attribute)]
    fn path(this: &BuiltArtifact) -> anyhow::Result<String> {
        Ok(this.info.path.clone())
    }

    /// The content digest as lower-case hex, or None when Bazel recorded none.
    #[starlark(attribute)]
    fn digest<'v>(this: &BuiltArtifact, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(match &this.info.digest {
            Some(d) => heap.alloc(d.as_str()),
            None => Value::new_none(),
        })
    }

    /// The size in bytes.
    #[starlark(attribute)]
    fn size(this: &BuiltArtifact) -> anyhow::Result<i64> {
        Ok(this.info.size)
    }

    /// Whether the bytes are on this machine's disk; false when only a remote store holds them.
    #[starlark(attribute)]
    fn is_local(this: &BuiltArtifact) -> anyhow::Result<bool> {
        Ok(this.info.is_local)
    }

    /// Whether this is a directory of files (a tree artifact).
    #[starlark(attribute)]
    fn is_tree(this: &BuiltArtifact) -> anyhow::Result<bool> {
        Ok(this.info.children.is_some())
    }

    /// The files of a tree artifact, empty for a file.
    fn children<'v>(this: &BuiltArtifact, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let kids = this.info.children.iter().flatten();
        Ok(heap.alloc(AllocList(
            kids.map(|c| heap.alloc(BuiltArtifact::new(c.clone()))),
        )))
    }

    /// The live `File` Bazel built, as its generated type.
    #[starlark(attribute)]
    fn artifact<'v>(this: &BuiltArtifact, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let mut env = blazon_rt::vm::env()?;
        Ok(this.info.artifact.get(&mut env)?.to_value(heap)?)
    }
}

/// A target that did not build.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Failure {
    #[allocative(skip)]
    label: blazon_rt::Global<bazel::Label<'static>>,
    message: String,
}

impl Failure {
    pub fn new(label: blazon_rt::Global<bazel::Label<'static>>, message: String) -> Failure {
        Failure { label, message }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failure({})", self.message)
    }
}

starlark_simple_value!(Failure);

#[starlark_value(type = "Failure")]
impl<'v> StarlarkValue<'v> for Failure {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("failure_methods", failure_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn failure_methods(registry: &mut MethodsBuilder) {
    /// The label of the target that failed.
    #[starlark(attribute)]
    fn label<'v>(this: &Failure, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let mut env = blazon_rt::vm::env()?;
        let obj = this.label.get(&mut env)?;
        Ok(obj.to_value(heap)?)
    }

    /// What Bazel reported.
    #[starlark(attribute)]
    fn message(this: &Failure) -> anyhow::Result<String> {
        Ok(this.message.clone())
    }
}

/// One target's outcome of `ctx.build`: an index into the build's batch, whose
/// configured target, artifacts and their metadata are read when first asked for.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuildResult {
    #[allocative(skip)]
    batch: Arc<Batch>,
    index: usize,
}

impl fmt::Debug for Batch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Batch({})", self.len())
    }
}

impl BuildResult {
    pub fn new(batch: Arc<Batch>, index: usize) -> BuildResult {
        BuildResult { batch, index }
    }

    pub fn artifacts(&self) -> anyhow::Result<Vec<BuiltArtifact>> {
        Ok(self
            .batch
            .artifacts(self.index)?
            .iter()
            .map(|a| BuiltArtifact::new(a.clone()))
            .collect())
    }
}

impl fmt::Display for BuildResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.batch.failure(self.index) {
            None => f.write_str("BuildResult"),
            Some(_) => f.write_str("BuildResult(failed)"),
        }
    }
}

starlark_simple_value!(BuildResult);

#[starlark_value(type = "BuildResult")]
impl<'v> StarlarkValue<'v> for BuildResult {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("build_result_methods", build_result_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn build_result_methods(registry: &mut MethodsBuilder) {
    /// The artifacts the target built; empty when it failed.
    fn artifacts<'v>(this: &BuildResult, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc(AllocList(
            this.artifacts()?.into_iter().map(|a| heap.alloc(a)),
        )))
    }

    /// Why the target did not build: empty on success, otherwise one `Failure`.
    fn failures<'v>(this: &BuildResult, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let failures: Vec<Value<'v>> = match this.batch.failure(this.index) {
            Some(m) => vec![heap.alloc(Failure::new(
                this.batch.labels[this.index].clone(),
                m.to_string(),
            ))],
            None => Vec::new(),
        };
        Ok(heap.alloc(AllocList(failures)))
    }

    /// The live configured target that was built, None when it did not analyze.
    #[starlark(attribute)]
    fn target<'v>(this: &BuildResult, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let Some(target) = this.batch.target(this.index)? else {
            return Ok(Value::new_none());
        };
        let mut env = blazon_rt::vm::env()?;
        Ok(target.get(&mut env)?.to_value(heap)?)
    }
}

/// `ctx.outputs`: materialization. `build` never moves bytes; this is where a
/// script asks for them.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Outputs;

impl fmt::Display for Outputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Outputs")
    }
}

starlark_simple_value!(Outputs);

#[starlark_value(type = "Outputs")]
impl<'v> StarlarkValue<'v> for Outputs {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("outputs_methods", outputs_methods);
        Some(RES.methods())
    }
}

fn collect_files<'v>(v: Value<'v>, heap: Heap<'v>, out: &mut Vec<String>) -> anyhow::Result<()> {
    if let Some(a) = v.downcast_ref::<BuiltArtifact>() {
        out.extend(a.files());
        return Ok(());
    }
    if let Some(r) = v.downcast_ref::<BuildResult>() {
        for a in r.artifacts()? {
            out.extend(a.files());
        }
        return Ok(());
    }
    if let Some(d) = DictRef::from_value(v) {
        for (_, item) in d.iter() {
            collect_files(item, heap, out)?;
        }
        return Ok(());
    }
    let items = v.iterate(heap).map_err(|_| {
        anyhow::anyhow!(
            "ensure takes built artifacts, a BuildResult, a dict of them or a list; got {}",
            v.get_type()
        )
    })?;
    for item in items {
        collect_files(item, heap, out)?;
    }
    Ok(())
}

#[starlark_module]
fn outputs_methods(registry: &mut MethodsBuilder) {
    /// Makes the artifacts available on disk and returns their paths, tree
    /// artifacts expanded to their files. With only the local executor the
    /// bytes are already there, so this verifies each file exists.
    fn ensure<'v>(
        this: &Outputs,
        artifacts: Value<'v>,
        heap: Heap<'v>,
    ) -> anyhow::Result<Value<'v>> {
        let _ = this;
        let mut paths = Vec::new();
        collect_files(artifacts, heap, &mut paths)?;
        for p in &paths {
            std::fs::symlink_metadata(p)
                .map_err(|e| anyhow::anyhow!("built output {p} is not on disk: {e}"))?;
        }
        Ok(heap.alloc(AllocList(paths.iter().map(String::as_str))))
    }
}

/// `ctx.execution`: executors.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Execution {
    #[allocative(skip)]
    heap: FrozenHeapHolder,
}

/// The frozen heap of the script's module, for routing functions.
pub struct FrozenHeapHolder(pub starlark::values::FrozenHeapRef);

impl fmt::Debug for FrozenHeapHolder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FrozenHeap")
    }
}

impl Execution {
    pub fn new(heap: starlark::values::FrozenHeapRef) -> Execution {
        Execution {
            heap: FrozenHeapHolder(heap),
        }
    }

    /// A routing function as a router: frozen, or an error.
    pub fn router_of(
        &self,
        f: Value,
        executors: Vec<(String, Arc<dyn Executor>)>,
    ) -> anyhow::Result<Arc<dyn Router>> {
        let executors = if executors.is_empty() {
            vec![(
                "local".to_string(),
                Arc::new(crate::execution::LocalExecutor {
                    name: "local".into(),
                }) as Arc<dyn Executor>,
            )]
        } else {
            executors
        };
        Ok(Arc::new(StarlarkRouter {
            function: RouteFn::new(f, &self.heap.0)?,
            executors,
        }))
    }
}

impl fmt::Display for Execution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Execution")
    }
}

starlark_simple_value!(Execution);

#[starlark_value(type = "Execution")]
impl<'v> StarlarkValue<'v> for Execution {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("execution_methods", execution_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn execution_methods(registry: &mut MethodsBuilder) {
    /// Bazel's local spawn strategy: actions run as local processes in the
    /// execution root. `name` tells executors apart for routing.
    fn local<'v>(
        this: &Execution,
        #[starlark(require = named)] name: Option<&str>,
    ) -> anyhow::Result<ExecutorValue> {
        let _ = this;
        Ok(ExecutorValue(Arc::new(crate::execution::LocalExecutor {
            name: name.unwrap_or("local").to_string(),
        })))
    }

    /// Bazel's sandboxed local runner (`darwin-sandbox`, or `processwrapper-sandbox`
    /// where there is no platform sandbox): actions see only their declared inputs.
    /// Actions tagged `no-sandbox`, `nosandbox` or `local` run unsandboxed, as in
    /// Bazel. Fails when the host cannot sandbox.
    fn sandbox<'v>(
        this: &Execution,
        #[starlark(require = named)] name: Option<&str>,
    ) -> anyhow::Result<ExecutorValue> {
        let _ = this;
        Ok(ExecutorValue(crate::sandbox::SandboxExecutor::required(
            name,
        )?))
    }

    /// A REAPI store: the content-addressable storage and action cache at
    /// `endpoint` (`grpc://host:port` or `grpcs://host:port`) and `instance`.
    /// `headers` are sent with every call and never logged.
    fn store<'v>(
        this: &Execution,
        endpoint: &str,
        #[starlark(require = named)] instance: Option<&str>,
        #[starlark(require = named)] headers: Option<UnpackDictEntries<&'v str, &'v str>>,
    ) -> anyhow::Result<StoreValue> {
        let _ = this;
        let headers = headers
            .map(|h| {
                h.entries
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(StoreValue(Store::connect(
            endpoint,
            instance.unwrap_or(""),
            headers,
        )?))
    }

    /// An executor that runs spawns on the REAPI server at `endpoint`: it
    /// uploads what the server lacks, calls `Execute`, and downloads the outputs
    /// into the execution root. `store` shares a connection made by `store`.
    fn remote<'v>(
        this: &Execution,
        endpoint: &str,
        #[starlark(require = named)] instance: Option<&str>,
        #[starlark(require = named)] store: Option<&StoreValue>,
        #[starlark(require = named)] name: Option<&str>,
    ) -> anyhow::Result<ExecutorValue> {
        let _ = this;
        let store = match store {
            Some(s) => s.0.clone(),
            None => Store::connect(endpoint, instance.unwrap_or(""), Vec::new())?,
        };
        Ok(ExecutorValue(Arc::new(crate::remote::RemoteExecutor {
            name: name.unwrap_or("remote").to_string(),
            store,
        })))
    }

    /// `executor` behind the action cache of `store`: a hit downloads the
    /// outputs and skips the executor; a miss runs it and, when it succeeded,
    /// stores the outputs and the result.
    fn cached<'v>(
        this: &Execution,
        store: &StoreValue,
        executor: &ExecutorValue,
    ) -> anyhow::Result<ExecutorValue> {
        let _ = this;
        Ok(ExecutorValue(Arc::new(crate::remote::Cached {
            store: store.0.clone(),
            inner: executor.0.clone(),
        })))
    }

    /// An executor that picks one per spawn with `function(spawn)`, which must
    /// be a top-level function of the script returning one of `executors`
    /// (a dict of names to executors, also `spawn.executors`).
    fn route<'v>(
        this: &Execution,
        function: Value<'v>,
        #[starlark(require = named)] executors: Option<
            UnpackDictEntries<&'v str, &'v ExecutorValue>,
        >,
    ) -> anyhow::Result<RouteExecutor> {
        let executors = executors
            .map(|d| {
                d.entries
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.0.clone()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(RouteExecutor {
            router: RouterRef(this.router_of(function, executors)?),
        })
    }
}

/// An executor: what `ctx.execution.local()`, `remote` and `cached` return,
/// accepted by `ctx.build` and returned by routing functions.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct ExecutorValue(#[allocative(skip)] pub Arc<dyn Executor>);

impl fmt::Debug for ExecutorValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Executor({})", self.0.name())
    }
}

impl fmt::Display for ExecutorValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Executor({:?})", self.0.name())
    }
}

starlark_simple_value!(ExecutorValue);

#[starlark_value(type = "Executor")]
impl<'v> StarlarkValue<'v> for ExecutorValue {}

/// A REAPI store, from `ctx.execution.store(...)`.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct StoreValue(#[allocative(skip)] pub Arc<Store>);

impl fmt::Debug for StoreValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl fmt::Display for StoreValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Store({:?}, instance = {:?})",
            self.0.endpoint(),
            self.0.instance()
        )
    }
}

starlark_simple_value!(StoreValue);

#[starlark_value(type = "Store")]
impl<'v> StarlarkValue<'v> for StoreValue {}
