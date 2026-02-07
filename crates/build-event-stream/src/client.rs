use std::collections::HashMap;

use axl_proto::google::devtools::build::v1::{
    PublishBuildToolEventStreamRequest, PublishBuildToolEventStreamResponse,
    PublishLifecycleEventRequest, publish_build_event_client::PublishBuildEventClient,
};
use futures::Stream;
use http::uri::InvalidUri;
use tonic::{
    Request, Response, Streaming,
    service::interceptor::InterceptedService,
    transport::{Channel, ClientTlsConfig},
};

use crate::auth::AuthInterceptor;

pub struct Client {
    inner: PublishBuildEventClient<InterceptedService<Channel, AuthInterceptor>>,
}

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error(transparent)]
    InvalidEndpoint(#[from] InvalidUri),
    #[error(transparent)]
    Transport(#[from] tonic::transport::Error),
    #[error(transparent)]
    Status(#[from] tonic::Status),
}

impl Client {
    pub async fn new(
        endpoint: String,
        headers: HashMap<String, String>,
    ) -> Result<Self, ClientError> {
        let channel = Channel::from_shared(endpoint)?
            .user_agent("AXL")?
            .tls_config(
                ClientTlsConfig::new()
                    .with_native_roots()
                    .with_enabled_roots(),
            )?
            .connect_lazy();
        let interceptor = AuthInterceptor::new(headers);
        let inner = PublishBuildEventClient::with_interceptor(channel, interceptor);
        Ok(Self { inner })
    }

    pub async fn publish_lifecycle_event(
        &mut self,
        event: PublishLifecycleEventRequest,
    ) -> Result<Response<()>, ClientError> {
        let ev = self
            .inner
            .publish_lifecycle_event(Request::new(event))
            .await?;
        Ok(ev)
    }

    pub async fn publish_build_tool_event_stream<
        S: Stream<Item = PublishBuildToolEventStreamRequest> + Send + 'static,
    >(
        &mut self,
        events: S,
    ) -> Result<Response<Streaming<PublishBuildToolEventStreamResponse>>, ClientError> {
        let x = self
            .inner
            .publish_build_tool_event_stream(Request::new(events))
            .await?;
        Ok(x)
    }
}

#[cfg(test)]
mod tests {

    use std::time::SystemTime;

    use axl_proto::{
        build_event_stream::{
            BuildEvent, BuildEventId, BuildFinished, BuildMetadata, BuildStarted, OptionsParsed,
            Progress, UnstructuredCommandLine, WorkspaceConfig,
            build_event::Payload,
            build_event_id::{
                BuildFinishedId, BuildMetadataId, BuildStartedId, Id, OptionsParsedId, ProgressId,
                StructuredCommandLineId, UnstructuredCommandLineId, WorkspaceConfigId,
            },
            build_finished::ExitCode,
        },
        command_line::{
            ChunkList, CommandLine, CommandLineSection, Option as BazelOption, OptionList,
            command_line_section::SectionType,
        },
        google::devtools::build::v1::BuildStatus,
    };
    use prost_types::Timestamp;
    use tokio_stream::{self, StreamExt};

    use crate::{build_tool, lifecycle};

    use super::*;

    #[tokio::test]
    async fn connect_test() -> Result<(), ClientError> {
        let mut client = Client::new(
            "https://testing.io".to_string(),
            HashMap::from_iter(vec![(
                "x-api-key".to_string(),
                "your_key_goes_here".to_string(),
            )]),
        )
        .await?;

        let uuid = uuid::Uuid::new_v4().to_string();
        let build_id = uuid.to_string();
        let invocation_id = uuid.to_string();

        // https://github.com/bazelbuild/bazel/blob/198c4c8aae1b5ef3d202f602932a99ce19707fc4/src/main/java/com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java#L322-L341

        let result = client
            .publish_lifecycle_event(lifecycle::build_enqueued(
                build_id.to_string(),
                invocation_id.to_string(),
            ))
            .await?;
        eprintln!("{:?}", result);
        let result = client
            .publish_lifecycle_event(lifecycle::invocation_started(
                build_id.to_string(),
                invocation_id.to_string(),
            ))
            .await?;
        eprintln!("{:?}", result);

        let mut stream = client
            .publish_build_tool_event_stream(tokio_stream::iter(vec![
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    1,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::Started(BuildStartedId {})),
                        }),
                        children: vec![],
                        last_message: false,
                        payload: Some(Payload::Started(BuildStarted {
                            build_tool_version: "8.0.0".to_string(),
                            command: "build".to_string(),
                            options_description: "".to_string(),
                            server_pid: 1246,
                            start_time: Some(Timestamp::from(SystemTime::now())),
                            uuid: uuid.to_string(),
                            working_directory: "/tmp/work".to_string(),
                            workspace_directory: "/tmp/work".to_string(),
                            ..Default::default()
                        })),
                    },
                ),
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    2,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::BuildMetadata(BuildMetadataId {})),
                        }),
                        children: vec![],
                        last_message: false,
                        payload: Some(Payload::BuildMetadata(BuildMetadata {
                            metadata: HashMap::new(),
                        })),
                    },
                ),
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    3,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::UnstructuredCommandLine(UnstructuredCommandLineId {})),
                        }),
                        children: vec![],
                        last_message: false,
                        payload: Some(Payload::UnstructuredCommandLine(
                            UnstructuredCommandLine::default(),
                        )),
                    },
                ),
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    4,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::OptionsParsed(OptionsParsedId {})),
                        }),
                        children: vec![],
                        last_message: false,
                        payload: Some(Payload::OptionsParsed(OptionsParsed {
                            startup_options: vec![],
                            explicit_startup_options: vec![],
                            cmd_line: vec![
                                "--nobuild_runfile_links".to_string(),
                                "--enable_platform_specific_config".to_string(),
                                "--noexperimental_check_external_repository_files".to_string(),
                                "--experimental_fetch_all_coverage_outputs".to_string(),
                                "--heap_dump_on_oom".to_string(),
                            ],
                            ..Default::default()
                        })),
                    },
                ),
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    5,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::StructuredCommandLine(StructuredCommandLineId {
                                command_line_label: "build".to_string(),
                            })),
                        }),
                        children: vec![],
                        last_message: false,
                        payload: Some(Payload::StructuredCommandLine(CommandLine {
                            command_line_label: "original".to_string(),
                            sections: vec![
                                CommandLineSection {
                                    section_label: "executable".to_string(),
                                    section_type: Some(SectionType::ChunkList(ChunkList {
                                        chunk: vec!["bazel".to_string()],
                                    })),
                                },
                                CommandLineSection {
                                    section_label: "startup options".to_string(),
                                    section_type: Some(SectionType::OptionList(OptionList {
                                        option: vec![],
                                    })),
                                },
                                CommandLineSection {
                                    section_label: "command".to_string(),
                                    section_type: Some(SectionType::ChunkList(ChunkList {
                                        chunk: vec!["build".to_string()],
                                    })),
                                },
                                CommandLineSection {
                                    section_label: "command options".to_string(),
                                    section_type: Some(SectionType::OptionList(OptionList {
                                        option: vec![BazelOption {
                                            combined_form: "--foo=test".to_string(),
                                            effect_tags: vec![],
                                            metadata_tags: vec![],
                                            option_name: "foo".to_string(),
                                            option_value: "test".to_string(),
                                            source: ".bazelrc".to_string(),
                                        }],
                                    })),
                                },
                                CommandLineSection {
                                    section_label: "residual".to_string(),
                                    section_type: Some(SectionType::ChunkList(ChunkList {
                                        chunk: vec!["...".to_string(), "REDACTED".to_string()],
                                    })),
                                },
                            ],
                        })),
                    },
                ),
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    6,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::Workspace(WorkspaceConfigId {})),
                        }),
                        children: vec![],
                        last_message: false,
                        payload: Some(Payload::WorkspaceInfo(WorkspaceConfig {
                            local_exec_root: "/private/var/tmp/_bazel_thesayyn/417a0dd96fe4f36c47b911f3d33c4442/execroot/_main".to_string()
                        })),
                    },
                ),
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    7,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::Progress(ProgressId {
                                opaque_count: 0
                            })),
                        }),
                        children: vec![],
                        last_message: false,
                        payload: Some(Payload::Progress(Progress { stdout: "This output came from somewhere else.".to_string(), stderr: "".to_string() })),
                    },
                ),
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    8,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::Progress(ProgressId {
                                opaque_count: 0
                            })),
                        }),
                        children: vec![],
                        last_message: false,
                        payload: Some(Payload::Progress(Progress { stdout: "Build succesful!".to_string(), stderr: "".to_string() })),
                    },
                ),
                build_tool::bazel_event(
                    build_id.to_string(),
                    invocation_id.to_string(),
                    9,
                    &BuildEvent {
                        id: Some(BuildEventId {
                            id: Some(Id::BuildFinished(BuildFinishedId {})),
                        }),
                        children: vec![],
                        last_message: true,
                        payload: Some(Payload::Finished(BuildFinished {
                            exit_code: Some(ExitCode {
                                code: 0,
                                name: "normal".to_string(),
                            }),
                            failure_detail: None,
                            finish_time: Some(Timestamp::from(SystemTime::now())),
                            ..Default::default()
                        })),
                    },
                ),
            ]))
            .await?
            .into_inner();

        eprintln!("sent succesfully {:?}", stream);

        while let Some(event) = stream.next().await {
            match event {
                Ok(ev) => println!("{ev:?}"),
                Err(err) => panic!("{}", err),
            }
        }

        let result = client
            .publish_lifecycle_event(lifecycle::invocation_finished(
                build_id.to_string(),
                invocation_id.to_string(),
                BuildStatus {
                    result: 0,
                    final_invocation_id: build_id.to_string(),
                    build_tool_exit_code: Some(0),
                    error_message: String::new(),
                    details: None,
                },
            ))
            .await?;
        eprintln!("{:?}", result);

        let result = client
            .publish_lifecycle_event(lifecycle::build_finished(
                build_id.to_string(),
                invocation_id.to_string(),
            ))
            .await?;
        eprintln!("{:?}", result);

        Ok(())
    }
}
