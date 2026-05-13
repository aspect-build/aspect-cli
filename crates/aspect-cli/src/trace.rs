//! Process tracing setup.
//!
//! Initialization is deferred — at startup we install the tracing-subscriber
//! registry with an `OpenTelemetryLayer` / `OpenTelemetryTracingBridge` whose
//! exporters are `BufferingSpanExporter` / `BufferingLogExporter` (which
//! accumulate everything in memory) plus a runtime-togglable `StderrLayer`
//! that's wired but inactive by default.
//!
//! After phase 3 of the axl runtime, `install_late_exporters(specs)` is
//! called with the set of `ExporterSpec`s collected from
//! `ctx.telemetry.exporters.add(...)`. Each OTLP spec becomes a real OTLP
//! exporter; the buffer is replayed. Each file/stderr spec activates the
//! `StderrLayer` (or a future per-path file sink). Users who want
//! `ASPECT_DEBUG=1`-style stderr output register a stderr exporter from
//! .axl config — there's no env-var special-casing in this binary.

use axl_runtime::engine::telemetry::{
    ExporterSpec, FileDestination, FileSpec, OtlpProtocol, OtlpSpec,
};
use opentelemetry::{KeyValue, global, trace::TracerProvider as _};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig, WithTonicConfig};
use opentelemetry_sdk::{
    Resource,
    logs::{LogExporter as _, SdkLoggerProvider},
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
};
use opentelemetry_semantic_conventions::{SCHEMA_URL, attribute::SERVICE_VERSION};
use std::sync::{Mutex, OnceLock};
use tonic::metadata::{MetadataKey, MetadataMap, MetadataValue};
use tracing_core::Level;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{
    Layer,
    filter::{self, Targets},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::trace_buffer::{BufferingLogExporter, BufferingSpanExporter};

use axl_runtime::engine::telemetry::{FileFormat, Signals};

/// One installed file/stderr/stdout sink. Each `ExporterSpec::File`
/// registered via `ctx.telemetry.exporters.add(file=..., format=..., signals=[...])`
/// becomes one entry here. The layer walks this list per event and
/// writes to whichever sinks accept the event's target.
struct Sink {
    writer: SinkWriter,
    format: FileFormat,
    signals: Signals,
}

enum SinkWriter {
    Stderr,
    Stdout,
    File(std::sync::Mutex<std::fs::File>),
}

impl SinkWriter {
    fn write_line(&self, bytes: &[u8]) {
        use std::io::Write;
        // Best-effort: writes can fail (closed pipe, full disk, etc.).
        // We swallow errors rather than panic — telemetry must never
        // take down the process. A dropped record is the right failure
        // mode for a debug/observability sink.
        match self {
            SinkWriter::Stderr => {
                let mut out = std::io::stderr().lock();
                let _ = out.write_all(bytes);
                let _ = out.write_all(b"\n");
            }
            SinkWriter::Stdout => {
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(bytes);
                let _ = out.write_all(b"\n");
            }
            SinkWriter::File(m) => {
                if let Ok(mut f) = m.lock() {
                    let _ = f.write_all(bytes);
                    let _ = f.write_all(b"\n");
                }
            }
        }
    }
}

/// Append-only list of installed sinks. Populated by
/// `install_late_exporters(...)`. Held in a `Mutex` for interior
/// mutability; reads on the hot path are short and uncontended in
/// practice (one process-wide lock per emitted event, only when at
/// least one sink is installed).
static SINKS: std::sync::Mutex<Vec<Sink>> = std::sync::Mutex::new(Vec::new());

/// Cap for the pre-install event buffer. Matches the OTel buffer
/// (`trace_buffer.rs`) so file-sink replay has the same memory floor as
/// span/log replay. Drop-oldest semantics on overflow.
const FILE_SINK_BUFFER_CAP: usize = 50_000;

/// Pre-install event buffer for the file-sinks path. Mirrors
/// `BufferingSpanExporter` / `BufferingLogExporter` for the OTel side:
/// during phases 1-3 (and any time before `install_late_exporters` runs)
/// the file-sinks layer has nowhere to write, so we buffer here and
/// replay on first sink installation. After replay, `BUFFER_DRAINED` is
/// set and the buffer stays empty.
static EVENT_BUFFER: std::sync::Mutex<Vec<BufferedEvent>> = std::sync::Mutex::new(Vec::new());

static BUFFER_DRAINED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct BufferedEvent {
    kind: RecordKind,
    fields: VisitedFields,
}

/// Format `(kind, fields)` per `sink.format` and write to `sink`. Shared
/// between live emission (`FileSinksLayer::on_event`) and buffer replay
/// (`drain_buffer_to_sinks`) so the wire format stays consistent.
fn write_event_to_sink(sink: &Sink, kind: RecordKind, fields: &VisitedFields) {
    if !sink_accepts(sink.signals, kind) {
        return;
    }
    let line = match sink.format {
        FileFormat::Compact => format_compact(kind, fields),
        FileFormat::Jsonl => format_jsonl(kind, fields),
    };
    sink.writer.write_line(line.as_bytes());
}

/// Drain the pre-install event buffer into the freshly-installed sinks.
/// Called once from `install_late_exporters` after sinks are populated.
/// Idempotent — sets `BUFFER_DRAINED` so subsequent file-spec
/// activations (if any) skip the replay.
fn drain_buffer_to_sinks() {
    use std::sync::atomic::Ordering;
    if BUFFER_DRAINED.swap(true, Ordering::AcqRel) {
        return;
    }
    let buffered: Vec<BufferedEvent> = match EVENT_BUFFER.lock() {
        Ok(mut g) => std::mem::take(&mut *g),
        Err(_) => return,
    };
    if buffered.is_empty() {
        return;
    }
    let sinks = match SINKS.lock() {
        Ok(g) if !g.is_empty() => g,
        _ => return,
    };
    for ev in &buffered {
        for sink in sinks.iter() {
            write_event_to_sink(sink, ev.kind, &ev.fields);
        }
    }
}

/// Discard the pre-install event buffer and mark it drained. Called
/// when no file sink ever shows up, so the buffered events have no
/// destination — free the memory and stop accumulating.
fn discard_buffer() {
    use std::sync::atomic::Ordering;
    if BUFFER_DRAINED.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Ok(mut g) = EVENT_BUFFER.lock() {
        g.clear();
        g.shrink_to_fit();
    }
}

/// Sink layer for `trace.log` / `trace.event`. Listens on the `axl.log`
/// and `axl.event` targets, formats per-sink, and writes to each sink's
/// destination. Filtering to those targets is done at the layer level so
/// the rest of the process's tracing doesn't pollute these sinks.
///
/// Activated by `ctx.telemetry.exporters.add(file=..., format=..., signals=...)`
/// — including the stock `ASPECT_DEBUG=1` translation in
/// `aspect/feature/telemetry.axl`.
///
/// Pre-install events (anything fired before `install_late_exporters`
/// runs) are buffered into `EVENT_BUFFER` and replayed on first sink
/// installation, mirroring the OTel buffer-replay mechanism so file
/// sinks see the same history their OTLP counterparts do.
struct FileSinksLayer;

impl<S: tracing_core::Subscriber> Layer<S> for FileSinksLayer {
    fn on_event(
        &self,
        event: &tracing_core::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        use std::sync::atomic::Ordering;
        let target = event.metadata().target();
        let kind = match target {
            "axl.log" => RecordKind::Log,
            "axl.event" => RecordKind::Event,
            _ => return,
        };

        // Visit fields once. We always need them — either to write to a
        // sink immediately or to buffer for later replay.
        let mut visited = VisitedFields::default();
        event.record(&mut visited);

        // If sinks are installed, write directly. Otherwise (and only
        // before `install_late_exporters` has run), buffer for replay.
        let sinks_guard = SINKS.lock();
        let sinks_installed = matches!(&sinks_guard, Ok(g) if !g.is_empty());
        if sinks_installed {
            let sinks = sinks_guard.unwrap();
            for sink in sinks.iter() {
                write_event_to_sink(sink, kind, &visited);
            }
            return;
        }
        drop(sinks_guard);

        if BUFFER_DRAINED.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut buf) = EVENT_BUFFER.lock() {
            // Drop-oldest on overflow — keeps memory bounded if no file
            // sink ever shows up but the buffer accumulates regardless.
            if buf.len() >= FILE_SINK_BUFFER_CAP {
                buf.remove(0);
            }
            buf.push(BufferedEvent {
                kind,
                fields: visited,
            });
        }
    }
}

