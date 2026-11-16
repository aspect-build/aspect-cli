//! Telemetry — Starlark surface for late-installed exporters.
//!
//! Exposed as `ctx.telemetry` on `ConfigContext` and `FeatureContext`. Holds an
//! `Exporters` mutable list; features and config call
//! `ctx.telemetry.exporters.add(...)`. After phase 3, the runtime downcasts
//! the Telemetry value, walks the Exporters, and builds real exporters from
//! the collected specs in `crates/aspect-cli/src/trace.rs`.
//!
//! Two exporter shapes are supported:
//!
//! - **OTLP**: `add(url=..., protocol=..., headers=..., signals=..., resource_attributes=...)`
//!   ships traces/logs/metrics to an OTel collector over gRPC or HTTP/protobuf.
//! - **File**: `add(file=..., format=..., signals=..., resource_attributes=...)`
//!   writes locally to stderr, stdout, or a file path. Used (among other things)
//!   to express `ASPECT_DEBUG=1` as ordinary user-space configuration.
//!
//! `ctx.telemetry.build_events` and `ctx.telemetry.execution_log` set when a
//! Bazel build's stream becomes telemetry: a [`SourcePolicy`] each, held in
//! [`TelemetrySources`] on the run's `Env` so every `ctx.bazel.build` reads it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::{self, Display};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::dict::{AllocDict, UnpackDictEntries};
use starlark::values::list::{AllocList, UnpackList};
use starlark::values::none::NoneOr;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, FrozenValue, Heap, NoSerialize, ProvidesStaticType,
    StarlarkValue, Trace, Tracer, Value, ValueLike, ValueTyped, starlark_value,
};

