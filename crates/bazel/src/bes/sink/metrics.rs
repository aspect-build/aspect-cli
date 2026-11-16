//! BES → OpenTelemetry metrics, recorded by the trace sink as events arrive.
//!
//! | Instrument | Type | Unit | Attributes |
//! |---|---|---|---|
//! | `bazel.test.results` | counter | `{test}` | `status`, `cached` |
//! | `bazel.test.attempt.duration` | histogram | `s` | `status` |
//! | `bazel.action_cache.lookups` | counter | `{lookup}` | `result`, `reason` |
//! | `bazel.build.duration` | histogram | `s` | `command`, `success` |
//! | `bazel.build.phase.duration` | histogram | `s` | `command`, `success`, `phase` |
//!
//! No attribute carries a target label, so the number of series stays bounded
//! however large the build. `bazel.test.attempt.duration` records only
//! attempts that ran in this build; a cached attempt took no time here.
//! `bazel.build.phase.duration` has an `analysis` and an `execution` phase,
//! which overlap when Bazel interleaves them.

use axl_proto::{
    blaze::action_cache_statistics::MissReason,
    build_event_stream::{
        BuildMetrics, TestResult, TestStatus, TestSummary, build_metrics::TimingMetrics,
    },
};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, Meter},
};

use super::events::{attempt_cached, attempt_duration};

/// Bucket boundaries in seconds for `bazel.test.attempt.duration`, spanning
/// sub-second unit tests to hour-long integration tests. The SDK's defaults
/// are sized for milliseconds.
const TEST_DURATION_BOUNDARIES: &[f64] = &[
    0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0,
];

/// Bucket boundaries in seconds for the build durations, from a no-op build
/// to a multi-hour one.
const BUILD_DURATION_BOUNDARIES: &[f64] = &[
    0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0, 1800.0, 3600.0, 7200.0,
];

/// What the stream said about the invocation before its `BuildMetrics`.
#[derive(Debug, Default)]
pub(super) struct Invocation {
    /// The Bazel command, from `BuildStarted`.
    pub command: Option<String>,
    /// Whether Bazel exited 0, from `BuildFinished`. A build that found no
    /// tests to run (exit 4) is not a success, as for Bazel's own
    /// `overall_success`.
    pub success: Option<bool>,
}

pub(super) struct BesMetrics {
    test_results: Counter<u64>,
    test_attempt_duration: Histogram<f64>,
    action_cache_lookups: Counter<u64>,
    build_duration: Histogram<f64>,
    build_phase_duration: Histogram<f64>,
}

impl BesMetrics {
    pub(super) fn new(meter: &Meter) -> Self {
        Self {
            test_results: meter
                .u64_counter("bazel.test.results")
                .with_unit("{test}")
                .with_description("Test targets by final status")
                .build(),
            test_attempt_duration: meter
                .f64_histogram("bazel.test.attempt.duration")
                .with_unit("s")
                .with_description("Duration of test attempts that ran in this build")
                .with_boundaries(TEST_DURATION_BOUNDARIES.to_vec())
                .build(),
            action_cache_lookups: meter
                .u64_counter("bazel.action_cache.lookups")
                .with_unit("{lookup}")
                .with_description("Local action cache lookups by result and miss reason")
                .build(),
            build_duration: meter
                .f64_histogram("bazel.build.duration")
                .with_unit("s")
                .with_description("Wall time of a Bazel build, as Bazel measured it")
                .with_boundaries(BUILD_DURATION_BOUNDARIES.to_vec())
                .build(),
            build_phase_duration: meter
                .f64_histogram("bazel.build.phase.duration")
                .with_unit("s")
                .with_description("Wall time of a Bazel build's analysis and execution phases")
                .with_boundaries(BUILD_DURATION_BOUNDARIES.to_vec())
                .build(),
        }
    }

    /// One test target's final outcome. `cached` is true when every run of it
    /// came from a cache.
    pub(super) fn record_test_summary(&self, summary: &TestSummary) {
        let status = TestStatus::try_from(summary.overall_status).unwrap_or(TestStatus::NoStatus);
        let cached =
            summary.total_num_cached > 0 && summary.total_num_cached >= summary.total_run_count;
        self.test_results.add(
            1,
            &[
                KeyValue::new("status", status.as_str_name()),
                KeyValue::new("cached", cached),
            ],
        );
    }

