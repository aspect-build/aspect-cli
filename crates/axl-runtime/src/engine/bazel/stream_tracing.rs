use std::{
    collections::HashMap,
    thread::{self, JoinHandle},
    time::SystemTime,
};

use axl_proto::{
    Timestamp,
    build_event_stream::{BuildEvent, build_event::Payload, build_event_id::Id},
};

use fibre::spmc::Receiver;
use tokio::task;
use tracing::{Level, Span, span::EnteredSpan};

use super::super::r#async::rt::AsyncRuntime;

#[derive(Debug)]
pub struct TracingEventStreamSink {}

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

impl TracingEventStreamSink {
    pub fn spawn(rt: AsyncRuntime, recv: Receiver<BuildEvent>) -> JoinHandle<()> {
        let span = tracing::info_span!("events");
        thread::spawn(move || {
            rt.block_on(async { TracingEventStreamSink::task_spawn(span, recv).await.await })
                .expect("failed to join")
        })
    }

    pub async fn task_spawn(span: Span, recv: Receiver<BuildEvent>) -> task::JoinHandle<()> {
        tokio::task::spawn(async move {
            let _guard = span.enter();
            let mut spans: HashMap<&str, EnteredSpan> = HashMap::new();
            loop {
                let event = recv.recv();
                if event.is_err() {
                    break;
                }
                let event = event.unwrap();

                let id = event.id.as_ref().unwrap().id.as_ref().unwrap();

                match (event.payload.unwrap(), id) {
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

                            let span = tracing::info_span!(
                                "action",
                                otel.start_time = start_time,
                                otel.end_time = end_time
                            )
                            .entered();
                            drop(span);
                        } else {
                            tracing::event!(name: "action_completed", Level::INFO, label = ?id.label);
                        }
                    }
                    (Payload::Started(s), Id::Started(_)) => {
                        assert!(
                            spans
                                .insert(
                                    "building",
                                    tracing::info_span!(
                                        "build_tool",
                                        version = ?s.build_tool_version,
                                        pid = ?s.server_pid,
                                        uuid = ?s.uuid,
                                        current_dir = ?s.working_directory,
                                        repo_root = ?s.workspace_directory,
                                    )
                                    .entered()
                                )
                                .is_none()
                        );
                    }
                    (Payload::Finished(_), Id::BuildFinished(_)) => {
                        spans
                            .remove("building")
                            .expect("build_finished should have been called after build_started")
                            .exit();
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
                };
            }
        })
    }
}