#[derive(Copy, Clone)]
enum RecordKind {
    Log,
    Event,
}

fn sink_accepts(signals: Signals, kind: RecordKind) -> bool {
    match kind {
        RecordKind::Log => signals.logs,
        // `axl.event` is span-event-shaped; routed under the `traces`
        // signal in OTLP. File sinks mirror the same gate.
        RecordKind::Event => signals.traces,
    }
}

#[derive(Default)]
struct VisitedFields {
    /// For `axl.log`: the formatted log message body. For `axl.event`:
    /// the event name (which we routed onto tracing's `message` field
    /// so `tracing-opentelemetry` uses it as the OTel span event name).
    message: String,
    description: String,
    category: String,
    error: String,
    fields: String,
}

impl tracing_core::field::Visit for VisitedFields {
    fn record_str(&mut self, f: &tracing_core::field::Field, v: &str) {
        match f.name() {
            "message" => self.message = v.to_owned(),
            "description" => self.description = v.to_owned(),
            "category" => self.category = v.to_owned(),
            "error" => self.error = v.to_owned(),
            "fields" => self.fields = v.to_owned(),
            _ => {}
        }
    }
    fn record_debug(&mut self, f: &tracing_core::field::Field, v: &dyn std::fmt::Debug) {
        // `%expr` Display values arrive as a Debug-impl that delegates to
        // Display, so `format!("{:?}", v)` returns the Display output.
        let s = format!("{:?}", v);
        match f.name() {
            "message" => self.message = s,
            "description" => self.description = s,
            "category" => self.category = s,
            "error" => self.error = s,
            "fields" => self.fields = s,
            _ => {}
        }
    }
}