/// Plain-data spec for one exporter. Sent across thread boundaries after
/// phase 3 (deliberately decoupled from any Starlark heap lifetime) AND
/// exposed as a Starlark value (`iterate_collect` on `Exporters` yields one
/// of these per registered exporter, with `kind`/`url`/`headers`/etc.
/// attribute access for features that want to introspect them — typically
/// for `trace.log` debugging).
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub enum ExporterSpec {
    Otlp(#[allocative(skip)] OtlpSpec),
    File(#[allocative(skip)] FileSpec),
}

#[derive(Clone, Debug)]
pub struct OtlpSpec {
    pub url: String,
    pub protocol: OtlpProtocol,
    pub headers: BTreeMap<String, String>,
    pub signals: Signals,
    pub resource_attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct FileSpec {
    pub destination: FileDestination,
    pub format: FileFormat,
    pub signals: Signals,
    pub resource_attributes: BTreeMap<String, String>,
}

impl ExporterSpec {
    fn signals(&self) -> Signals {
        match self {
            ExporterSpec::Otlp(o) => o.signals,
            ExporterSpec::File(f) => f.signals,
        }
    }

    fn resource_attributes(&self) -> &BTreeMap<String, String> {
        match self {
            ExporterSpec::Otlp(o) => &o.resource_attributes,
            ExporterSpec::File(f) => &f.resource_attributes,
        }
    }
}

unsafe impl<'v> Trace<'v> for ExporterSpec {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark::starlark_simple_value!(ExporterSpec);

#[starlark_value(type = "ExporterSpec")]
impl<'v> StarlarkValue<'v> for ExporterSpec {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        // Common attributes
        match attribute {
            "kind" => {
                return Some(heap.alloc(match self {
                    ExporterSpec::Otlp(_) => "otlp",
                    ExporterSpec::File(_) => "file",
                }));
            }
            "signals" => return Some(signals_to_starlark(self.signals(), heap)),
            "resource_attributes" => {
                return Some(string_map_to_starlark(self.resource_attributes(), heap));
            }
            _ => {}
        }
        // Variant-specific attributes
        match self {
            ExporterSpec::Otlp(o) => match attribute {
                "url" => Some(heap.alloc(o.url.as_str())),
                "protocol" => Some(heap.alloc(match o.protocol {
                    OtlpProtocol::Grpc => "grpc",
                    OtlpProtocol::HttpProtobuf => "http/protobuf",
                })),
                "headers" => Some(string_map_to_starlark(&o.headers, heap)),
                _ => None,
            },
            ExporterSpec::File(f) => match attribute {
                "file" => Some(heap.alloc(match &f.destination {
                    FileDestination::Stderr => "stderr".to_string(),
                    FileDestination::Stdout => "stdout".to_string(),
                    FileDestination::Path(p) => p.display().to_string(),
                })),
                "format" => Some(heap.alloc(match f.format {
                    FileFormat::Compact => "compact",
                    FileFormat::Jsonl => "jsonl",
                })),
                _ => None,
            },
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        let mut v = vec![
            "kind".to_owned(),
            "signals".to_owned(),
            "resource_attributes".to_owned(),
        ];
        match self {
            ExporterSpec::Otlp(_) => {
                v.extend(["url", "protocol", "headers"].map(String::from));
            }
            ExporterSpec::File(_) => {
                v.extend(["file", "format"].map(String::from));
            }
        }
        v
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OtlpProtocol {
    Grpc,
    HttpProtobuf,
}

impl OtlpProtocol {
    fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "grpc" => Ok(OtlpProtocol::Grpc),
            "http/protobuf" | "http" => Ok(OtlpProtocol::HttpProtobuf),
            other => Err(anyhow::anyhow!(
                "unknown OTLP protocol {:?} (expected \"grpc\" or \"http/protobuf\")",
                other
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDestination {
    Stderr,
    Stdout,
    Path(PathBuf),
}

impl FileDestination {
    fn parse(s: &str) -> Self {
        match s {
            "stderr" => FileDestination::Stderr,
            "stdout" => FileDestination::Stdout,
            other => FileDestination::Path(PathBuf::from(other)),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FileFormat {
    /// One compact human-readable line per record.
    Compact,
    /// One JSON object per line — machine-parseable.
    Jsonl,
}

impl FileFormat {
    fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "compact" => Ok(FileFormat::Compact),
            "jsonl" | "json" => Ok(FileFormat::Jsonl),
            other => Err(anyhow::anyhow!(
                "unknown file format {:?} (expected \"compact\" or \"jsonl\")",
                other
            )),
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Signals {
    pub traces: bool,
    pub logs: bool,
    pub metrics: bool,
}

impl Signals {
    pub fn all() -> Self {
        Signals {
            traces: true,
            logs: true,
            metrics: true,
        }
    }
}

/// When a Bazel build's stream becomes telemetry: its spans and, where it
/// has them, its metrics, together.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SourcePolicy {
    /// When the build already has the stream.
    #[default]
    Auto,
    /// Turn the stream on for every build.
    Always,
    /// Never.
    Off,
}

impl SourcePolicy {
    const NAMES: [(&'static str, SourcePolicy); 3] = [
        ("auto", SourcePolicy::Auto),
        ("always", SourcePolicy::Always),
        ("off", SourcePolicy::Off),
    ];

    pub fn parse(name: &str) -> anyhow::Result<Self> {
        Self::NAMES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, p)| *p)
            .ok_or_else(|| anyhow::anyhow!("expected one of auto, always, off; got {name:?}"))
    }

    pub fn name(self) -> &'static str {
        Self::NAMES.iter().find(|(_, p)| *p == self).unwrap().0
    }

    /// `(stream on, export its telemetry)` for a build that asked for the
    /// stream as `requested`, while `exporting` says an exporter takes that
    /// telemetry. `Always` turns the stream on only when one does.
    pub fn resolve(self, requested: bool, exporting: bool) -> (bool, bool) {
        match self {
            SourcePolicy::Auto => (requested, requested && exporting),
            SourcePolicy::Always => (requested || exporting, exporting),
            SourcePolicy::Off => (requested, false),
        }
    }
}

/// The run's [`SourcePolicy`] per Bazel stream, shared by `ctx.telemetry`
/// (which sets it) and every build spawned in the run (which reads it), and
/// which signals the run's exporters take, set once they are collected.
#[derive(Debug, Default)]
pub struct TelemetrySources {
    build_events: AtomicU8,
    execution_log: AtomicU8,
    traces: AtomicBool,
    metrics: AtomicBool,
}

impl TelemetrySources {
    /// Whether an OTLP exporter takes traces. File exporters (such as
    /// `ASPECT_DEBUG=1`'s) print AXL logs and events and take no spans.
    pub fn traces(&self) -> bool {
        self.traces.load(Ordering::Relaxed)
    }

    /// Whether an OTLP exporter takes metrics.
    pub fn metrics(&self) -> bool {
        self.metrics.load(Ordering::Relaxed)
    }

    /// Record which signals `specs` export.
    pub fn set_exported(&self, specs: &[ExporterSpec]) {
        let otlp = || {
            specs.iter().filter_map(|s| match s {
                ExporterSpec::Otlp(o) => Some(o.signals),
                ExporterSpec::File(_) => None,
            })
        };
        self.traces
            .store(otlp().any(|s| s.traces), Ordering::Relaxed);
        self.metrics
            .store(otlp().any(|s| s.metrics), Ordering::Relaxed);
    }

    /// Mark every signal exported, for tests that build without exporters.
    #[cfg(test)]
    pub fn export_everything(&self) {
        self.traces.store(true, Ordering::Relaxed);
        self.metrics.store(true, Ordering::Relaxed);
    }

    pub fn build_events(&self) -> SourcePolicy {
        Self::load(&self.build_events)
    }

    pub fn execution_log(&self) -> SourcePolicy {
        Self::load(&self.execution_log)
    }

    const ATTRIBUTES: [&'static str; 2] = ["build_events", "execution_log"];

    fn slot(&self, attribute: &str) -> Option<&AtomicU8> {
        match attribute {
            "build_events" => Some(&self.build_events),
            "execution_log" => Some(&self.execution_log),
            _ => None,
        }
    }

    fn load(slot: &AtomicU8) -> SourcePolicy {
        SourcePolicy::NAMES[slot.load(Ordering::Relaxed) as usize].1
    }

    fn store(slot: &AtomicU8, policy: SourcePolicy) {
        let index = SourcePolicy::NAMES.iter().position(|(_, p)| *p == policy);
        slot.store(index.unwrap() as u8, Ordering::Relaxed);
    }
}

fn sources_get_attr<'v>(
    sources: &TelemetrySources,
    attribute: &str,
    heap: Heap<'v>,
) -> Option<Value<'v>> {
    sources
        .slot(attribute)
        .map(|slot| heap.alloc(TelemetrySources::load(slot).name()))
}

fn sources_set_attr(
    sources: &TelemetrySources,
    attribute: &str,
    value: Value,
) -> starlark::Result<()> {
    let slot = sources.slot(attribute).ok_or_else(|| {
        starlark::Error::new_other(anyhow::anyhow!(
            "`ctx.telemetry` has no settable attribute `{attribute}`; set \
             `build_events` or `execution_log`"
        ))
    })?;
    let name = value.unpack_str().ok_or_else(|| {
        starlark::Error::new_other(anyhow::anyhow!(
            "`ctx.telemetry.{attribute}` must be a string, got `{}`",
            value.get_type()
        ))
    })?;
    let policy = SourcePolicy::parse(name).map_err(|e| {
        starlark::Error::new_other(anyhow::anyhow!("`ctx.telemetry.{attribute}`: {e}"))
    })?;
    TelemetrySources::store(slot, policy);
    Ok(())
}

/// `ctx.telemetry` value. Holds a `Value` pointing at its `Exporters`
/// instance so both ConfigContext and FeatureContext views share a single
/// list across phases, and the run's [`TelemetrySources`].
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Telemetry>")]
pub struct Telemetry<'v> {
    exporters: Value<'v>,
    #[allocative(skip)]
    sources: Arc<TelemetrySources>,
}

impl<'v> Telemetry<'v> {
    /// Allocate a fresh Telemetry (with an empty Exporters list) on the heap.
    pub fn alloc(heap: Heap<'v>, sources: Arc<TelemetrySources>) -> Value<'v> {
        let exporters = heap.alloc(Exporters::new());
        heap.alloc(Telemetry { exporters, sources })
    }

    pub fn exporters(&self) -> Value<'v> {
        self.exporters
    }
}

unsafe impl<'v> Trace<'v> for Telemetry<'v> {
    fn trace(&mut self, tracer: &Tracer<'v>) {
        self.exporters.trace(tracer);
    }
}

impl<'v> AllocValue<'v> for Telemetry<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for Telemetry<'v> {
    type Frozen = FrozenTelemetry;
    fn freeze(self, freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenTelemetry {
            exporters: self.exporters.freeze(freezer)?,
            sources: self.sources,
        })
    }
}

#[starlark_value(type = "Telemetry")]
impl<'v> StarlarkValue<'v> for Telemetry<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("telemetry_methods", telemetry_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        sources_get_attr(&self.sources, attribute, heap)
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        sources_set_attr(&self.sources, attribute, value)
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        TelemetrySources::ATTRIBUTES.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        TelemetrySources::ATTRIBUTES.map(String::from).to_vec()
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Telemetry>")]
pub struct FrozenTelemetry {
    #[allocative(skip)]
    exporters: FrozenValue,
    #[allocative(skip)]
    sources: Arc<TelemetrySources>,
}

unsafe impl<'v> Trace<'v> for FrozenTelemetry {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark::starlark_simple_value!(FrozenTelemetry);

#[starlark_value(type = "Telemetry")]
impl<'v> StarlarkValue<'v> for FrozenTelemetry {
    type Canonical = Telemetry<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("telemetry_methods", telemetry_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        sources_get_attr(&self.sources, attribute, heap)
    }

    fn set_attr(&self, attribute: &str, value: Value<'v>) -> starlark::Result<()> {
        sources_set_attr(&self.sources, attribute, value)
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        TelemetrySources::ATTRIBUTES.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        TelemetrySources::ATTRIBUTES.map(String::from).to_vec()
    }
}

#[starlark_module]
fn telemetry_methods(builder: &mut MethodsBuilder) {
    /// Mutable list of exporters. Use `ctx.telemetry.exporters.add(...)`
    /// during config or feature evaluation. The runtime collects these after
    /// phase 3 and builds real exporters; any spans/logs emitted earlier are
    /// replayed to them.
    #[starlark(attribute)]
    fn exporters<'v>(this: Value<'v>) -> anyhow::Result<Value<'v>> {
        if let Some(t) = this.downcast_ref::<Telemetry>() {
            return Ok(t.exporters);
        }
        if let Some(t) = this.downcast_ref::<FrozenTelemetry>() {
            return Ok(t.exporters.to_value());
        }
        Err(anyhow::anyhow!("expected Telemetry"))
    }
}

/// `ctx.telemetry.exporters` value. Mutable list with `add(**kwargs)`. After
/// freezing it remains readable (the runtime drains via `take_specs` before
/// freezing).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Exporters>")]
pub struct Exporters {
    #[allocative(skip)]
    specs: RefCell<Vec<ExporterSpec>>,
}

impl Exporters {
    pub fn new() -> Self {
        Self {
            specs: RefCell::new(Vec::new()),
        }
    }

    /// Drain all collected specs out of the Exporters list. Intended to be
    /// called once by the runtime after phase 3.
    pub fn take_specs(&self) -> Vec<ExporterSpec> {
        std::mem::take(&mut *self.specs.borrow_mut())
    }

    pub fn len(&self) -> usize {
        self.specs.borrow().len()
    }
}

unsafe impl<'v> Trace<'v> for Exporters {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl<'v> AllocValue<'v> for Exporters {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl Freeze for Exporters {
    type Frozen = FrozenExporters;
    fn freeze(self, _freezer: &Freezer) -> Result<Self::Frozen, FreezeError> {
        Ok(FrozenExporters {
            specs: self.specs.into_inner(),
        })
    }
}

#[starlark_value(type = "Exporters")]
impl<'v> StarlarkValue<'v> for Exporters {
    fn length(&self) -> starlark::Result<i32> {
        Ok(self.specs.borrow().len() as i32)
    }

    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self
            .specs
            .borrow()
            .iter()
            .map(|s| heap.alloc(s.clone()))
            .collect())
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("exporters_methods", exporters_methods);
        Some(RES.methods())
    }
}

/// Frozen variant of `Exporters` — read-only after freeze. The runtime
/// drains specs before freezing in practice, so this typically holds an
/// empty `Vec`. Kept around so the Starlark value remains valid in any
/// frozen-module path that touches it.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Exporters>")]
pub struct FrozenExporters {
    #[allocative(skip)]
    specs: Vec<ExporterSpec>,
}

unsafe impl<'v> Trace<'v> for FrozenExporters {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark::starlark_simple_value!(FrozenExporters);

#[starlark_value(type = "Exporters")]
impl<'v> StarlarkValue<'v> for FrozenExporters {
    type Canonical = Exporters;

    fn length(&self) -> starlark::Result<i32> {
        Ok(self.specs.len() as i32)
    }

    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self.specs.iter().map(|s| heap.alloc(s.clone())).collect())
    }
}

fn parse_signals(list: UnpackList<String>) -> anyhow::Result<Signals> {
    let mut s = Signals::default();
    for name in list.items {
        match name.as_str() {
            "traces" => s.traces = true,
            "logs" => s.logs = true,
            "metrics" => s.metrics = true,
            other => {
                return Err(anyhow::anyhow!(
                    "unknown signal {:?} (expected \"traces\", \"logs\", or \"metrics\")",
                    other
                ));
            }
        }
    }
    Ok(s)
}

fn signals_to_starlark<'v>(s: Signals, heap: Heap<'v>) -> Value<'v> {
    let mut names: Vec<Value<'v>> = Vec::with_capacity(3);
    if s.traces {
        names.push(heap.alloc("traces").to_value());
    }
    if s.logs {
        names.push(heap.alloc("logs").to_value());
    }
    if s.metrics {
        names.push(heap.alloc("metrics").to_value());
    }
    heap.alloc(AllocList(names))
}

fn string_map_to_starlark<'v>(m: &BTreeMap<String, String>, heap: Heap<'v>) -> Value<'v> {
    let entries: Vec<(Value<'v>, Value<'v>)> = m
        .iter()
        .map(|(k, v)| {
            (
                heap.alloc(k.as_str()).to_value(),
                heap.alloc(v.as_str()).to_value(),
            )
        })
        .collect();
    heap.alloc(AllocDict(entries))
}

