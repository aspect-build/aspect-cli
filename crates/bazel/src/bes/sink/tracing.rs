//! BES → OpenTelemetry trace sink.
//!
//! Consumes the build event stream on its own thread and turns it into spans
//! and span events under the active `ctx.bazel.build` span:
//!
//! - `events` covers the stream, and `build_tool` the Bazel invocation
//!   (`Started` → `BuildFinished`). Both are ordinary `tracing` spans, timed
//!   by when the sink sees those events.
//! - `action`, `test` and `test_attempt` describe work Bazel already finished
//!   by the time its event arrives, so they take their times from the event.
//!   A cached test attempt (local or remote) carries the times of the run that
//!   produced it, so it is instead a zero-length span at the moment this build
//!   reported it, and a `test` spans only this build's uncached attempts (see
//!   [`TestWindows`]). They are [`CompletedSpan`]s.
//! - Everything else becomes a span event, except progress and
//!   `NamedSetOfFiles`, which are too frequent to be worth recording.
//!
//! The same loop records the [`BesMetrics`] from test and build-metrics
//! events, with the command from `Started` and the outcome from
//! `BuildFinished` on the build durations.

use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, mpsc::RecvError},
    thread::{self, JoinHandle},
    time::SystemTime,
};

use axl_proto::build_event_stream::{
    ActionExecuted, TestResult, TestStatus, TestSummary,
    build_event::Payload,
    build_event_id::{ActionCompletedId, Id, TestResultId, TestSummaryId},
};
use opentelemetry::{
    Array, KeyValue, StringValue, Value, global,
    trace::{Status, Tracer},
};
use tracing::{Level, span::EnteredSpan};

use super::events::{attempt_cached, attempt_duration, time_or_legacy};
use super::metrics::{BesMetrics, Invocation};
use crate::bes::BuildEventEnvelope;
use crate::fanout::{SinkOutcome, Subscriber};
use crate::otel::{CompletedSpan, system_time};

#[derive(Debug)]
pub struct Tracing {}

/// Instrumentation scope of the [`CompletedSpan`]s and of the [`BesMetrics`].
const TELEMETRY_SCOPE: &str = "aspect-cli.bes";

impl Tracing {
    pub fn spawn(recv: Subscriber<Arc<BuildEventEnvelope>>) -> JoinHandle<SinkOutcome> {
        let events_span = tracing::info_span!("events");
        thread::spawn(move || {
            let _events_guard = events_span.enter();
            let metrics = BesMetrics::new(&global::meter(TELEMETRY_SCOPE));
            consume(&recv, &global::tracer(TELEMETRY_SCOPE), &metrics);
            Ok(())
        })
    }
}