/// Compact one-line format:
/// - `axl.log`: `<message>`
/// - `axl.event`: `<name> [<category>] "<description>" error="<error>" fields=<fields>`
///   (empty slots elided)
fn format_compact(kind: RecordKind, v: &VisitedFields) -> String {
    match kind {
        RecordKind::Log => v.message.clone(),
        RecordKind::Event => {
            let mut buf = String::with_capacity(v.message.len() + 8);
            buf.push_str(&v.message);
            if !v.category.is_empty() {
                buf.push_str(" [");
                buf.push_str(&v.category);
                buf.push(']');
            }
            if !v.description.is_empty() {
                buf.push(' ');
                buf.push('"');
                buf.push_str(&v.description);
                buf.push('"');
            }
            if !v.error.is_empty() {
                buf.push_str(" error=\"");
                buf.push_str(&v.error);
                buf.push('"');
            }
            if !v.fields.is_empty() {
                buf.push_str(" fields=");
                buf.push_str(&v.fields);
            }
            buf
        }
    }
}

/// JSONL: one JSON object per line. Keys consistent across records so
/// downstream consumers can expect a stable schema.
fn format_jsonl(kind: RecordKind, v: &VisitedFields) -> String {
    use serde_json::{Map, Value, json};
    let mut obj = Map::new();
    obj.insert(
        "ts".to_string(),
        Value::String(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)),
    );
    match kind {
        RecordKind::Log => {
            obj.insert("kind".to_string(), Value::String("log".to_string()));
            obj.insert("message".to_string(), Value::String(v.message.clone()));
        }
        RecordKind::Event => {
            obj.insert("kind".to_string(), Value::String("event".to_string()));
            obj.insert("name".to_string(), Value::String(v.message.clone()));
            if !v.description.is_empty() {
                obj.insert(
                    "description".to_string(),
                    Value::String(v.description.clone()),
                );
            }
            if !v.category.is_empty() {
                obj.insert("category".to_string(), Value::String(v.category.clone()));
            }
            if !v.error.is_empty() {
                obj.insert("error".to_string(), Value::String(v.error.clone()));
            }
            if !v.fields.is_empty() {
                // `fields` is a Starlark `repr(...)` string; surface as a
                // single string attribute. Backends that want per-key
                // queryability can re-parse on their side.
                obj.insert("fields".to_string(), Value::String(v.fields.clone()));
            }
        }
    }
    let _ = json!(null); // pull `json!` macro into use; placeholder for future per-field handling
    Value::Object(obj).to_string()
}

fn base_resource_builder() -> opentelemetry_sdk::resource::ResourceBuilder {
    Resource::builder()
        .with_service_name("aspect-cli")
        .with_schema_url(
            [KeyValue::new(
                SERVICE_VERSION,
                aspect_telemetry::cargo_pkg_version(),
            )],
            SCHEMA_URL,
        )
}

fn base_resource() -> Resource {
    base_resource_builder().build()
}

fn merged_resource(extra: &std::collections::BTreeMap<String, String>) -> Resource {
    if extra.is_empty() {
        return base_resource();
    }
    let mut b = base_resource_builder();
    for (k, v) in extra {
        b = b.with_attribute(KeyValue::new(k.clone(), v.clone()));
    }
    b.build()
}

