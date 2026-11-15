//! BES → OpenTelemetry trace sink.
//!
//! Consumes the build event stream on its own thread and turns it into spans
//! and span events under the active `ctx.bazel.build` span:
//!
//! - `events` covers the stream, and `build_tool` the Bazel invocation
//!   (`Started` → `BuildFinished`). Both are ordinary `tracing` spans, timed
//!   by when the sink sees those events.
//! - `action`, `test` and `test_attempt` describe work Bazel already finished
//!   by the time its event arrives, so their times come from the event. They
//!   are built with the OpenTelemetry tracer directly
//!   ([`export_completed_span`]), because a `tracing` span can only be timed
//!   by when it is entered and dropped. Each is parented to the current
//!   `tracing` span through `OpenTelemetrySpanExt::context`, which needs the
//!   global tracer provider and the `tracing-opentelemetry` layer to share
//!   one `SdkTracerProvider` (`aspect-cli`'s `trace::init`).
//! - Everything else becomes a span event, except progress and
//!   `NamedSetOfFiles`, which are too frequent to be worth recording.

use std::{
    sync::{Arc, mpsc::RecvError},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

use axl_proto::{
    Timestamp,
    build_event_stream::{TestStatus, build_event::Payload, build_event_id::Id},
};
use opentelemetry::{
    Array, KeyValue, StringValue, Value, global,
    trace::{Span as _, Status, Tracer as _},
};
use tracing::{Level, span::EnteredSpan};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use crate::bes::BuildEventEnvelope;
use crate::fanout::{SinkOutcome, Subscriber};

#[derive(Debug)]
pub struct Tracing {}

/// Instrumentation scope of the spans this sink builds with the tracer.
const TRACER_NAME: &str = "aspect-cli.bes";

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

fn duration_or_legacy(duration: Option<&prost_types::Duration>, legacy_millis: i64) -> Duration {
    match duration {
        Some(d) => Duration::new(d.seconds.max(0) as u64, d.nanos.max(0) as u32),
        None => Duration::from_millis(legacy_millis.max(0) as u64),
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

/// Exports a span for work that already finished, with its real start and
/// end, as a child of the current `tracing` span.
fn export_completed_span(
    name: &'static str,
    start: SystemTime,
    end: SystemTime,
    status: Status,
    attributes: Vec<KeyValue>,
) {
    let parent = tracing::Span::current().context();
    let tracer = global::tracer(TRACER_NAME);
    let mut span = tracer
        .span_builder(name)
        .with_start_time(start)
        .with_attributes(attributes)
        .start_with_context(&tracer, &parent);
    span.set_status(status);
    span.end_with_timestamp(end.max(start));
}

impl Tracing {
    pub fn spawn(recv: Subscriber<Arc<BuildEventEnvelope>>) -> JoinHandle<SinkOutcome> {
        let events_span = tracing::info_span!("events");
        thread::spawn(move || {
            let _events_guard = events_span.enter();
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
                        tracing::event!(name: "fetch", Level::INFO, url = ?id.url);
                    }
                    (Payload::OptionsParsed(opt), Id::OptionsParsed(_)) => {
                        tracing::event!(
                            name: "options_parsed",
                            Level::INFO,
                            build_tool = opt.tool_tag,
                            command_line = ?opt.cmd_line
                        );
                    }
                    (Payload::Action(action), Id::ActionCompleted(id)) => {
                        let (Some(start), Some(end)) =
                            (action.start_time.as_ref(), action.end_time.as_ref())
                        else {
                            tracing::event!(name: "action_completed", Level::INFO, label = id.label);
                            continue;
                        };
                        let mut attributes = vec![
                            KeyValue::new("label", id.label.clone()),
                            KeyValue::new("success", action.success),
                            KeyValue::new("mnemonic", action.r#type.clone()),
                            KeyValue::new("exit_code", i64::from(action.exit_code)),
                            KeyValue::new("command_line", string_array(&action.command_line)),
                        ];
                        let debug_fields = [
                            ("stdout", action.stdout.as_ref().map(|f| format!("{f:?}"))),
                            ("stderr", action.stderr.as_ref().map(|f| format!("{f:?}"))),
                            (
                                "primary_output",
                                action.primary_output.as_ref().map(|f| format!("{f:?}")),
                            ),
                            (
                                "failure_detail",
                                action.failure_detail.as_ref().map(|f| format!("{f:?}")),
                            ),
                            (
                                "action_metadata_logs",
                                (!action.action_metadata_logs.is_empty())
                                    .then(|| format!("{:?}", action.action_metadata_logs)),
                            ),
                            (
                                "strategy_details",
                                (!action.strategy_details.is_empty())
                                    .then(|| format!("{:?}", action.strategy_details)),
                            ),
                        ];
                        attributes.extend(
                            debug_fields
                                .into_iter()
                                .filter_map(|(k, v)| v.map(|v| KeyValue::new(k, v))),
                        );
                        let status = if action.success {
                            Status::Unset
                        } else {
                            Status::error(format!("exit code {}", action.exit_code))
                        };
                        export_completed_span(
                            "action",
                            system_time(start),
                            system_time(end),
                            status,
                            attributes,
                        );
                    }
                    (Payload::Started(s), Id::Started(_)) => {
                        if build_span.is_some() {
                            tracing::warn!("ignoring duplicate Started event");
                            continue;
                        }
                        build_span = Some(
                            tracing::info_span!(
                                "build_tool",
                                version = ?s.build_tool_version,
                                pid = ?s.server_pid,
                                uuid = ?s.uuid,
                                current_dir = ?s.working_directory,
                                root_dir = ?s.workspace_directory,
                            )
                            .entered(),
                        );
                    }
                    (Payload::Finished(_), Id::BuildFinished(_)) => {
                        if let Some(span) = build_span.take() {
                            span.exit();
                        } else {
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
                        // The authoritative per-target outcome. The TestRunner
                        // `action` span carries the spawn's exit code instead.
                        let status = TestStatus::try_from(summary.overall_status)
                            .unwrap_or(TestStatus::NoStatus);
                        #[allow(deprecated)]
                        let start = time_or_legacy(
                            summary.first_start_time.as_ref(),
                            summary.first_start_time_millis,
                        );
                        #[allow(deprecated)]
                        let end = time_or_legacy(
                            summary.last_stop_time.as_ref(),
                            summary.last_stop_time_millis,
                        );
                        export_completed_span(
                            "test",
                            start,
                            end,
                            test_status(status),
                            vec![
                                KeyValue::new("label", id.label.clone()),
                                KeyValue::new("status", status.as_str_name()),
                                KeyValue::new(
                                    "success",
                                    matches!(status, TestStatus::Passed | TestStatus::Flaky),
                                ),
                                KeyValue::new("run_count", i64::from(summary.run_count)),
                                KeyValue::new("shard_count", i64::from(summary.shard_count)),
                                KeyValue::new("attempt_count", i64::from(summary.attempt_count)),
                                KeyValue::new(
                                    "total_run_count",
                                    i64::from(summary.total_run_count),
                                ),
                                KeyValue::new("cached", i64::from(summary.total_num_cached)),
                            ],
                        );
                    }
                    (Payload::TestResult(result), Id::TestResult(id)) => {
                        // One shard/run/attempt. A failing attempt that later
                        // passes is an error span here while its `test` is ok.
                        let status =
                            TestStatus::try_from(result.status).unwrap_or(TestStatus::NoStatus);
                        #[allow(deprecated)]
                        let start = time_or_legacy(
                            result.test_attempt_start.as_ref(),
                            result.test_attempt_start_millis_epoch,
                        );
                        #[allow(deprecated)]
                        let duration = duration_or_legacy(
                            result.test_attempt_duration.as_ref(),
                            result.test_attempt_duration_millis,
                        );
                        export_completed_span(
                            "test_attempt",
                            start,
                            start + duration,
                            test_status(status),
                            vec![
                                KeyValue::new("label", id.label.clone()),
                                KeyValue::new("status", status.as_str_name()),
                                KeyValue::new("cached", result.cached_locally),
                                KeyValue::new("run", i64::from(id.run)),
                                KeyValue::new("shard", i64::from(id.shard)),
                                KeyValue::new("attempt", i64::from(id.attempt)),
                            ],
                        );
                    }
                    (Payload::Progress(_), _) | (Payload::NamedSetOfFiles(_), _) => {}
                    (_, id) => {
                        tracing::event!(name: "bes_event", Level::INFO, kind = id.as_str_name());
                    }
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{SpanKind, TracerProvider as _};
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::layer::SubscriberExt as _;

    #[test]
    fn failing_statuses_map_to_error() {
        for s in [
            TestStatus::Failed,
            TestStatus::Timeout,
            TestStatus::FailedToBuild,
            TestStatus::RemoteFailure,
            TestStatus::ToolHaltedBeforeTesting,
        ] {
            assert!(
                matches!(test_status(s), Status::Error { .. }),
                "{s:?} should be an error span"
            );
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
        let ts = Timestamp {
            seconds: 42,
            nanos: 123_456_789,
        };
        assert_eq!(
            time_or_legacy(Some(&ts), 999_000),
            SystemTime::UNIX_EPOCH + Duration::new(42, 123_456_789)
        );
    }

    #[test]
    fn timestamp_falls_back_to_legacy_millis() {
        assert_eq!(
            time_or_legacy(None, 1_723_000_000_250),
            SystemTime::UNIX_EPOCH + Duration::from_millis(1_723_000_000_250)
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
        let d = prost_types::Duration {
            seconds: 1,
            nanos: 5,
        };
        assert_eq!(duration_or_legacy(Some(&d), 9_999), Duration::new(1, 5));
    }

    /// The span keeps the event's sub-second times and nests under the
    /// `tracing` span that was current when it was exported.
    #[test]
    fn completed_span_keeps_times_and_parent() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        global::set_tracer_provider(provider.clone());
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));

        let start = SystemTime::UNIX_EPOCH + Duration::new(1_700_000_000, 250_000_000);
        let end = start + Duration::from_millis(1_234);
        tracing::subscriber::with_default(subscriber, || {
            let _parent = tracing::info_span!("build_tool").entered();
            export_completed_span(
                "test_attempt",
                start,
                end,
                Status::Ok,
                vec![KeyValue::new("label", "//:t")],
            );
        });
        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        let child = spans.iter().find(|s| s.name == "test_attempt").unwrap();
        let parent = spans.iter().find(|s| s.name == "build_tool").unwrap();
        assert_eq!(child.start_time, start);
        assert_eq!(child.end_time, end);
        assert_eq!(child.span_kind, SpanKind::Internal);
        assert_eq!(child.status, Status::Ok);
        assert_eq!(child.parent_span_id, parent.span_context.span_id());
        assert_eq!(
            child.span_context.trace_id(),
            parent.span_context.trace_id()
        );
    }
}