/// Reads events until the stream closes, recording each in the current
/// `tracing` span, exporting [`CompletedSpan`]s through `tracer` and
/// recording `metrics`.
fn consume<T: Tracer>(
    recv: &Subscriber<Arc<BuildEventEnvelope>>,
    tracer: &T,
    metrics: &BesMetrics,
) {
    let mut build_span: Option<EnteredSpan> = None;
    let mut test_windows = TestWindows::default();
    let mut invocation = Invocation::default();
    loop {
        let envelope = match recv.recv() {
            Ok(e) => e,
            Err(RecvError) => break,
        };
        let event = &envelope.event;
        let Some(id) = event.id.as_ref().and_then(|w| w.id.as_ref()) else {
            continue;
        };
        let Some(payload) = event.payload.as_ref() else {
            continue;
        };

        match (payload, id) {
            (_, Id::Fetch(id)) => {
                tracing::event!(name: "fetch", Level::INFO, url = id.url);
            }
            (Payload::OptionsParsed(opt), Id::OptionsParsed(_)) => {
                tracing::event!(
                    name: "options_parsed",
                    Level::INFO,
                    build_tool = opt.tool_tag,
                    command_line = ?opt.cmd_line
                );
            }
            (Payload::Action(action), Id::ActionCompleted(id)) => match action_span(action, id) {
                Some(span) => span.export(tracer),
                None => tracing::event!(name: "action_completed", Level::INFO, label = id.label),
            },
            (Payload::Started(s), Id::Started(_)) => {
                if build_span.is_some() {
                    tracing::warn!("ignoring duplicate Started event");
                    continue;
                }
                invocation.command = Some(s.command.clone()).filter(|c| !c.is_empty());
                build_span = Some(
                    tracing::info_span!(
                        "build_tool",
                        version = s.build_tool_version,
                        pid = s.server_pid,
                        uuid = s.uuid,
                        current_dir = s.working_directory,
                        root_dir = s.workspace_directory,
                    )
                    .entered(),
                );
            }
            (Payload::Finished(finished), Id::BuildFinished(_)) => {
                #[allow(deprecated)]
                let success = finished
                    .exit_code
                    .as_ref()
                    .map_or(finished.overall_success, |exit| exit.code == 0);
                invocation.success = Some(success);
                if build_span.take().is_none() {
                    tracing::warn!("BuildFinished without prior Started");
                }
            }
            (Payload::Completed(target), Id::TargetCompleted(id)) => {
                tracing::event!(
                    name: "target_completed",
                    Level::INFO,
                    label = id.label,
                    aspect = id.aspect,
                    success = target.success
                );
            }
            (Payload::TestSummary(summary), Id::TestSummary(id)) => {
                metrics.record_test_summary(summary);
                let window = test_windows.take(&id.label);
                test_span(summary, id, window, SystemTime::now()).export(tracer);
            }
            (Payload::TestResult(result), Id::TestResult(id)) => {
                metrics.record_test_result(result);
                let span = test_attempt_span(result, id, SystemTime::now());
                if !attempt_cached(result) {
                    test_windows.record(&id.label, span.start, span.end);
                }
                span.export(tracer);
            }
            (Payload::BuildMetrics(build_metrics), _) => {
                metrics.record_build_metrics(build_metrics, &invocation);
                tracing::event!(name: "bes_event", Level::INFO, kind = id.as_str_name());
            }
            (Payload::Progress(_), _) | (Payload::NamedSetOfFiles(_), _) => {}
            (_, id) => {
                tracing::event!(name: "bes_event", Level::INFO, kind = id.as_str_name());
            }
        }
    }
}

/// For each test label, the window its uncached attempts ran in during this
/// build, from their `TestResult`s. A `TestSummary`'s own first-start and
/// last-stop include cached attempts from earlier runs.
#[derive(Debug, Default)]
struct TestWindows(HashMap<String, (SystemTime, SystemTime)>);

impl TestWindows {
    fn record(&mut self, label: &str, start: SystemTime, end: SystemTime) {
        self.0
            .entry(label.to_owned())
            .and_modify(|(s, e)| {
                *s = (*s).min(start);
                *e = (*e).max(end);
            })
            .or_insert((start, end));
    }

    fn take(&mut self, label: &str) -> Option<(SystemTime, SystemTime)> {
        self.0.remove(label)
    }
}

