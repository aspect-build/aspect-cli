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
    use crate::otel::test_support::{point, recorded};
    use axl_proto::tools::protos::SpawnMetrics;

    fn compile() -> Spawn {
        Spawn {
            mnemonic: "CppCompile".into(),
            runner: "remote".into(),
            ..Default::default()
        }
    }

    #[test]
    fn spawns_are_counted_and_timed_by_mnemonic_and_runner() {
        let timed = Spawn {
            metrics: Some(SpawnMetrics {
                total_time: Some(axl_proto::Duration {
                    seconds: 2,
                    nanos: 500_000_000,
                }),
                ..Default::default()
            }),
            ..compile()
        };
        let points = recorded(|meter| {
            let instruments = SpawnInstruments::new(meter);
            instruments.record(&timed, true);
            instruments.record(&timed, true);
            instruments.record(&compile(), false);
        });
        let attrs = |success| {
            [
                "cache_hit=false",
                "mnemonic=CppCompile",
                "runner=remote",
                success,
            ]
        };
        assert_eq!(
            points,
            [
                point(
                    "bazel.spawn.duration",
                    &["mnemonic=CppCompile", "runner=remote"],
                    5.0
                ),
                point("bazel.spawns", &attrs("success=false"), 1.0),
                point("bazel.spawns", &attrs("success=true"), 2.0),
            ]
        );
    }

    #[test]
    fn a_spawn_without_a_total_time_is_counted_but_not_timed() {
        let points = recorded(|meter| SpawnInstruments::new(meter).record(&compile(), true));
        let names: Vec<&str> = points.iter().map(|(name, ..)| name.as_str()).collect();
        assert_eq!(names, ["bazel.spawns"]);
    }
}