    pub(super) fn record_test_result(&self, result: &TestResult) {
        if attempt_cached(result) {
            return;
        }
        let status = TestStatus::try_from(result.status).unwrap_or(TestStatus::NoStatus);
        self.test_attempt_duration.record(
            attempt_duration(result).as_secs_f64(),
            &[KeyValue::new("status", status.as_str_name())],
        );
    }

    /// Action cache lookups, and the build's timing for `invocation`.
    pub(super) fn record_build_metrics(&self, metrics: &BuildMetrics, invocation: &Invocation) {
        if let Some(timing) = &metrics.timing_metrics {
            self.record_timing(timing, invocation);
        }
        self.record_action_cache(metrics);
    }

    /// Durations Bazel reported as `0` are left out.
    fn record_timing(&self, timing: &TimingMetrics, invocation: &Invocation) {
        let attributes: Vec<KeyValue> = invocation
            .command
            .as_ref()
            .map(|c| KeyValue::new("command", c.clone()))
            .into_iter()
            .chain(invocation.success.map(|s| KeyValue::new("success", s)))
            .collect();
        if timing.wall_time_in_ms > 0 {
            self.build_duration
                .record(seconds(timing.wall_time_in_ms), &attributes);
        }
        for (phase, ms) in [
            ("analysis", timing.analysis_phase_time_in_ms),
            ("execution", timing.execution_phase_time_in_ms),
        ] {
            if ms > 0 {
                let mut attributes = attributes.clone();
                attributes.push(KeyValue::new("phase", phase));
                self.build_phase_duration.record(seconds(ms), &attributes);
            }
        }
    }

    /// Hits, and misses by reason. Misses Bazel reported without a reason are
    /// recorded with no `reason` attribute.
    fn record_action_cache(&self, metrics: &BuildMetrics) {
        let Some(stats) = metrics
            .action_summary
            .as_ref()
            .and_then(|summary| summary.action_cache_statistics.as_ref())
        else {
            return;
        };
        let hit = [KeyValue::new("result", "hit")];
        self.action_cache_lookups.add(count(stats.hits), &hit);

        let mut explained = 0;
        for detail in stats.miss_details.iter().filter(|d| d.count > 0) {
            let reason = MissReason::try_from(detail.reason)
                .map(|r| r.as_str_name())
                .unwrap_or("UNKNOWN");
            self.action_cache_lookups.add(
                count(detail.count),
                &[
                    KeyValue::new("result", "miss"),
                    KeyValue::new("reason", reason),
                ],
            );
            explained += count(detail.count);
        }
        let unexplained = count(stats.misses).saturating_sub(explained);
        if unexplained > 0 {
            self.action_cache_lookups
                .add(unexplained, &[KeyValue::new("result", "miss")]);
        }
    }
}

