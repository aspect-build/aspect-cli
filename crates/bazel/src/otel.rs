//! Spans for work Bazel already finished, timed from what Bazel reported.
//!
//! A `tracing` span is timed by when it is entered and dropped, so a span for
//! an action, test or spawn that ended before its record arrived is built as a
//! [`CompletedSpan`] and exported with an OpenTelemetry tracer instead,
//! parented to the current `tracing` span through
//! `OpenTelemetrySpanExt::context`. That parent link needs the tracer and the
//! `tracing-opentelemetry` layer to share one provider, which `aspect-cli`'s
//! `trace::init` arranges by installing it globally.
//!
//! Also holds the proto time conversions the sinks share and, under test, the
//! in-memory metric helpers their tests read points with.

use std::time::{Duration, SystemTime};

use axl_proto::Timestamp;
use opentelemetry::{
    KeyValue,
    trace::{Span as _, Status, Tracer},
};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

#[derive(Debug)]
pub(crate) struct CompletedSpan {
    pub name: &'static str,
    pub start: SystemTime,
    pub end: SystemTime,
    pub status: Status,
    pub attributes: Vec<KeyValue>,
}

impl CompletedSpan {
    /// Exports the span as a child of the current `tracing` span. An end
    /// before the start is clamped to the start.
    pub fn export<T: Tracer>(self, tracer: &T) {
        let parent = tracing::Span::current().context();
        let mut span = tracer
            .span_builder(self.name)
            .with_start_time(self.start)
            .with_attributes(self.attributes)
            .start_with_context(tracer, &parent);
        span.set_status(self.status);
        span.end_with_timestamp(self.end.max(self.start));
    }
}

/// A protobuf timestamp as a `SystemTime`; negative fields clamp to zero.
pub(crate) fn system_time(ts: &Timestamp) -> SystemTime {
    SystemTime::UNIX_EPOCH
        + duration(&axl_proto::Duration {
            seconds: ts.seconds,
            nanos: ts.nanos,
        })
}

/// A protobuf duration as a `Duration`; negative fields clamp to zero.
pub(crate) fn duration(d: &axl_proto::Duration) -> Duration {
    Duration::new(d.seconds.max(0) as u64, d.nanos.max(0) as u32)
}

/// Metric assertions shared by the sinks' tests.
#[cfg(test)]
pub(crate) mod test_support {
    use opentelemetry::{
        KeyValue,
        metrics::{Meter, MeterProvider as _},
    };
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
        data::{AggregatedMetrics, MetricData},
    };

    /// Records through `record` on a fresh provider and returns every data
    /// point, as [`points`].
    pub(crate) fn recorded(record: impl FnOnce(&Meter)) -> Vec<Point> {
        let (provider, exporter) = provider();
        record(&provider.meter("test"));
        provider.force_flush().unwrap();
        points(&exporter)
    }

    /// A meter provider whose metrics land in the returned exporter on flush.
    pub(crate) fn provider() -> (SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        (provider, exporter)
    }

    /// `(instrument, sorted attributes as "key=value", value)`; a histogram's
    /// value is its sum.
    pub(crate) type Point = (String, Vec<String>, f64);

    /// Every u64 sum and f64 histogram data point `exporter` holds, sorted.
    pub(crate) fn points(exporter: &InMemoryMetricExporter) -> Vec<Point> {
        let mut points = Vec::new();
        for resource in exporter.get_finished_metrics().unwrap() {
            for scope in resource.scope_metrics() {
                for metric in scope.metrics() {
                    let name = metric.name().to_owned();
                    let attrs = |kvs: &mut dyn Iterator<Item = &KeyValue>| {
                        let mut a: Vec<String> =
                            kvs.map(|kv| format!("{}={}", kv.key, kv.value)).collect();
                        a.sort();
                        a
                    };
                    match metric.data() {
                        AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                            for p in sum.data_points() {
                                points.push((
                                    name.clone(),
                                    attrs(&mut p.attributes()),
                                    p.value() as f64,
                                ));
                            }
                        }
                        AggregatedMetrics::F64(MetricData::Histogram(h)) => {
                            for p in h.data_points() {
                                points.push((name.clone(), attrs(&mut p.attributes()), p.sum()));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        points.sort_by(|a, b| a.partial_cmp(b).unwrap());
        points
    }

    pub(crate) fn point(name: &str, attrs: &[&str], value: f64) -> Point {
        (
            name.to_owned(),
            attrs.iter().map(|s| (*s).to_owned()).collect(),
            value,
        )
    }
}