/// The `action` span, or `None` when Bazel reported no start or end time.
fn action_span(action: &ActionExecuted, id: &ActionCompletedId) -> Option<CompletedSpan> {
    let (start, end) = (action.start_time.as_ref()?, action.end_time.as_ref()?);
    let mut attributes = vec![
        KeyValue::new("label", id.label.clone()),
        KeyValue::new("success", action.success),
        KeyValue::new("mnemonic", action.r#type.clone()),
        KeyValue::new("exit_code", i64::from(action.exit_code)),
        KeyValue::new("command_line", string_array(&action.command_line)),
    ];
    attributes.extend(
        [
            debug_attribute("stdout", action.stdout.as_ref()),
            debug_attribute("stderr", action.stderr.as_ref()),
            debug_attribute("primary_output", action.primary_output.as_ref()),
            debug_attribute("failure_detail", action.failure_detail.as_ref()),
            debug_attribute(
                "action_metadata_logs",
                non_empty(&action.action_metadata_logs),
            ),
            debug_attribute("strategy_details", non_empty(&action.strategy_details)),
        ]
        .into_iter()
        .flatten(),
    );
    Some(CompletedSpan {
        name: "action",
        start: system_time(start),
        end: system_time(end),
        status: if action.success {
            Status::Unset
        } else {
            Status::error(format!("exit code {}", action.exit_code))
        },
        attributes,
    })
}

/// The `test` span: a target's final outcome across every run, shard and
/// attempt. It covers `window`, this build's uncached attempts; with none, a
/// fully cached test is a zero-length span at `reported`, and otherwise the
/// summary's own times are used.
fn test_span(
    summary: &TestSummary,
    id: &TestSummaryId,
    window: Option<(SystemTime, SystemTime)>,
    reported: SystemTime,
) -> CompletedSpan {
    let status = TestStatus::try_from(summary.overall_status).unwrap_or(TestStatus::NoStatus);
    #[allow(deprecated)]
    let (start, end) = match window {
        Some(window) => window,
        None if summary.total_num_cached > 0 => (reported, reported),
        None => (
            time_or_legacy(
                summary.first_start_time.as_ref(),
                summary.first_start_time_millis,
            ),
            time_or_legacy(
                summary.last_stop_time.as_ref(),
                summary.last_stop_time_millis,
            ),
        ),
    };
    CompletedSpan {
        name: "test",
        start,
        end,
        status: test_status(status),
        attributes: vec![
            KeyValue::new("label", id.label.clone()),
            KeyValue::new("status", status.as_str_name()),
            KeyValue::new(
                "success",
                matches!(status, TestStatus::Passed | TestStatus::Flaky),
            ),
            KeyValue::new("run_count", i64::from(summary.run_count)),
            KeyValue::new("shard_count", i64::from(summary.shard_count)),
            KeyValue::new("attempt_count", i64::from(summary.attempt_count)),
            KeyValue::new("total_run_count", i64::from(summary.total_run_count)),
            KeyValue::new("cached", i64::from(summary.total_num_cached)),
        ],
    }
}

/// The `test_attempt` span: one run of one shard. A flaky test has a failed
/// attempt under a passing `test`. A cached attempt did no work in this build,
/// so it is a zero-length span at `reported`.
fn test_attempt_span(
    result: &TestResult,
    id: &TestResultId,
    reported: SystemTime,
) -> CompletedSpan {
    let status = TestStatus::try_from(result.status).unwrap_or(TestStatus::NoStatus);
    #[allow(deprecated)]
    let (start, end) = if attempt_cached(result) {
        (reported, reported)
    } else {
        let start = time_or_legacy(
            result.test_attempt_start.as_ref(),
            result.test_attempt_start_millis_epoch,
        );
        (start, start + attempt_duration(result))
    };
    CompletedSpan {
        name: "test_attempt",
        start,
        end,
        status: test_status(status),
        attributes: vec![
            KeyValue::new("label", id.label.clone()),
            KeyValue::new("status", status.as_str_name()),
            KeyValue::new("cached", attempt_cached(result)),
            KeyValue::new("run", i64::from(id.run)),
            KeyValue::new("shard", i64::from(id.shard)),
            KeyValue::new("attempt", i64::from(id.attempt)),
        ],
    }
}

/// Span status for a Bazel test outcome. Failing outcomes are errors; `PASSED`
/// and `FLAKY` (which ultimately passed) are ok; `NO_STATUS`/`INCOMPLETE` stay
/// unset. Mirrors `bazel_results.axl`'s `_FAILED_STATUSES`.
fn test_status(status: TestStatus) -> Status {
    match status {
        TestStatus::Passed | TestStatus::Flaky => Status::Ok,
        TestStatus::Failed
        | TestStatus::Timeout
        | TestStatus::FailedToBuild
        | TestStatus::RemoteFailure
        | TestStatus::ToolHaltedBeforeTesting => Status::error(status.as_str_name()),
        _ => Status::Unset,
    }
}

fn string_array(items: &[String]) -> Value {
    Value::Array(Array::String(
        items.iter().cloned().map(StringValue::from).collect(),
    ))
}

/// A Debug-formatted attribute, or `None` when there is no value.
fn debug_attribute<V: Debug>(key: &'static str, value: Option<V>) -> Option<KeyValue> {
    value.map(|v| KeyValue::new(key, format!("{v:?}")))
}

fn non_empty<T>(items: &[T]) -> Option<&[T]> {
    (!items.is_empty()).then_some(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use axl_proto::Timestamp;
    use axl_proto::build_event_stream::{
        BuildEvent, BuildEventId, BuildFinished, BuildMetrics, BuildStarted,
        build_event_id::BuildFinishedId, build_event_id::BuildStartedId, build_finished,
        build_metrics, test_result,
    };
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
    use prost::{Message as _, bytes::Bytes};
    use tracing_subscriber::layer::SubscriberExt as _;

    use crate::fanout::Broadcaster;
    use crate::otel::test_support::{Point, point, points};

    fn ts(seconds: i64, nanos: i32) -> Timestamp {
        Timestamp { seconds, nanos }
    }

    fn at(seconds: u64, nanos: u32) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::new(seconds, nanos)
    }

    fn attribute<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
        span.attributes
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| &kv.value)
    }

    fn event(id: Id, payload: Payload) -> Arc<BuildEventEnvelope> {
        let event = BuildEvent {
            id: Some(BuildEventId { id: Some(id) }),
            payload: Some(payload),
            ..Default::default()
        };
        let encoded = Bytes::from(event.encode_to_vec());
        Arc::new(BuildEventEnvelope { event, encoded })
    }

    /// Runs `events` through [`consume`] under a `tracing` span named `parent`
    /// and returns every exported span and recorded metric point. Uses its own
    /// providers and subscriber, so it never touches the globals.
    fn run(events: Vec<Arc<BuildEventEnvelope>>) -> (Vec<SpanData>, Vec<Point>) {
        let span_exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(span_exporter.clone())
            .build();
        let metric_exporter = InMemoryMetricExporter::default();
        let meter_provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(metric_exporter.clone()).build())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("layer")));

        let broadcaster = Broadcaster::new();
        let recv = broadcaster.subscribe();
        for e in events {
            broadcaster.send(e);
        }
        broadcaster.close();

        let tracer = provider.tracer(TELEMETRY_SCOPE);
        let metrics = BesMetrics::new(&meter_provider.meter(TELEMETRY_SCOPE));
        tracing::subscriber::with_default(subscriber, || {
            let _parent = tracing::info_span!("parent").entered();
            consume(&recv, &tracer, &metrics);
        });
        provider.force_flush().unwrap();
        meter_provider.force_flush().unwrap();

        (
            span_exporter.get_finished_spans().unwrap(),
            points(&metric_exporter),
        )
    }

    fn export(events: Vec<Arc<BuildEventEnvelope>>) -> Vec<SpanData> {
        run(events).0
    }

    fn find<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
        spans.iter().find(|s| s.name == name).unwrap_or_else(|| {
            let names: Vec<_> = spans.iter().map(|s| &s.name).collect();
            panic!("no {name} span in {names:?}")
        })
    }

    #[test]
    fn stream_exports_completed_spans_with_bazel_times_under_build_tool() {
        let spans = export(vec![
            event(
                Id::Started(BuildStartedId::default()),
                Payload::Started(BuildStarted::default()),
            ),
            event(
                Id::ActionCompleted(ActionCompletedId {
                    label: "//:bad".into(),
                    ..Default::default()
                }),
                Payload::Action(ActionExecuted {
                    success: false,
                    exit_code: 3,
                    r#type: "Genrule".into(),
                    command_line: vec!["bash".into(), "-c".into()],
                    start_time: Some(ts(100, 250_000_000)),
                    end_time: Some(ts(100, 288_000_000)),
                    ..Default::default()
                }),
            ),
            event(
                Id::TestResult(TestResultId {
                    label: "//:t".into(),
                    run: 1,
                    shard: 2,
                    attempt: 3,
                    ..Default::default()
                }),
                Payload::TestResult(TestResult {
                    status: TestStatus::Failed as i32,
                    test_attempt_start: Some(ts(200, 1)),
                    test_attempt_duration: Some(axl_proto::Duration {
                        seconds: 0,
                        nanos: 59_000_000,
                    }),
                    ..Default::default()
                }),
            ),
            event(
                Id::BuildFinished(BuildFinishedId::default()),
                Payload::Finished(BuildFinished::default()),
            ),
        ]);

        let parent = find(&spans, "parent");
        let build_tool = find(&spans, "build_tool");
        assert_eq!(build_tool.parent_span_id, parent.span_context.span_id());

        let action = find(&spans, "action");
        assert_eq!(action.parent_span_id, build_tool.span_context.span_id());
        assert_eq!(
            action.span_context.trace_id(),
            parent.span_context.trace_id()
        );
        assert_eq!(action.start_time, at(100, 250_000_000));
        assert_eq!(action.end_time, at(100, 288_000_000));
        assert_eq!(action.status, Status::error("exit code 3"));
        assert_eq!(
            attribute(action, "label"),
            Some(&Value::from("//:bad".to_string()))
        );
        assert_eq!(
            attribute(action, "command_line"),
            Some(&string_array(&["bash".into(), "-c".into()]))
        );
        assert_eq!(attribute(action, "stdout"), None);

        let attempt = find(&spans, "test_attempt");
        assert_eq!(attempt.parent_span_id, build_tool.span_context.span_id());
        assert_eq!(attempt.start_time, at(200, 1));
        assert_eq!(attempt.end_time, at(200, 59_000_001));
        assert_eq!(attempt.status, Status::error("FAILED"));
        assert_eq!(attribute(attempt, "attempt"), Some(&Value::I64(3)));
    }

    #[test]
    fn stream_records_test_and_action_cache_metrics() {
        let (_, points) = run(vec![
            event(
                Id::TestResult(TestResultId::default()),
                Payload::TestResult(TestResult {
                    status: TestStatus::Passed as i32,
                    ..Default::default()
                }),
            ),
            event(
                Id::TestSummary(TestSummaryId::default()),
                Payload::TestSummary(TestSummary::default()),
            ),
            event(
                Id::BuildMetrics(Default::default()),
                Payload::BuildMetrics(BuildMetrics {
                    action_summary: Some(build_metrics::ActionSummary {
                        action_cache_statistics: Some(Default::default()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            ),
        ]);
        let mut names: Vec<&str> = points.iter().map(|(name, ..)| name.as_str()).collect();
        names.dedup();
        assert_eq!(
            names,
            [
                "bazel.action_cache.lookups",
                "bazel.test.attempt.duration",
                "bazel.test.results"
            ]
        );
    }

    #[test]
    fn build_duration_carries_the_command_and_outcome_from_the_stream() {
        let (_, points) = run(vec![
            event(
                Id::Started(BuildStartedId::default()),
                Payload::Started(BuildStarted {
                    command: "test".into(),
                    ..Default::default()
                }),
            ),
            event(
                Id::BuildFinished(BuildFinishedId::default()),
                Payload::Finished(BuildFinished {
                    exit_code: Some(build_finished::ExitCode {
                        code: 3,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            ),
            event(
                Id::BuildMetrics(Default::default()),
                Payload::BuildMetrics(BuildMetrics {
                    timing_metrics: Some(build_metrics::TimingMetrics {
                        wall_time_in_ms: 4_000,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            ),
        ]);
        assert_eq!(
            points,
            [point(
                "bazel.build.duration",
                &["command=test", "success=false"],
                4.0
            )]
        );
    }

    fn wall_time(ms: i64) -> Arc<BuildEventEnvelope> {
        event(
            Id::BuildMetrics(Default::default()),
            Payload::BuildMetrics(BuildMetrics {
                timing_metrics: Some(build_metrics::TimingMetrics {
                    wall_time_in_ms: ms,
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
    }

    #[test]
    fn build_duration_without_started_or_finished_has_no_attributes() {
        let (_, points) = run(vec![wall_time(2_000)]);
        assert_eq!(points, [point("bazel.build.duration", &[], 2.0)]);
    }

    #[test]
    #[allow(deprecated)]
    fn build_success_falls_back_to_overall_success_without_an_exit_code() {
        let (_, points) = run(vec![
            event(
                Id::BuildFinished(BuildFinishedId::default()),
                Payload::Finished(BuildFinished {
                    overall_success: true,
                    ..Default::default()
                }),
            ),
            wall_time(1_000),
        ]);
        assert_eq!(
            points,
            [point("bazel.build.duration", &["success=true"], 1.0)]
        );
    }

    #[test]
    fn action_without_times_is_not_a_span() {
        let spans = export(vec![event(
            Id::ActionCompleted(ActionCompletedId::default()),
            Payload::Action(ActionExecuted {
                start_time: Some(ts(1, 0)),
                ..Default::default()
            }),
        )]);
        assert!(spans.iter().all(|s| s.name != "action"));
        assert_eq!(find(&spans, "parent").events.len(), 1);
    }

    fn summary(total_num_cached: i32) -> TestSummary {
        TestSummary {
            overall_status: TestStatus::Flaky as i32,
            attempt_count: 2,
            total_num_cached,
            first_start_time: Some(ts(10, 5)),
            last_stop_time: Some(ts(12, 7)),
            ..Default::default()
        }
    }

    fn summary_id() -> TestSummaryId {
        TestSummaryId {
            label: "//:t".into(),
            ..Default::default()
        }
    }

    fn attempt(cached_locally: bool, cached_remotely: bool) -> TestResult {
        TestResult {
            status: TestStatus::Passed as i32,
            cached_locally,
            execution_info: Some(test_result::ExecutionInfo {
                cached_remotely,
                ..Default::default()
            }),
            test_attempt_start: Some(ts(10, 0)),
            test_attempt_duration: Some(axl_proto::Duration {
                seconds: 2,
                nanos: 0,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn uncached_test_span_takes_summary_times_and_status() {
        let span = test_span(&summary(0), &summary_id(), None, at(99, 0));
        assert_eq!((span.start, span.end), (at(10, 5), at(12, 7)));
        assert_eq!(span.status, Status::Ok);
    }

    #[test]
    fn test_span_covers_this_builds_uncached_attempts() {
        let window = Some((at(20, 0), at(21, 0)));
        let span = test_span(&summary(1), &summary_id(), window, at(99, 0));
        assert_eq!((span.start, span.end), (at(20, 0), at(21, 0)));
    }

    #[test]
    fn fully_cached_test_span_is_at_report_time() {
        let span = test_span(&summary(2), &summary_id(), None, at(99, 0));
        assert_eq!((span.start, span.end), (at(99, 0), at(99, 0)));
    }

    #[test]
    fn cached_attempts_are_at_report_time() {
        for (local, remote) in [(true, false), (false, true)] {
            let span =
                test_attempt_span(&attempt(local, remote), &TestResultId::default(), at(99, 0));
            assert_eq!((span.start, span.end), (at(99, 0), at(99, 0)));
            assert!(span.attributes.contains(&KeyValue::new("cached", true)));
        }
        let span = test_attempt_span(&attempt(false, false), &TestResultId::default(), at(99, 0));
        assert_eq!((span.start, span.end), (at(10, 0), at(12, 0)));
    }

    #[test]
    fn windows_merge_attempts_per_label() {
        let mut windows = TestWindows::default();
        windows.record("//:a", at(5, 0), at(6, 0));
        windows.record("//:a", at(3, 0), at(4, 0));
        windows.record("//:b", at(9, 0), at(10, 0));
        assert_eq!(windows.take("//:a"), Some((at(3, 0), at(6, 0))));
        assert_eq!(windows.take("//:a"), None);
        assert_eq!(windows.take("//:b"), Some((at(9, 0), at(10, 0))));
    }

    #[test]
    fn export_clamps_end_before_start() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        CompletedSpan {
            name: "x",
            start: at(5, 0),
            end: at(4, 0),
            status: Status::Unset,
            attributes: Vec::new(),
        }
        .export(&provider.tracer("t"));
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans[0].end_time, at(5, 0));
    }

    #[test]
    fn failing_statuses_map_to_error() {
        for s in [
            TestStatus::Failed,
            TestStatus::Timeout,
            TestStatus::FailedToBuild,
            TestStatus::RemoteFailure,
            TestStatus::ToolHaltedBeforeTesting,
        ] {
            assert_eq!(test_status(s), Status::error(s.as_str_name()));
        }
    }

    #[test]
    fn passing_and_flaky_map_to_ok() {
        assert_eq!(test_status(TestStatus::Passed), Status::Ok);
        assert_eq!(test_status(TestStatus::Flaky), Status::Ok);
    }

    #[test]
    fn indeterminate_statuses_stay_unset() {
        assert_eq!(test_status(TestStatus::NoStatus), Status::Unset);
        assert_eq!(test_status(TestStatus::Incomplete), Status::Unset);
    }
}
