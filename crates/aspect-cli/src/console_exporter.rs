//! Console exporters: spans and metrics printed to stderr, one compact line
//! each, for `ctx.telemetry.exporters.add(console = True, ...)` (what
//! `OTEL_TRACES_EXPORTER=console` / `OTEL_METRICS_EXPORTER=console` and
//! `ASPECT_DEBUG=1` register).
//!
//! A span prints as it ends, `span <name> <ms>ms <key>=<value>...`; span
//! events and log records go through the file-sink layer in `trace.rs`
//! instead, which prints them as they happen. A metric prints a line per data
//! point at each export, `metric <name> <value> <key>=<value>...`; a
//! histogram's value is `count=<n> sum=<s>`.
//!
//! The exporters are process-local debugging aids, so they print to stderr
//! (stdout is for machine consumers) and swallow write errors.

use std::fmt::Write as _;
use std::time::Duration;

use axl_runtime::errln;
use opentelemetry::KeyValue;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};

/// Prints each span it exports to stderr; see the module docs.
#[derive(Debug, Default)]
pub struct ConsoleSpanExporter;

impl SpanExporter for ConsoleSpanExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        for span in &batch {
            errln!("{}", span_line(span));
        }
        Ok(())
    }
}

/// `span <name> <ms>ms <attributes>`, without the source location, thread
/// and timing attributes `tracing-opentelemetry` adds to every span.
fn span_line(span: &SpanData) -> String {
    let ms = span
        .end_time
        .duration_since(span.start_time)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0;
    let mut line = format!("span {} {ms:.1}ms", span.name);
    push_attributes(
        &mut line,
        span.attributes.iter().filter(|kv| {
            let key = kv.key.as_str();
            !(key.starts_with("code.")
                || key.starts_with("thread.")
                || matches!(key, "target" | "busy_ns" | "idle_ns"))
        }),
    );
    line
}

/// Prints each metric data point it exports to stderr; see the module docs.
#[derive(Debug, Default)]
pub struct ConsoleMetricExporter;

impl PushMetricExporter for ConsoleMetricExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        for line in metric_lines(metrics) {
            errln!("{line}");
        }
        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }

    fn temporality(&self) -> Temporality {
        Temporality::Cumulative
    }
}

/// One `metric <name> <value> <attributes>` line per data point.
fn metric_lines(metrics: &ResourceMetrics) -> Vec<String> {
    let mut lines = Vec::new();
    for scope in metrics.scope_metrics() {
        for metric in scope.metrics() {
            let name = metric.name();
            match metric.data() {
                AggregatedMetrics::F64(data) => data_lines(&mut lines, name, data),
                AggregatedMetrics::U64(data) => data_lines(&mut lines, name, data),
                AggregatedMetrics::I64(data) => data_lines(&mut lines, name, data),
            }
        }
    }
    lines
}

fn data_lines<T: Copy + std::fmt::Display>(
    lines: &mut Vec<String>,
    name: &str,
    data: &MetricData<T>,
) {
    let mut push = |value: String, attributes: &mut dyn Iterator<Item = &KeyValue>| {
        let mut line = format!("metric {name} {value}");
        push_attributes(&mut line, attributes);
        lines.push(line);
    };
    match data {
        MetricData::Gauge(g) => {
            for p in g.data_points() {
                push(p.value().to_string(), &mut p.attributes());
            }
        }
        MetricData::Sum(s) => {
            for p in s.data_points() {
                push(p.value().to_string(), &mut p.attributes());
            }
        }
        MetricData::Histogram(h) => {
            for p in h.data_points() {
                push(
                    format!("count={} sum={}", p.count(), p.sum()),
                    &mut p.attributes(),
                );
            }
        }
        MetricData::ExponentialHistogram(h) => {
            for p in h.data_points() {
                push(
                    format!("count={} sum={}", p.count(), p.sum()),
                    &mut p.attributes(),
                );
            }
        }
    }
}

fn push_attributes<'a>(line: &mut String, attributes: impl Iterator<Item = &'a KeyValue>) {
    for kv in attributes {
        let _ = write!(line, " {}={}", kv.key, kv.value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{SpanContext, SpanId, SpanKind, Status};
    use opentelemetry::{InstrumentationScope, Value};
    use std::time::SystemTime;

    #[test]
    fn a_span_prints_its_name_duration_and_attributes() {
        let start = SystemTime::UNIX_EPOCH;
        let span = SpanData {
            span_context: SpanContext::empty_context(),
            parent_span_id: SpanId::INVALID,
            parent_span_is_remote: false,
            span_kind: SpanKind::Internal,
            name: "task".into(),
            start_time: start,
            end_time: start + Duration::from_micros(12_340),
            attributes: vec![
                KeyValue::new("task.name", Value::from("build")),
                KeyValue::new("code.line.number", 85),
                KeyValue::new("thread.name", "main"),
                KeyValue::new("busy_ns", 12),
            ],
            dropped_attributes_count: 0,
            events: Default::default(),
            links: Default::default(),
            status: Status::Unset,
            instrumentation_scope: InstrumentationScope::builder("test").build(),
        };
        assert_eq!(span_line(&span), "span task 12.3ms task.name=build");
    }

    #[test]
    fn a_counter_prints_a_line_per_data_point() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::{
            InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
        };

        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        let counter = provider.meter("test").u64_counter("builds").build();
        counter.add(2, &[KeyValue::new("verb", "build")]);
        provider.force_flush().unwrap();
        let exported = exporter.get_finished_metrics().unwrap();
        let lines: Vec<String> = exported.iter().flat_map(metric_lines).collect();
        assert_eq!(lines, ["metric builds 2 verb=build"]);
    }
}
