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
//!   A `tracing` span can only be timed by when it is entered and dropped, so
//!   these are built as [`CompletedSpan`]s and exported with an OpenTelemetry
//!   tracer, parented to the current `tracing` span through
//!   `OpenTelemetrySpanExt::context`. That parent link needs the tracer and
//!   the `tracing-opentelemetry` layer to share one provider, which
//!   `aspect-cli`'s `trace::init` arranges by installing it globally.
//! - Everything else becomes a span event, except progress and
//!   `NamedSetOfFiles`, which are too frequent to be worth recording.

use std::{
    fmt::Debug,
    sync::{Arc, mpsc::RecvError},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

use axl_proto::{
    Timestamp,
    build_event_stream::{
        ActionExecuted, TestResult, TestStatus, TestSummary,
        build_event::Payload,
        build_event_id::{ActionCompletedId, Id, TestResultId, TestSummaryId},
    },
};
use opentelemetry::{
    Array, KeyValue, StringValue, Value, global,
    trace::{Span as _, Status, Tracer},
};
use tracing::{Level, span::EnteredSpan};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use crate::bes::BuildEventEnvelope;
use crate::fanout::{SinkOutcome, Subscriber};

#[derive(Debug)]
pub struct Tracing {}

/// Instrumentation scope of the spans exported as [`CompletedSpan`]s.
const TRACER_NAME: &str = "aspect-cli.bes";

impl Tracing {
    pub fn spawn(recv: Subscriber<Arc<BuildEventEnvelope>>) -> JoinHandle<SinkOutcome> {
        let events_span = tracing::info_span!("events");
        thread::spawn(move || {
            let _events_guard = events_span.enter();
            consume(&recv, &global::tracer(TRACER_NAME));
            Ok(())
        })
    }
}

/// Reads events until the stream closes, recording each in the current
/// `tracing` span and exporting [`CompletedSpan`]s through `tracer`.
fn consume<T: Tracer>(recv: &Subscriber<Arc<BuildEventEnvelope>>, tracer: &T) {
    let mut build_span: Option<EnteredSpan> = None;
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
            (Payload::Finished(_), Id::BuildFinished(_)) => {
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
                test_span(summary, id).export(tracer);
            }
            (Payload::TestResult(result), Id::TestResult(id)) => {
                test_attempt_span(result, id).export(tracer);
            }
            (Payload::Progress(_), _) | (Payload::NamedSetOfFiles(_), _) => {}
            (_, id) => {
                tracing::event!(name: "bes_event", Level::INFO, kind = id.as_str_name());
            }
        }
    }
}

/// A span for work that already finished, with the times Bazel reported.
#[derive(Debug)]
struct CompletedSpan {
    name: &'static str,
    start: SystemTime,
    end: SystemTime,
    status: Status,
    attributes: Vec<KeyValue>,
}

