use std::{
    sync::mpsc::RecvError,
    thread::{self, JoinHandle},
    time::SystemTime,
};

use axl_proto::{
    Timestamp,
    build_event_stream::{BuildEvent, TestStatus, build_event::Payload, build_event_id::Id},
};

use tracing::{Level, span::EnteredSpan};

use super::super::stream::Subscriber;
use super::retry::SinkOutcome;

#[derive(Debug)]
pub struct Tracing {}

fn timestamp_or_now(timestamp: Option<&Timestamp>) -> i64 {
    timestamp.map_or_else(
        || {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
        },
        |t| t.seconds,
    )
}

/// OTel span status code (`otel.status_code`) for a Bazel test outcome.
///
/// The tracing→OTel layer maps this field's value through `str_to_status`:
/// `"ok"` → `Status::Ok`, `"error"` → `Status::Error`, anything else →
/// `Status::Unset`. Failing outcomes (`FAILED`/`TIMEOUT`/`FAILED_TO_BUILD`/
/// `REMOTE_FAILURE`/`TOOL_HALTED_BEFORE_TESTING`) become errors so failing tests
/// surface as error spans; `PASSED`/`FLAKY` are `ok` (flaky ultimately passed);
/// `NO_STATUS`/`INCOMPLETE` stay unset. Mirrors `bazel_results.axl`'s
/// `_FAILED_STATUSES`.
fn test_status_code(status: TestStatus) -> &'static str {
    match status {
        TestStatus::Passed | TestStatus::Flaky => "ok",
        TestStatus::Failed
        | TestStatus::Timeout
        | TestStatus::FailedToBuild
        | TestStatus::RemoteFailure
        | TestStatus::ToolHaltedBeforeTesting => "error",
        _ => "unset",
    }
}

impl Tracing {
    pub fn spawn(recv: Subscriber<BuildEvent>) -> JoinHandle<SinkOutcome> {
        let events_span = tracing::info_span!("events");
        thread::spawn(move || {
            let _events_guard = events_span.enter();
            let mut build_span: Option<EnteredSpan> = None;

            loop {
                let event = match recv.recv() {
                    Ok(e) => e,
                    Err(RecvError) => break,
                };

                let Some(id) = event.id.as_ref().and_then(|w| w.id.as_ref()) else {
                    continue;
                };
                let Some(payload) = event.payload else {
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
                        if action.start_time.is_some() && action.end_time.is_some() {
                            let start_time = timestamp_or_now(action.start_time.as_ref());
                            let end_time = timestamp_or_now(action.end_time.as_ref());

                            // Span is entered+dropped on the same line; the OTel layer
                            // honors otel.start_time/otel.end_time as timing overrides,
                            // so the exported span carries the action's real wall-clock
                            // window rather than this near-zero local duration.
                            let _action = tracing::info_span!(
                                "action",
                                otel.start_time = start_time,
                                otel.end_time = end_time,
                                otel.status_code = if action.success { "unset" } else { "error" },
                                label = ?id.label,
                                success = action.success,
                                mnemonic = action.r#type,
                                exit_code = action.exit_code,
                                command_line = ?action.command_line,
                                stdout = ?action.stdout,
                                stderr = ?action.stderr,
                                primary_output = ?action.primary_output,
                                action_metadata_logs = ?action.action_metadata_logs,
                                failure_detail = ?action.failure_detail,
                                strategy_details = ?action.strategy_details,
                            )
                            .entered();
                        } else {
                            tracing::event!(name: "action_completed", Level::INFO, label = ?id.label);
                        }
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
                        // The authoritative per-target test outcome. Unlike the
                        // TestRunner `action` span (which carries the spawn's
                        // exit code — 0 for a spawn that merely executed), this
                        // span carries the real BlazeTestStatus and sets the OTel
                        // span status to error when the test failed.
                        let status =
                            TestStatus::try_from(summary.overall_status).unwrap_or(TestStatus::NoStatus);
                        let start_time = timestamp_or_now(summary.first_start_time.as_ref());
                        let end_time = timestamp_or_now(summary.last_stop_time.as_ref());
                        let _test = tracing::info_span!(
                            "test",
                            otel.start_time = start_time,
                            otel.end_time = end_time,
                            otel.status_code = test_status_code(status),
                            label = id.label.as_str(),
                            status = status.as_str_name(),
                            success = matches!(status, TestStatus::Passed | TestStatus::Flaky),
                            run_count = summary.run_count,
                            shard_count = summary.shard_count,
                            attempt_count = summary.attempt_count,
                            total_run_count = summary.total_run_count,
                            cached = summary.total_num_cached,
                        )
                        .entered();
                    }
                    (Payload::TestResult(result), Id::TestResult(id)) => {
                        // Per shard/run/attempt outcome. TestSummary is the
                        // final target verdict; these are the individual attempts
                        // (a failing attempt that later passes is visible here as
                        // an error span while the summary reports FLAKY).
                        let status = TestStatus::try_from(result.status).unwrap_or(TestStatus::NoStatus);
                        let start_time = timestamp_or_now(result.test_attempt_start.as_ref());
                        let end_time = start_time
                            + result.test_attempt_duration.as_ref().map_or(0, |d| d.seconds);
                        let _test_attempt = tracing::info_span!(
                            "test_attempt",
                            otel.start_time = start_time,
                            otel.end_time = end_time,
                            otel.status_code = test_status_code(status),
                            label = id.label.as_str(),
                            status = status.as_str_name(),
                            cached = result.cached_locally,
                            run = id.run,
                            shard = id.shard,
                            attempt = id.attempt,
                        )
                        .entered();
                    }
                    // High-volume streaming events (progress stdout/stderr chunks
                    // and the NamedSetOfFiles fan-out) fire thousands of times per
                    // build; representing each in the trace would flood the
                    // exporter with no observability value. Everything else is
                    // recorded as a lightweight span-event keyed by BES event kind
                    // so no meaningful build event is silently dropped.
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

    #[test]
    fn failing_statuses_map_to_error() {
        for s in [
            TestStatus::Failed,
            TestStatus::Timeout,
            TestStatus::FailedToBuild,
            TestStatus::RemoteFailure,
            TestStatus::ToolHaltedBeforeTesting,
        ] {
            assert_eq!(test_status_code(s), "error", "{s:?} should be an error span");
        }
    }

    #[test]
    fn passing_and_flaky_map_to_ok() {
        assert_eq!(test_status_code(TestStatus::Passed), "ok");
        assert_eq!(test_status_code(TestStatus::Flaky), "ok");
    }

    #[test]
    fn indeterminate_statuses_stay_unset() {
        assert_eq!(test_status_code(TestStatus::NoStatus), "unset");
        assert_eq!(test_status_code(TestStatus::Incomplete), "unset");
    }
}