/// Process-wide handles to the buffering exporters so phase-3 → late-init
/// can reach them. Set in `init()`; read in `install_late_exporters()`.
static SPAN_BUFFER: OnceLock<BufferingSpanExporter> = OnceLock::new();
static LOG_BUFFER: OnceLock<BufferingLogExporter> = OnceLock::new();
/// Set by `install_late_exporters` when at least one endpoint requested
/// `metrics`. Read by `OtelGuard::drop` to flush at shutdown.
static LATE_METER_PROVIDER: Mutex<Option<SdkMeterProvider>> = Mutex::new(None);

/// True once `install_late_exporters` has activated at least one exporter
/// (OTLP, file, or stderr). Drives whether `OtelGuard::drop` emits flush
/// progress to stderr — for fast no-telemetry paths (`--help`, `version`,
/// or any run with all exporters disabled), shutdown is silent.
static ANY_LATE_EXPORTER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Initialize the tracing subscriber with stderr (when `ASPECT_DEBUG` is set)
/// and OTel layers backed by the buffering exporters. Returns the guard that
/// flushes/shuts down providers on drop.
pub fn init() -> OtelGuard {
    let span_buffer = BufferingSpanExporter::new();
    let log_buffer = BufferingLogExporter::new();

    let tracer_provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            1.0,
        ))))
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(base_resource())
        .with_batch_exporter(span_buffer.clone())
        .build();

    let logger_provider = SdkLoggerProvider::builder()
        .with_resource(base_resource())
        .with_batch_exporter(log_buffer.clone())
        .build();

    SPAN_BUFFER
        .set(span_buffer)
        .ok()
        .expect("trace::init called twice");
    LOG_BUFFER
        .set(log_buffer)
        .ok()
        .expect("trace::init called twice");

    let tracer = tracer_provider.tracer("tracing-otel-subscriber");

    // The trace builtin (axl-runtime) emits via the tracing macros on two
    // targets:
    // - `axl.log`  → free-form log lines, routed to OTel logs (LogRecord)
    //   via the appender bridge. Filtered out of the span-event sink so
    //   they don't double-emit.
    // - `axl.event` → structured span events, routed to the
    //   OpenTelemetryLayer (span-event sink) which calls
    //   `span.add_event(name, attributes)` on the active span. Filtered
    //   out of the appender bridge so they don't also become LogRecords.
    let otel_layer =
        OpenTelemetryLayer::new(tracer).with_filter(filter::filter_fn(|m| m.target() != "axl.log"));
    let log_bridge = OpenTelemetryTracingBridge::new(&logger_provider)
        .with_filter(filter::filter_fn(|m| m.target() != "axl.event"));

    // `axl.event` always emits at INFO (span events have no severity in
    // OTel, so the trace builtin doesn't take a `level=` arg). `axl.log`
    // emits at TRACE so its target floor must be lowered explicitly.
    let level_targets = Targets::new()
        .with_default(Level::INFO)
        .with_target("axl.log", Level::TRACE);

    // The file-sinks layer is always wired into the subscriber but
    // stays a no-op until `install_late_exporters` populates `SINKS`
    // in response to user `exporters.add(file=..., ...)` calls.
    let file_sinks_layer = FileSinksLayer.with_filter(filter::filter_fn(|m| {
        let t = m.target();
        t == "axl.log" || t == "axl.event"
    }));

    tracing_subscriber::registry()
        .with(level_targets)
        .with(otel_layer)
        .with(log_bridge)
        .with(file_sinks_layer)
        .init();

    OtelGuard {
        tracer_provider,
        logger_provider,
    }
}

/// Build a real `opentelemetry_otlp::SpanExporter` for one OTLP spec.
fn build_otlp_span_exporter(spec: &OtlpSpec) -> anyhow::Result<opentelemetry_otlp::SpanExporter> {
    let exporter = match spec.protocol {
        OtlpProtocol::Grpc => {
            let mut b = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&spec.url)
                .with_tls_config(
                    opentelemetry_otlp::tonic_types::transport::ClientTlsConfig::new()
                        .with_native_roots(),
                );
            if !spec.headers.is_empty() {
                b = b.with_metadata(metadata_from(&spec.headers)?);
            }
            b.build()?
        }
        OtlpProtocol::HttpProtobuf => {
            let mut b = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(&spec.url);
            if !spec.headers.is_empty() {
                b = b.with_headers(spec.headers.clone().into_iter().collect());
            }
            b.build()?
        }
    };
    Ok(exporter)
}