fn collect_str_dict(entries: UnpackDictEntries<String, String>) -> BTreeMap<String, String> {
    entries.entries.into_iter().collect()
}

#[starlark_module]
fn exporters_methods(builder: &mut MethodsBuilder) {
    /// Register an exporter. The runtime builds the corresponding sink after
    /// phase 3 and replays any buffered spans/logs into it.
    ///
    /// Exactly one of `url` (OTLP exporter) or `file` (local file/stderr/stdout
    /// exporter) must be provided.
    ///
    /// Args (OTLP shape):
    ///   url: OTLP collector URL
    ///   protocol: "grpc" (default) or "http/protobuf"
    ///   headers: optional dict[str,str] for auth/tenant routing
    ///
    /// Args (file shape):
    ///   file: filesystem path, or "stderr" / "stdout"
    ///   format: "compact" (default, one human-readable line per record) or
    ///           "jsonl" (one JSON object per line, machine-parseable)
    ///
    /// Common args:
    ///   signals: optional list, subset of ["traces", "logs", "metrics"];
    ///            default is all three
    ///   resource_attributes: optional dict[str,str] merged into the
    ///                        Resource an OTLP exporter sends. Metric
    ///                        exporters share one Resource, merged from all
    ///                        of theirs; where two set a key differently,
    ///                        the first registered wins and the CLI warns.
    ///                        File exporters write no Resource.
    fn add<'v>(
        this: ValueTyped<'v, Exporters>,
        #[starlark(require = named, default = NoneOr::None)] url: NoneOr<&'v str>,
        #[starlark(require = named, default = NoneOr::None)] file: NoneOr<&'v str>,
        #[starlark(require = named, default = NoneOr::None)] protocol: NoneOr<&'v str>,
        #[starlark(require = named, default = NoneOr::None)] format: NoneOr<&'v str>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        headers: UnpackDictEntries<String, String>,
        #[starlark(require = named, default = UnpackList::default())] signals: UnpackList<String>,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        resource_attributes: UnpackDictEntries<String, String>,
    ) -> anyhow::Result<starlark::values::none::NoneType> {
        let url = url.into_option();
        let file = file.into_option();
        let protocol = protocol.into_option();
        let format = format.into_option();

        if url.is_some() == file.is_some() {
            return Err(anyhow::anyhow!(
                "exporters.add: exactly one of `url` (OTLP) or `file` (local sink) must be set"
            ));
        }

        let sig = if signals.items.is_empty() {
            Signals::all()
        } else {
            parse_signals(signals)?
        };
        let resource_attributes = collect_str_dict(resource_attributes);

        let spec = if let Some(url) = url {
            if url.is_empty() {
                return Err(anyhow::anyhow!("exporters.add: url must not be empty"));
            }
            if format.is_some() {
                return Err(anyhow::anyhow!(
                    "exporters.add: `format` is only valid with `file=` (use `protocol=` for OTLP)"
                ));
            }
            let proto = OtlpProtocol::parse(protocol.unwrap_or("grpc"))?;
            ExporterSpec::Otlp(OtlpSpec {
                url: url.to_string(),
                protocol: proto,
                headers: collect_str_dict(headers),
                signals: sig,
                resource_attributes,
            })
        } else {
            let file = file.unwrap();
            if file.is_empty() {
                return Err(anyhow::anyhow!("exporters.add: file must not be empty"));
            }
            if protocol.is_some() {
                return Err(anyhow::anyhow!(
                    "exporters.add: `protocol` is only valid with `url=` (use `format=` for files)"
                ));
            }
            if !headers.entries.is_empty() {
                return Err(anyhow::anyhow!(
                    "exporters.add: `headers` is only valid with OTLP `url=`"
                ));
            }
            let fmt = FileFormat::parse(format.unwrap_or("compact"))?;
            ExporterSpec::File(FileSpec {
                destination: FileDestination::parse(file),
                format: fmt,
                signals: sig,
                resource_attributes,
            })
        };

        this.as_ref().specs.borrow_mut().push(spec);

        // Mark telemetry as active so trace.enabled in subsequent phases reports
        // honestly. Idempotent — safe to call repeatedly.
        crate::trace::enable();

        Ok(starlark::values::none::NoneType)
    }
}

