//! Execution log → OpenTelemetry metrics, recorded by the telemetry sink for
//! every spawn.
//!
//! | Instrument | Type | Unit | Attributes |
//! |---|---|---|---|
//! | `bazel.spawns` | counter | `{spawn}` | `mnemonic`, `runner`, `cache_hit`, `success` |
//! | `bazel.spawn.duration` | histogram | `s` | `mnemonic`, `runner` |
//!
//! `runner` says where a spawn ran or came from (`linux-sandbox`, `remote`,
//! `disk cache hit`, …). No attribute carries a target label, so the number of
//! series stays bounded however large the build.

use axl_proto::tools::protos::exec_log_entry::Spawn;
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, Meter},
};

use crate::otel::duration;

/// Bucket boundaries in seconds for `bazel.spawn.duration`, from a cache hit
/// to a long link or test.
const SPAWN_DURATION_BOUNDARIES: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0,
];

pub(super) struct SpawnInstruments {
    spawns: Counter<u64>,
    duration: Histogram<f64>,
}

impl SpawnInstruments {
    pub(super) fn new(meter: &Meter) -> Self {
        Self {
            spawns: meter
                .u64_counter("bazel.spawns")
                .with_unit("{spawn}")
                .with_description("Spawns by mnemonic, runner, cache hit and outcome")
                .build(),
            duration: meter
                .f64_histogram("bazel.spawn.duration")
                .with_unit("s")
                .with_description("Wall time of a spawn in this build, cache hits included")
                .with_boundaries(SPAWN_DURATION_BOUNDARIES.to_vec())
                .build(),
        }
    }

    /// `success` is whether the spawn succeeded; a spawn without a
    /// `total_time` is counted but not timed.
    pub(super) fn record(&self, spawn: &Spawn, success: bool) {
        let mnemonic = KeyValue::new("mnemonic", spawn.mnemonic.clone());
        let runner = KeyValue::new("runner", spawn.runner.clone());
        self.spawns.add(
            1,
            &[
                mnemonic.clone(),
                runner.clone(),
                KeyValue::new("cache_hit", spawn.cache_hit),
                KeyValue::new("success", success),
            ],
        );
        if let Some(total) = spawn.metrics.as_ref().and_then(|m| m.total_time.as_ref()) {
            self.duration
                .record(duration(total).as_secs_f64(), &[mnemonic, runner]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::tools::protos::SpawnMetrics;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
        data::{AggregatedMetrics, MetricData},
    };

    #[test]
    fn spawns_are_counted_and_timed_by_mnemonic_and_runner() {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        let instruments = SpawnInstruments::new(&provider.meter("test"));
        let timed = Spawn {
            mnemonic: "CppCompile".into(),
            runner: "remote".into(),
            metrics: Some(SpawnMetrics {
                total_time: Some(axl_proto::Duration {
                    seconds: 2,
                    nanos: 500_000_000,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        instruments.record(&timed, true);
        instruments.record(&timed, true);
        instruments.record(
            &Spawn {
                mnemonic: "CppCompile".into(),
                runner: "remote".into(),
                ..Default::default()
            },
            false,
        );
        provider.force_flush().unwrap();

        let metrics = exporter.get_finished_metrics().unwrap();
        let mut counted = Vec::new();
        let mut timed_sum = None;
        for metric in metrics
            .iter()
            .flat_map(|r| r.scope_metrics())
            .flat_map(|s| s.metrics())
        {
            match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                    for p in sum.data_points() {
                        let success = p
                            .attributes()
                            .find(|kv| kv.key.as_str() == "success")
                            .map(|kv| kv.value.to_string());
                        counted.push((success, p.value()));
                    }
                }
                AggregatedMetrics::F64(MetricData::Histogram(h)) => {
                    let p = h.data_points().next().unwrap();
                    timed_sum = Some((p.count(), p.sum()));
                }
                _ => {}
            }
        }
        counted.sort();
        assert_eq!(
            counted,
            [(Some("false".into()), 1), (Some("true".into()), 2)]
        );
        assert_eq!(timed_sum, Some((2, 5.0)), "an untimed spawn is not timed");
    }
}