fn build_otlp_log_exporter(spec: &OtlpSpec) -> anyhow::Result<opentelemetry_otlp::LogExporter> {
    let exporter = match spec.protocol {
        OtlpProtocol::Grpc => {
            let mut b = opentelemetry_otlp::LogExporter::builder()
                .with_tonic()
                .with_endpoint(&spec.url)
                .with_tls_config(
                    opentelemetry_otlp::tonic_types::transport::ClientTlsConfig::new()
                        .with_native_roots(),
                );
            if !spec.headers.is_empty() {
                b = b.with_metadata(metadata_from(&spec.headers)?);
            }
            b.build()?
        }
        OtlpProtocol::HttpProtobuf => {
            let mut b = opentelemetry_otlp::LogExporter::builder()
                .with_http()
                .with_endpoint(&spec.url);
            if !spec.headers.is_empty() {
                b = b.with_headers(spec.headers.clone().into_iter().collect());
            }
            b.build()?
        }
    };
    Ok(exporter)
}

fn build_otlp_metric_exporter(
    spec: &OtlpSpec,
) -> anyhow::Result<opentelemetry_otlp::MetricExporter> {
    let exporter = match spec.protocol {
        OtlpProtocol::Grpc => {
            let mut b = opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_endpoint(&spec.url)
                .with_tls_config(
                    opentelemetry_otlp::tonic_types::transport::ClientTlsConfig::new()
                        .with_native_roots(),
                );
            if !spec.headers.is_empty() {
                b = b.with_metadata(metadata_from(&spec.headers)?);
            }
            b.with_temporality(opentelemetry_sdk::metrics::Temporality::default())
                .build()?
        }
        OtlpProtocol::HttpProtobuf => {
            let mut b = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_endpoint(&spec.url);
            if !spec.headers.is_empty() {
                b = b.with_headers(spec.headers.clone().into_iter().collect());
            }
            b.with_temporality(opentelemetry_sdk::metrics::Temporality::default())
                .build()?
        }
    };
    Ok(exporter)
}

fn metadata_from(
    headers: &std::collections::BTreeMap<String, String>,
) -> anyhow::Result<MetadataMap> {
    let mut m = MetadataMap::new();
    for (k, v) in headers {
        let key: MetadataKey<_> = k
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid OTLP header name {:?}: {}", k, e))?;
        let value: MetadataValue<_> = v
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid OTLP header value for {:?}: {}", k, e))?;
        m.insert(key, value);
    }
    Ok(m)
}

/// Activate a file exporter spec — register a sink with the
/// `FileSinksLayer`. Stderr / stdout / file path are all live; format
/// (`compact` or `jsonl`) and signals filter (`logs` / `traces`) are
/// honored per-sink.
fn activate_file_spec(spec: &FileSpec) -> anyhow::Result<()> {
    let writer = match &spec.destination {
        FileDestination::Stderr => SinkWriter::Stderr,
        FileDestination::Stdout => SinkWriter::Stdout,
        FileDestination::Path(p) => {
            // Append, create-if-missing — never truncate; multiple
            // invocations against the same path should accumulate.
            let f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| {
                    anyhow::anyhow!("telemetry exporter file={:?}: open failed: {}", p, e)
                })?;
            SinkWriter::File(std::sync::Mutex::new(f))
        }
    };
    let sink = Sink {
        writer,
        format: spec.format,
        signals: spec.signals,
    };
    if let Ok(mut sinks) = SINKS.lock() {
        sinks.push(sink);
    }
    Ok(())
}