impl CompletedSpan {
    /// Exports the span as a child of the current `tracing` span. An end
    /// before the start is clamped to the start.
    fn export<T: Tracer>(self, tracer: &T) {
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
/// attempt.
fn test_span(summary: &TestSummary, id: &TestSummaryId) -> CompletedSpan {
    let status = TestStatus::try_from(summary.overall_status).unwrap_or(TestStatus::NoStatus);
    #[allow(deprecated)]
    let (start, end) = (
        time_or_legacy(
            summary.first_start_time.as_ref(),
            summary.first_start_time_millis,
        ),
        time_or_legacy(
            summary.last_stop_time.as_ref(),
            summary.last_stop_time_millis,
        ),
    );
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
/// attempt under a passing `test`.
fn test_attempt_span(result: &TestResult, id: &TestResultId) -> CompletedSpan {
    let status = TestStatus::try_from(result.status).unwrap_or(TestStatus::NoStatus);
    #[allow(deprecated)]
    let (start, duration) = (
        time_or_legacy(
            result.test_attempt_start.as_ref(),
            result.test_attempt_start_millis_epoch,
        ),
        duration_or_legacy(
            result.test_attempt_duration.as_ref(),
            result.test_attempt_duration_millis,
        ),
    );
    CompletedSpan {
        name: "test_attempt",
        start,
        end: start + duration,
        status: test_status(status),
        attributes: vec![
            KeyValue::new("label", id.label.clone()),
            KeyValue::new("status", status.as_str_name()),
            KeyValue::new("cached", result.cached_locally),
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

fn system_time(ts: &Timestamp) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::new(ts.seconds.max(0) as u64, ts.nanos.max(0) as u32)
}

/// Prefers the `Timestamp` field, then the deprecated `*_millis` field, then
/// `now`. Older Bazel versions and cached test summaries populate only the
/// millis field.
fn time_or_legacy(timestamp: Option<&Timestamp>, legacy_millis: i64) -> SystemTime {
    match timestamp {
        Some(t) => system_time(t),
        None if legacy_millis > 0 => {
            SystemTime::UNIX_EPOCH + Duration::from_millis(legacy_millis as u64)
        }
        None => SystemTime::now(),
    }
}

/// Prefers the `Duration` field, then the deprecated `*_millis` field.
fn duration_or_legacy(duration: Option<&axl_proto::Duration>, legacy_millis: i64) -> Duration {
    match duration {
        Some(d) => Duration::new(d.seconds.max(0) as u64, d.nanos.max(0) as u32),
        None => Duration::from_millis(legacy_millis.max(0) as u64),
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
    use axl_proto::build_event_stream::{
        BuildEvent, BuildEventId, BuildFinished, BuildStarted, build_event_id::BuildFinishedId,
        build_event_id::BuildStartedId,
    };
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
    use prost::{Message as _, bytes::Bytes};
    use tracing_subscriber::layer::SubscriberExt as _;

    use crate::fanout::Broadcaster;

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
    /// and returns every exported span. Uses its own provider and subscriber,
    /// so it never touches the global tracer.
    fn export(events: Vec<Arc<BuildEventEnvelope>>) -> Vec<SpanData> {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("layer")));

        let broadcaster = Broadcaster::new();
        let recv = broadcaster.subscribe();
        for e in events {
            broadcaster.send(e);
        }
        broadcaster.close();

        let tracer = provider.tracer(TRACER_NAME);
        tracing::subscriber::with_default(subscriber, || {
            let _parent = tracing::info_span!("parent").entered();
            consume(&recv, &tracer);
        });
        provider.force_flush().unwrap();
        exporter.get_finished_spans().unwrap()
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

    #[test]
    fn test_span_takes_summary_window_and_status() {
        let span = test_span(
            &TestSummary {
                overall_status: TestStatus::Flaky as i32,
                attempt_count: 2,
                first_start_time: Some(ts(10, 5)),
                last_stop_time: Some(ts(12, 7)),
                ..Default::default()
            },
            &TestSummaryId {
                label: "//:t".into(),
                ..Default::default()
            },
        );
        assert_eq!((span.start, span.end), (at(10, 5), at(12, 7)));
        assert_eq!(span.status, Status::Ok);
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

    #[test]
    fn timestamp_keeps_nanos() {
        assert_eq!(
            time_or_legacy(Some(&ts(42, 123_456_789)), 999_000),
            at(42, 123_456_789)
        );
    }

    #[test]
    fn timestamp_falls_back_to_legacy_millis() {
        assert_eq!(
            time_or_legacy(None, 1_723_000_000_250),
            at(1_723_000_000, 250_000_000)
        );
    }

    #[test]
    fn timestamp_uses_now_when_nothing_populated() {
        let before = SystemTime::now();
        assert!(time_or_legacy(None, 0) >= before);
    }

    #[test]
    fn duration_falls_back_to_legacy_millis() {
        assert_eq!(
            duration_or_legacy(None, 1_500),
            Duration::from_millis(1_500)
        );
        let d = axl_proto::Duration {
            seconds: 1,
            nanos: 5,
        };
        assert_eq!(duration_or_legacy(Some(&d), 9_999), Duration::new(1, 5));
    }
}