/// Walk a Telemetry Value and drain its exporter specs out of the underlying
/// Exporters list. Returns an empty Vec if `value` is not a Telemetry or its
/// exporters field can't be unpacked (unexpected, but defensively handled).
pub fn drain_exporters<'v>(value: Value<'v>) -> Vec<ExporterSpec> {
    let exporters_value = if let Some(t) = value.downcast_ref::<Telemetry>() {
        t.exporters
    } else if let Some(t) = value.downcast_ref::<FrozenTelemetry>() {
        t.exporters.to_value()
    } else {
        return Vec::new();
    };
    if let Some(ex) = exporters_value.downcast_ref::<Exporters>() {
        ex.take_specs()
    } else {
        Vec::new()
    }
}

impl Display for ExporterSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExporterSpec::Otlp(s) => write!(f, "otlp:{}@{:?}", s.url, s.protocol),
            ExporterSpec::File(s) => match &s.destination {
                FileDestination::Stderr => write!(f, "file:stderr@{:?}", s.format),
                FileDestination::Stdout => write!(f, "file:stdout@{:?}", s.format),
                FileDestination::Path(p) => write!(f, "file:{}@{:?}", p.display(), s.format),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_policy_decides_the_stream_and_its_spans() {
        use SourcePolicy::*;
        // (policy, requested, exporting) -> (stream on, spans)
        let table = [
            (Auto, false, false, (false, false)),
            (Auto, false, true, (false, false)),
            (Auto, true, false, (true, false)),
            (Auto, true, true, (true, true)),
            (Always, false, false, (false, false)),
            (Always, false, true, (true, true)),
            (Always, true, false, (true, false)),
            (Always, true, true, (true, true)),
            (Off, false, true, (false, false)),
            (Off, true, true, (true, false)),
        ];
        for (policy, requested, exporting, want) in table {
            assert_eq!(
                policy.resolve(requested, exporting),
                want,
                "{policy:?} requested={requested} exporting={exporting}"
            );
        }
    }

    #[test]
    fn only_otlp_exporters_count_and_each_for_its_own_signals() {
        let otlp = |traces, metrics| {
            ExporterSpec::Otlp(OtlpSpec {
                url: "http://collector".into(),
                protocol: OtlpProtocol::Grpc,
                headers: BTreeMap::new(),
                signals: Signals {
                    traces,
                    logs: true,
                    metrics,
                },
                resource_attributes: BTreeMap::new(),
            })
        };
        let file = ExporterSpec::File(FileSpec {
            destination: FileDestination::Stderr,
            format: FileFormat::parse("compact").unwrap(),
            signals: Signals::all(),
            resource_attributes: BTreeMap::new(),
        });
        let exported = |specs: &[ExporterSpec]| {
            let sources = TelemetrySources::default();
            sources.set_exported(specs);
            (sources.traces(), sources.metrics())
        };
        assert_eq!(exported(&[]), (false, false));
        assert_eq!(exported(&[file.clone()]), (false, false), "file only");
        assert_eq!(exported(&[otlp(false, false)]), (false, false), "logs only");
        assert_eq!(
            exported(&[otlp(false, true)]),
            (false, true),
            "metrics only"
        );
        assert_eq!(exported(&[otlp(true, false), file]), (true, false));
        assert_eq!(
            exported(&[otlp(true, false), otlp(false, true)]),
            (true, true)
        );
    }

    #[test]
    fn policies_round_trip_by_name() {
        for (name, policy) in SourcePolicy::NAMES {
            assert_eq!(SourcePolicy::parse(name).unwrap(), policy);
            assert_eq!(policy.name(), name);
        }
        assert!(SourcePolicy::parse("on").is_err());
        let sources = TelemetrySources::default();
        assert_eq!(sources.build_events(), SourcePolicy::Auto);
        assert_eq!(sources.execution_log(), SourcePolicy::Auto);
    }

    /// Config sets a policy and a feature in the next phase reads it back:
    /// both see the run's one `TelemetrySources`, which builds read too.
    #[test]
    fn config_sets_a_policy_that_features_see() {
        let marker = tempfile::NamedTempFile::new().unwrap();
        let exit = crate::test::eval(&format!(
            r#"
def _feature(ctx):
    ctx.std.fs.write({path:?}, ctx.telemetry.execution_log + " " + ctx.telemetry.build_events)

Probe = feature(implementation = _feature)

def _impl(ctx):
    return 0

t = task(implementation = _impl)
"#,
            path = marker.path().to_str().unwrap(),
        ))
        .with_config(
            r#"
def config(ctx):
    ctx.telemetry.execution_log = "always"
    ctx.telemetry.build_events = "off"
"#,
        )
        .with_features(&["Probe"])
        .run_task(0)
        .expect("run_task");
        assert_eq!(exit, Some(0));
        assert_eq!(
            std::fs::read_to_string(marker.path()).unwrap(),
            "always off"
        );
    }

    #[test]
    fn an_unknown_policy_is_refused_with_the_choices() {
        let err = crate::test::eval(
            r#"
def _impl(ctx):
    return 0

t = task(implementation = _impl)
"#,
        )
        .with_config(
            r#"
def config(ctx):
    ctx.telemetry.execution_log = "on"
"#,
        )
        .run_task(0)
        .expect_err("an unknown policy must be refused");
        let err = format!("{err:#}");
        assert!(err.contains("auto, always, off"), "{err}");
    }

    #[test]
    fn dir_lists_the_settable_policies() {
        let names = crate::test::eval(
            r#"
def _feature(ctx):
    names = [n for n in dir(ctx.telemetry) if n in ("build_events", "execution_log")]
    if names != ["build_events", "execution_log"] or not hasattr(ctx.telemetry, "execution_log"):
        fail("dir/hasattr miss the policies: %r" % names)

Probe = feature(implementation = _feature)

def _impl(ctx):
    return 0

t = task(implementation = _impl)
"#,
        )
        .with_features(&["Probe"])
        .run_task(0);
        assert_eq!(names.expect("run_task"), Some(0));
    }
}