/// Take the exporter specs collected during phase 1-3 evaluation and turn
/// them into real exporters. OTLP specs become `opentelemetry_otlp` exporters
/// installed into the SDK pipeline (with the in-memory buffer replayed). File
/// specs activate the runtime-togglable `StderrLayer` (or, eventually, file/
/// stdout sinks). Sets up a fresh metrics provider that writes to all
/// metric-enabled OTLP exporters (no pre-phase-3 metric replay; phases 1-3
/// don't emit meaningful metric data).
///
/// Stores the new `SdkMeterProvider` in a process-wide static so the
/// existing `OtelGuard` can flush it on drop without taking `&mut`.
pub async fn install_late_exporters(specs: Vec<ExporterSpec>) -> anyhow::Result<()> {
    if specs.is_empty() {
        // No exporter ever shows up. Mark OTel inactive so trace builtins
        // early-return for the rest of the run.
        axl_runtime::trace::disable();
        return Ok(());
    }

    let span_buf = SPAN_BUFFER
        .get()
        .ok_or_else(|| anyhow::anyhow!("trace::init not called"))?;
    let log_buf = LOG_BUFFER
        .get()
        .ok_or_else(|| anyhow::anyhow!("trace::init not called"))?;

    let mut span_exporters = Vec::new();
    let mut log_exporters = Vec::new();
    let mut metric_exporters = Vec::new();
    let mut any_file = false;

    for spec in &specs {
        match spec {
            ExporterSpec::Otlp(otlp) => {
                let resource = merged_resource(&otlp.resource_attributes);
                if otlp.signals.traces {
                    let mut ex = build_otlp_span_exporter(otlp)?;
                    opentelemetry_sdk::trace::SpanExporter::set_resource(&mut ex, &resource);
                    span_exporters.push(ex);
                }
                if otlp.signals.logs {
                    let mut ex = build_otlp_log_exporter(otlp)?;
                    ex.set_resource(&resource);
                    log_exporters.push(ex);
                }
                if otlp.signals.metrics {
                    metric_exporters.push((otlp.clone(), resource));
                }
            }
            ExporterSpec::File(file) => {
                activate_file_spec(file)?;
                any_file = true;
            }
        }
    }

    // Replay buffered file-sink events into the freshly-installed sinks
    // (mirrors the OTLP buffer-replay below). When no file sink shows
    // up, drop the buffer so it stops accumulating for the rest of the
    // run.
    if any_file {
        drain_buffer_to_sinks();
    } else {
        discard_buffer();
    }

    // Note: we intentionally do *not* disable OTel here even when no
    // OTLP spec was supplied. File-only configurations (e.g. the stock
    // `ASPECT_DEBUG=1` translation registers a stderr file spec) still
    // need trace macros to fire so `FileSinksLayer` can pick them up.
    // `axl_runtime::trace::enable()` is called unconditionally from
    // `Exporters::add`, so `axl_runtime::trace::enabled()` already
    // reflects "any exporter is registered".

    if !span_exporters.is_empty() {
        span_buf.install_late(span_exporters).await;
    }
    if !log_exporters.is_empty() {
        log_buf.install_late(log_exporters).await;
    }

    if !metric_exporters.is_empty() {
        let mut builder = MeterProviderBuilder::default()
            // Resource attribution at the metric provider level uses the
            // base resource. Per-exporter resource_attributes are not
            // separately surfaced in metrics in v1 (the OTLP metric
            // exporter respects the provider's resource). Acceptable for
            // an initial implementation; revisit if a use case appears.
            .with_resource(base_resource());
        for (spec, _resource) in metric_exporters {
            let exporter = build_otlp_metric_exporter(&spec)?;
            let reader = PeriodicReader::builder(exporter)
                .with_interval(std::time::Duration::from_secs(30))
                .build();
            builder = builder.with_reader(reader);
        }
        let provider = builder.build();
        global::set_meter_provider(provider.clone());
        if let Ok(mut slot) = LATE_METER_PROVIDER.lock() {
            *slot = Some(provider);
        }
    }

    if !specs.is_empty() {
        ANY_LATE_EXPORTER.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    Ok(())
}

/// Per-provider shutdown timeout. Override with the
/// `ASPECT_TELEMETRY_SHUTDOWN_TIMEOUT_MS` env var (e.g. `=15000` for 15s).
fn shutdown_timeout() -> std::time::Duration {
    let ms = std::env::var("ASPECT_TELEMETRY_SHUTDOWN_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5000);
    std::time::Duration::from_millis(ms)
}

pub(crate) struct OtelGuard {
    tracer_provider: SdkTracerProvider,
    logger_provider: SdkLoggerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // Fast paths (no telemetry installed: `--help`, `version`, or runs
        // with the Telemetry feature disabled) shut down silently.
        let any_otel = ANY_LATE_EXPORTER.load(std::sync::atomic::Ordering::Relaxed);
        let debug = std::env::var_os("ASPECT_DEBUG")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let timeout = shutdown_timeout();
        if any_otel && debug {
            eprintln!(
                "telemetry: flushing exporters (per-provider timeout {}ms; \
                 set ASPECT_TELEMETRY_SHUTDOWN_TIMEOUT_MS to override)",
                timeout.as_millis()
            );
        }
        let start = std::time::Instant::now();

        let mut errors: Vec<String> = Vec::new();
        if let Err(err) = self.tracer_provider.shutdown_with_timeout(timeout) {
            errors.push(format!("tracer: {err:?}"));
        }
        if let Ok(mut slot) = LATE_METER_PROVIDER.lock() {
            if let Some(mp) = slot.take() {
                if let Err(err) = mp.shutdown_with_timeout(timeout) {
                    errors.push(format!("meter: {err:?}"));
                }
            }
        }
        if let Err(err) = self.logger_provider.shutdown_with_timeout(timeout) {
            errors.push(format!("logger: {err:?}"));
        }

        if any_otel {
            let elapsed_ms = start.elapsed().as_millis();
            if errors.is_empty() {
                if debug {
                    eprintln!("telemetry: flush complete in {}ms", elapsed_ms);
                }
            } else {
                // Always surface errors — visibility gap, not a build failure.
                eprintln!(
                    "telemetry: flush completed with errors after {}ms — \
                     some records may have been dropped (this does not affect \
                     the task exit code)",
                    elapsed_ms
                );
                for err in errors {
                    eprintln!("  {}", err);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_runtime::engine::telemetry::{FileFormat, Signals};

    /// Process-wide lock serializing tests that touch shared globals
    /// (`SINKS`, `EVENT_BUFFER`, `BUFFER_DRAINED`). Cargo runs unit tests
    /// in parallel by default; concurrent mutation of these statics
    /// produces flaky failures that don't reproduce in single-threaded
    /// runs.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_test() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sample_event_fields() -> VisitedFields {
        VisitedFields {
            message: "build.cancel".to_string(),
            description: "user requested cancel".to_string(),
            category: "bazel.runner".to_string(),
            error: "ctrl-c".to_string(),
            fields: r#"{"cancelling": True}"#.to_string(),
        }
    }

    fn sample_log_fields() -> VisitedFields {
        VisitedFields {
            message: "loaded module foo".to_string(),
            ..VisitedFields::default()
        }
    }

    #[test]
    fn compact_log_renders_message_only() {
        let v = sample_log_fields();
        let s = format_compact(RecordKind::Log, &v);
        assert_eq!(s, "loaded module foo");
    }

    #[test]
    fn compact_event_renders_full_line() {
        let v = sample_event_fields();
        let s = format_compact(RecordKind::Event, &v);
        // `name [category] "description" error="…" fields=…`
        assert_eq!(
            s,
            r#"build.cancel [bazel.runner] "user requested cancel" error="ctrl-c" fields={"cancelling": True}"#
        );
    }

    #[test]
    fn compact_event_elides_empty_slots() {
        let v = VisitedFields {
            message: "boot".to_string(),
            ..VisitedFields::default()
        };
        let s = format_compact(RecordKind::Event, &v);
        assert_eq!(s, "boot");
    }

    #[test]
    fn jsonl_log_has_expected_keys() {
        let v = sample_log_fields();
        let s = format_jsonl(RecordKind::Log, &v);
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj["kind"], "log");
        assert_eq!(obj["message"], "loaded module foo");
        // RFC3339 timestamp present and well-formed.
        let ts = obj["ts"].as_str().unwrap();
        assert!(ts.contains('T'), "ts not RFC3339: {}", ts);
    }

    #[test]
    fn jsonl_event_has_expected_keys() {
        let v = sample_event_fields();
        let s = format_jsonl(RecordKind::Event, &v);
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj["kind"], "event");
        assert_eq!(obj["name"], "build.cancel");
        assert_eq!(obj["description"], "user requested cancel");
        assert_eq!(obj["category"], "bazel.runner");
        assert_eq!(obj["error"], "ctrl-c");
        assert_eq!(obj["fields"], r#"{"cancelling": True}"#);
    }

    #[test]
    fn jsonl_event_omits_empty_optional_slots() {
        let v = VisitedFields {
            message: "minimal".to_string(),
            ..VisitedFields::default()
        };
        let s = format_jsonl(RecordKind::Event, &v);
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj["name"], "minimal");
        assert!(!obj.contains_key("description"));
        assert!(!obj.contains_key("category"));
        assert!(!obj.contains_key("error"));
        assert!(!obj.contains_key("fields"));
    }

    #[test]
    fn sink_accepts_routes_signals_to_kinds() {
        let logs_only = Signals {
            traces: false,
            logs: true,
            metrics: false,
        };
        let traces_only = Signals {
            traces: true,
            logs: false,
            metrics: false,
        };
        assert!(sink_accepts(logs_only, RecordKind::Log));
        assert!(!sink_accepts(logs_only, RecordKind::Event));
        assert!(!sink_accepts(traces_only, RecordKind::Log));
        assert!(sink_accepts(traces_only, RecordKind::Event));
    }

    #[test]
    fn activate_file_spec_with_path_creates_file() {
        let _g = lock_test();
        // Path destination opens the file in append+create mode. We call
        // `activate_file_spec` directly to avoid touching the global
        // SINKS list (which would leak across tests). Verify the file
        // exists and the SinkWriter is the File variant.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.jsonl");
        let spec = axl_runtime::engine::telemetry::FileSpec {
            destination: axl_runtime::engine::telemetry::FileDestination::Path(path.clone()),
            format: FileFormat::Jsonl,
            signals: Signals::all(),
            resource_attributes: Default::default(),
        };
        // `activate_file_spec` pushes into SINKS — we drain after to
        // avoid leaking into other tests.
        activate_file_spec(&spec).unwrap();
        assert!(path.exists(), "expected file at {:?}", path);
        // Verify the freshly-pushed sink writes through.
        if let Ok(mut sinks) = SINKS.lock() {
            let sink = sinks.last().expect("sink was pushed");
            sink.writer.write_line(b"hello-from-test");
            // Drop our entry so subsequent tests start clean.
            sinks.pop();
        }
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("hello-from-test"), "got: {:?}", contents);
    }

    #[test]
    fn buffered_event_replays_into_late_installed_sink() {
        let _g = lock_test();
        use std::sync::atomic::Ordering;

        // Reset shared state before driving the scenario.
        BUFFER_DRAINED.store(false, Ordering::Relaxed);
        if let Ok(mut g) = SINKS.lock() {
            g.clear();
        }
        if let Ok(mut g) = EVENT_BUFFER.lock() {
            g.clear();
        }

        // Simulate a phase-1-3 event landing before any sink is
        // registered: push directly into the buffer.
        if let Ok(mut buf) = EVENT_BUFFER.lock() {
            buf.push(BufferedEvent {
                kind: RecordKind::Event,
                fields: VisitedFields {
                    message: "phase1.event".to_string(),
                    description: "buffered".to_string(),
                    ..VisitedFields::default()
                },
            });
        }

        // Now register a file sink (mirrors `install_late_exporters`'s
        // `activate_file_spec` step) and drain. The buffered event
        // should appear in the file.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("buffered.txt");
        let spec = axl_runtime::engine::telemetry::FileSpec {
            destination: axl_runtime::engine::telemetry::FileDestination::Path(path.clone()),
            format: FileFormat::Compact,
            signals: Signals::all(),
            resource_attributes: Default::default(),
        };
        activate_file_spec(&spec).unwrap();
        drain_buffer_to_sinks();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            contents.contains("phase1.event"),
            "expected buffered event in file, got: {:?}",
            contents
        );
        assert!(
            contents.contains("\"buffered\""),
            "expected description rendered, got: {:?}",
            contents
        );

        // Subsequent drain calls must be idempotent.
        drain_buffer_to_sinks();

        // Cleanup so other tests start clean.
        if let Ok(mut g) = SINKS.lock() {
            g.clear();
        }
        BUFFER_DRAINED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn discard_buffer_drops_pending_events_and_blocks_further_buffering() {
        let _g = lock_test();
        use std::sync::atomic::Ordering;

        // Reset state.
        BUFFER_DRAINED.store(false, Ordering::Relaxed);
        if let Ok(mut g) = EVENT_BUFFER.lock() {
            g.clear();
        }

        // Push something to the buffer.
        if let Ok(mut buf) = EVENT_BUFFER.lock() {
            buf.push(BufferedEvent {
                kind: RecordKind::Log,
                fields: VisitedFields {
                    message: "to-be-dropped".to_string(),
                    ..VisitedFields::default()
                },
            });
        }

        discard_buffer();

        // Buffer is empty, drained flag is set.
        let buf_len = EVENT_BUFFER.lock().map(|g| g.len()).unwrap_or(usize::MAX);
        assert_eq!(buf_len, 0, "discard_buffer should clear the buffer");
        assert!(
            BUFFER_DRAINED.load(Ordering::Relaxed),
            "discard_buffer should set BUFFER_DRAINED"
        );

        // Cleanup.
        BUFFER_DRAINED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn activate_file_spec_with_missing_dir_fails() {
        let spec = axl_runtime::engine::telemetry::FileSpec {
            destination: axl_runtime::engine::telemetry::FileDestination::Path(
                "/nonexistent/dir/that/should/not/exist/trace.jsonl".into(),
            ),
            format: FileFormat::Compact,
            signals: Signals::all(),
            resource_attributes: Default::default(),
        };
        let res = activate_file_spec(&spec);
        assert!(
            res.is_err(),
            "expected error for missing dir, got: {:?}",
            res
        );
    }
}
