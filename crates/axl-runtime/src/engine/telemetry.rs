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

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::{self, Display};
use std::path::PathBuf;

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

/// `ctx.telemetry` value. Holds a `Value` pointing at its `Exporters`
/// instance so both ConfigContext and FeatureContext views share a single
/// list across phases.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Telemetry>")]
pub struct Telemetry<'v> {
    exporters: Value<'v>,
}

impl<'v> Telemetry<'v> {
    /// Allocate a fresh Telemetry (with an empty Exporters list) on the heap.
    pub fn alloc(heap: Heap<'v>) -> Value<'v> {
        let exporters = heap.alloc(Exporters::new());
        heap.alloc(Telemetry { exporters })
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
        })
    }
}

#[starlark_value(type = "Telemetry")]
impl<'v> StarlarkValue<'v> for Telemetry<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(telemetry_methods)
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Telemetry>")]
pub struct FrozenTelemetry {
    #[allocative(skip)]
    exporters: FrozenValue,
}

unsafe impl<'v> Trace<'v> for FrozenTelemetry {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

starlark::starlark_simple_value!(FrozenTelemetry);

#[starlark_value(type = "Telemetry")]
impl<'v> StarlarkValue<'v> for FrozenTelemetry {
    type Canonical = Telemetry<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(telemetry_methods)
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
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(exporters_methods)
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
    ///   resource_attributes: optional dict[str,str] merged into the exporter's
    ///                        Resource
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
