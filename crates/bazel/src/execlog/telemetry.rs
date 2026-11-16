//! Execution log → OpenTelemetry: a `spawn` span for every spawn the log
//! records, under a `spawns` span beside the BES sink's `events`, and the
//! [`SpawnInstruments`] metrics.
//!
//! A spawn's span starts at `SpawnMetrics.start_time` and lasts its
//! `total_time`, both measured in this build, so a cache hit spans its lookup
//! and download. A spawn without a start time is a zero-length span at the
//! moment the sink read it.
//!
//! | Attribute | Value |
//! |---|---|
//! | `label`, `mnemonic`, `runner` | As Bazel recorded them |
//! | `cache_hit`, `remotable`, `cacheable`, `remote_cacheable` | Booleans |
//! | `exit_code`, and `status` when not successful | The spawn's result |
//! | `queue_ms`, `setup_ms`, `fetch_ms`, `execution_ms`, … | Each non-zero phase of [`PHASES`] |
//! | `input_files`, `input_bytes`, `memory_estimate_bytes` | When Bazel knew them |
//!
//! Phases are attributes rather than child spans because a build runs tens of
//! thousands of spawns, and a span per phase would multiply that.

use std::thread::{self, JoinHandle};
use std::time::SystemTime;

use axl_proto::tools::protos::{
    ExecLogEntry, SpawnMetrics,
    exec_log_entry::{Spawn, Type},
};
use fibre::spmc::Receiver;
use opentelemetry::{
    KeyValue, global,
    trace::{Status, Tracer},
};

use super::spawn_metrics::SpawnInstruments;
use crate::fanout::SinkOutcome;
use crate::otel::{CompletedSpan, duration, system_time};

/// Instrumentation scope of the `spawn` spans and the spawn metrics.
const TELEMETRY_SCOPE: &str = "aspect-cli.execlog";

/// Reads one phase's duration from a spawn's metrics.
type Phase = fn(&SpawnMetrics) -> Option<&axl_proto::Duration>;

/// `SpawnMetrics` durations recorded as `<name>_ms` attributes.
const PHASES: &[(&str, Phase)] = &[
    ("parse", |m| m.parse_time.as_ref()),
    ("network", |m| m.network_time.as_ref()),
    ("fetch", |m| m.fetch_time.as_ref()),
    ("queue", |m| m.queue_time.as_ref()),
    ("setup", |m| m.setup_time.as_ref()),
    ("upload", |m| m.upload_time.as_ref()),
    ("execution", |m| m.execution_wall_time.as_ref()),
    ("process_outputs", |m| m.process_outputs_time.as_ref()),
    ("retry", |m| m.retry_time.as_ref()),
];

/// Reads `recv` to the end of the stream on its own thread, exporting a span
/// per spawn under a `spawns` span opened in the caller's current span, and
/// recording the spawn metrics.
pub fn spawn(recv: Receiver<ExecLogEntry>) -> JoinHandle<SinkOutcome> {
    let spawns_span = tracing::info_span!("spawns");
    thread::spawn(move || {
        let _spawns = spawns_span.enter();
        let instruments = SpawnInstruments::new(&global::meter(TELEMETRY_SCOPE));
        consume(&recv, &global::tracer(TELEMETRY_SCOPE), &instruments);
        Ok(())
    })
}

fn consume<T: Tracer>(recv: &Receiver<ExecLogEntry>, tracer: &T, instruments: &SpawnInstruments) {
    while let Ok(entry) = recv.recv() {
        if let Some(Type::Spawn(spawn)) = entry.r#type {
            instruments.record(&spawn, failure(&spawn).is_none());
            spawn_span(&spawn, SystemTime::now()).export(tracer);
        }
    }
}