fn count(n: i32) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn seconds(ms: i64) -> f64 {
    ms as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otel::test_support::{self, Point, point};
    use axl_proto::{
        blaze::{ActionCacheStatistics, action_cache_statistics::MissDetail},
        build_event_stream::{build_metrics::ActionSummary, test_result::ExecutionInfo},
    };
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};

    fn recorded(record: impl FnOnce(&BesMetrics)) -> Vec<Point> {
        test_support::recorded(|meter| record(&BesMetrics::new(meter)))
    }

    #[test]
    fn test_summaries_count_by_status_and_full_caching() {
        let points = recorded(|m| {
            m.record_test_summary(&TestSummary {
                overall_status: TestStatus::Passed as i32,
                total_run_count: 2,
                total_num_cached: 2,
                ..Default::default()
            });
            m.record_test_summary(&TestSummary {
                overall_status: TestStatus::Failed as i32,
                total_run_count: 2,
                total_num_cached: 1,
                ..Default::default()
            });
        });
        assert_eq!(
            points,
            vec![
                point(
                    "bazel.test.results",
                    &["cached=false", "status=FAILED"],
                    1.0
                ),
                point("bazel.test.results", &["cached=true", "status=PASSED"], 1.0),
            ]
        );
    }

    #[test]
    fn attempt_duration_skips_cached_attempts() {
        let ran = |seconds| TestResult {
            status: TestStatus::Passed as i32,
            test_attempt_duration: Some(axl_proto::Duration { seconds, nanos: 0 }),
            ..Default::default()
        };
        let points = recorded(|m| {
            m.record_test_result(&ran(2));
            m.record_test_result(&ran(3));
            m.record_test_result(&TestResult {
                cached_locally: true,
                ..ran(100)
            });
            m.record_test_result(&TestResult {
                execution_info: Some(ExecutionInfo {
                    cached_remotely: true,
                    ..Default::default()
                }),
                ..ran(100)
            });
        });
        assert_eq!(
            points,
            vec![point(
                "bazel.test.attempt.duration",
                &["status=PASSED"],
                5.0
            )]
        );
    }

    #[test]
    fn attempt_duration_uses_second_buckets() {
        let (provider, exporter) = test_support::provider();
        BesMetrics::new(&provider.meter("test")).record_test_result(&TestResult {
            test_attempt_duration: Some(axl_proto::Duration {
                seconds: 3,
                nanos: 0,
            }),
            ..Default::default()
        });
        provider.force_flush().unwrap();
        let metrics = exporter.get_finished_metrics().unwrap();
        let metric = metrics[0]
            .scope_metrics()
            .next()
            .unwrap()
            .metrics()
            .next()
            .unwrap();
        let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else {
            panic!("expected an f64 histogram");
        };
        let point = histogram.data_points().next().unwrap();
        assert_eq!(point.bounds().collect::<Vec<_>>(), TEST_DURATION_BOUNDARIES);
        let counts: Vec<u64> = point.bucket_counts().collect();
        assert_eq!(
            counts[TEST_DURATION_BOUNDARIES.partition_point(|b| *b < 3.0)],
            1
        );
    }

    #[test]
    fn action_cache_lookups_split_hits_and_misses_by_reason() {
        let metrics = BuildMetrics {
            action_summary: Some(ActionSummary {
                action_cache_statistics: Some(ActionCacheStatistics {
                    hits: 7,
                    misses: 5,
                    miss_details: vec![
                        MissDetail {
                            reason: MissReason::NotCached as i32,
                            count: 3,
                        },
                        MissDetail {
                            reason: MissReason::DifferentFiles as i32,
                            count: 1,
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let points = recorded(|m| m.record_build_metrics(&metrics, &Invocation::default()));
        assert_eq!(
            points,
            vec![
                point(
                    "bazel.action_cache.lookups",
                    &["reason=DIFFERENT_FILES", "result=miss"],
                    1.0
                ),
                point(
                    "bazel.action_cache.lookups",
                    &["reason=NOT_CACHED", "result=miss"],
                    3.0
                ),
                point("bazel.action_cache.lookups", &["result=hit"], 7.0),
                point("bazel.action_cache.lookups", &["result=miss"], 1.0),
            ]
        );
    }

    #[test]
    fn fully_explained_misses_add_no_reasonless_point() {
        let metrics = BuildMetrics {
            action_summary: Some(ActionSummary {
                action_cache_statistics: Some(ActionCacheStatistics {
                    hits: 0,
                    misses: 2,
                    miss_details: vec![MissDetail {
                        reason: MissReason::DifferentDeps as i32,
                        count: 2,
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            recorded(|m| m.record_build_metrics(&metrics, &Invocation::default())),
            vec![
                point(
                    "bazel.action_cache.lookups",
                    &["reason=DIFFERENT_DEPS", "result=miss"],
                    2.0
                ),
                point("bazel.action_cache.lookups", &["result=hit"], 0.0),
            ]
        );
    }

    #[test]
    fn build_timing_is_recorded_by_command_and_outcome() {
        let metrics = BuildMetrics {
            timing_metrics: Some(TimingMetrics {
                wall_time_in_ms: 12_500,
                analysis_phase_time_in_ms: 2_000,
                execution_phase_time_in_ms: 0,
                ..Default::default()
            }),
            ..Default::default()
        };
        let invocation = Invocation {
            command: Some("test".into()),
            success: Some(false),
        };
        assert_eq!(
            recorded(|m| m.record_build_metrics(&metrics, &invocation)),
            vec![
                point(
                    "bazel.build.duration",
                    &["command=test", "success=false"],
                    12.5
                ),
                point(
                    "bazel.build.phase.duration",
                    &["command=test", "phase=analysis", "success=false"],
                    2.0
                ),
            ]
        );
    }

    #[test]
    fn build_metrics_without_cache_statistics_record_nothing() {
        assert!(
            recorded(|m| m.record_build_metrics(&BuildMetrics::default(), &Invocation::default()))
                .is_empty()
        );
    }
}
