use std::{
    sync::mpsc::RecvError,
    thread::{self, JoinHandle},
    time::SystemTime,
};

use axl_proto::{
    Timestamp,
    build_event_stream::{BuildEvent, build_event::Payload, build_event_id::Id},
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
                    _ => {}
                }
            }
            Ok(())
        })
    }
}