/// The `spawn` span; one without a start time is zero-length at `read`.
fn spawn_span(spawn: &Spawn, read: SystemTime) -> CompletedSpan {
    let metrics = spawn.metrics.as_ref();
    let (start, end) = match metrics.and_then(|m| m.start_time.as_ref()) {
        Some(start) => {
            let start = system_time(start);
            let total = metrics.and_then(|m| m.total_time.as_ref());
            (start, start + total.map(duration).unwrap_or_default())
        }
        None => (read, read),
    };

    let mut attributes = vec![
        KeyValue::new("label", spawn.target_label.clone()),
        KeyValue::new("mnemonic", spawn.mnemonic.clone()),
        KeyValue::new("runner", spawn.runner.clone()),
        KeyValue::new("cache_hit", spawn.cache_hit),
        KeyValue::new("remotable", spawn.remotable),
        KeyValue::new("cacheable", spawn.cacheable),
        KeyValue::new("remote_cacheable", spawn.remote_cacheable),
        KeyValue::new("exit_code", i64::from(spawn.exit_code)),
    ];
    if !succeeded(&spawn.status) {
        attributes.push(KeyValue::new("status", spawn.status.clone()));
    }
    if let Some(m) = metrics {
        attributes.extend(PHASES.iter().filter_map(|(name, phase)| {
            let ms = phase(m).map(duration)?.as_secs_f64() * 1000.0;
            (ms > 0.0).then(|| KeyValue::new(format!("{name}_ms"), ms))
        }));
        attributes.extend(
            [
                ("input_files", m.input_files),
                ("input_bytes", m.input_bytes),
                ("memory_estimate_bytes", m.memory_estimate_bytes),
            ]
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .map(|(key, n)| KeyValue::new(key, n)),
        );
    }

    CompletedSpan {
        name: "spawn",
        start,
        end,
        status: failure(spawn).map_or(Status::Unset, Status::error),
        attributes,
    }
}

/// Why the spawn failed, or `None` when it succeeded: its status, or else its
/// exit code.
fn failure(spawn: &Spawn) -> Option<String> {
    if !succeeded(&spawn.status) {
        Some(spawn.status.clone())
    } else if spawn.exit_code != 0 {
        Some(format!("exit code {}", spawn.exit_code))
    } else {
        None
    }
}

/// Bazel leaves `status` empty, or `SUCCESS`, for a spawn that succeeded.
fn succeeded(status: &str) -> bool {
    matches!(status, "" | "SUCCESS")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use axl_proto::Timestamp;
    use fibre::spmc::bounded;
    use opentelemetry::{Value, trace::TracerProvider as _};
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
    use tracing_subscriber::layer::SubscriberExt as _;

    fn at(seconds: u64, nanos: u32) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::new(seconds, nanos)
    }

    fn ms(millis: i32) -> Option<axl_proto::Duration> {
        Some(axl_proto::Duration {
            seconds: 0,
            nanos: millis * 1_000_000,
        })
    }

    fn attribute<'a>(span: &'a CompletedSpan, key: &str) -> Option<&'a Value> {
        span.attributes
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| &kv.value)
    }

    fn compiled() -> Spawn {
        Spawn {
            target_label: "//pkg:lib".into(),
            mnemonic: "CppCompile".into(),
            runner: "remote".into(),
            remotable: true,
            cacheable: true,
            metrics: Some(SpawnMetrics {
                start_time: Some(Timestamp {
                    seconds: 100,
                    nanos: 5,
                }),
                total_time: ms(250),
                queue_time: ms(40),
                execution_wall_time: ms(200),
                input_files: 12,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_spawn_is_timed_by_its_metrics() {
        let span = spawn_span(&compiled(), at(999, 0));
        assert_eq!(span.name, "spawn");
        assert_eq!(span.start, at(100, 5));
        assert_eq!(span.end, at(100, 250_000_005));
        assert_eq!(span.status, Status::Unset);
        assert_eq!(
            attribute(&span, "label"),
            Some(&Value::from("//pkg:lib".to_string()))
        );
        assert_eq!(
            attribute(&span, "runner"),
            Some(&Value::from("remote".to_string()))
        );
        assert_eq!(attribute(&span, "queue_ms"), Some(&Value::F64(40.0)));
        assert_eq!(attribute(&span, "execution_ms"), Some(&Value::F64(200.0)));
        assert_eq!(
            attribute(&span, "setup_ms"),
            None,
            "zero phases are left out"
        );
        assert_eq!(attribute(&span, "input_files"), Some(&Value::I64(12)));
        assert_eq!(attribute(&span, "input_bytes"), None);
        assert_eq!(attribute(&span, "status"), None);
    }

    #[test]
    fn a_spawn_without_a_start_time_is_zero_length_when_read() {
        let span = spawn_span(&Spawn::default(), at(7, 0));
        assert_eq!((span.start, span.end), (at(7, 0), at(7, 0)));
    }

    #[test]
    fn a_spawn_with_a_start_but_no_total_time_is_zero_length_at_its_start() {
        let spawn = Spawn {
            metrics: Some(SpawnMetrics {
                start_time: Some(Timestamp {
                    seconds: 100,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let span = spawn_span(&spawn, at(999, 0));
        assert_eq!((span.start, span.end), (at(100, 0), at(100, 0)));
    }

    #[test]
    fn a_failed_spawn_is_an_error_with_its_status_or_exit_code() {
        let exited = spawn_span(
            &Spawn {
                exit_code: 2,
                ..compiled()
            },
            at(0, 0),
        );
        assert_eq!(exited.status, Status::error("exit code 2"));
        assert_eq!(attribute(&exited, "status"), None);

        let timed_out = spawn_span(
            &Spawn {
                exit_code: 142,
                status: "TIMEOUT".into(),
                ..compiled()
            },
            at(0, 0),
        );
        assert_eq!(timed_out.status, Status::error("TIMEOUT"));
        assert_eq!(
            attribute(&timed_out, "status"),
            Some(&Value::from("TIMEOUT".to_string()))
        );

        let succeeded = spawn_span(
            &Spawn {
                status: "SUCCESS".into(),
                ..compiled()
            },
            at(0, 0),
        );
        assert_eq!(succeeded.status, Status::Unset);
    }

    #[test]
    fn consume_exports_and_counts_spawn_entries_only() {
        use crate::otel::test_support::{self, points};
        use opentelemetry::metrics::MeterProvider as _;

        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let (meter_provider, metric_exporter) = test_support::provider();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("layer")));
        let tracer = provider.tracer("test");

        let (sender, recv) = bounded(8);
        tracing::subscriber::with_default(subscriber, || {
            let _spawns = tracing::info_span!("spawns").entered();
            for r#type in [
                Type::File(Default::default()),
                Type::Spawn(compiled()),
                Type::Spawn(Spawn {
                    exit_code: 1,
                    ..compiled()
                }),
            ] {
                sender
                    .send(ExecLogEntry {
                        id: 0,
                        r#type: Some(r#type),
                    })
                    .unwrap();
            }
            drop(sender);
            consume(
                &recv,
                &tracer,
                &SpawnInstruments::new(&meter_provider.meter("test")),
            );
        });

        let spans: Vec<SpanData> = exporter.get_finished_spans().unwrap();
        let names: Vec<&str> = spans.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            names,
            ["spawn", "spawn", "spawns"],
            "a span per spawn entry only"
        );
        assert_eq!(spans[0].parent_span_id, spans[2].span_context.span_id());

        meter_provider.force_flush().unwrap();
        let success: Vec<String> = points(&metric_exporter)
            .into_iter()
            .filter(|(name, ..)| name == "bazel.spawns")
            .flat_map(|(_, attrs, _)| attrs.into_iter().filter(|a| a.starts_with("success=")))
            .collect();
        assert_eq!(
            success,
            ["success=false", "success=true"],
            "one count per spawn entry, and `success` follows the span's status"
        );
    }
}
