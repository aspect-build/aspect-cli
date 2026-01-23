

`type` [StructuredCommandLineId](/lib/bazel/build/build_event/build_event_id/structured_command_line_id)

`type` [BuildToolLogsId](/lib/bazel/build/build_event/build_event_id/build_tool_logs_id)

`type` [WorkspaceStatusId](/lib/bazel/build/build_event/build_event_id/workspace_status_id)

`type` [TestProgressId](/lib/bazel/build/build_event/build_event_id/test_progress_id)

`type` [TestResultId](/lib/bazel/build/build_event/build_event_id/test_result_id)

`type` [OptionsParsedId](/lib/bazel/build/build_event/build_event_id/options_parsed_id)

`type` [BuildStartedId](/lib/bazel/build/build_event/build_event_id/build_started_id)

`type` [TargetSummaryId](/lib/bazel/build/build_event/build_event_id/target_summary_id)

`type` [TargetConfiguredId](/lib/bazel/build/build_event/build_event_id/target_configured_id)

`type` [ActionCompletedId](/lib/bazel/build/build_event/build_event_id/action_completed_id)

`type` [TestSummaryId](/lib/bazel/build/build_event/build_event_id/test_summary_id)

`type` [BuildMetricsId](/lib/bazel/build/build_event/build_event_id/build_metrics_id)

`type` [ExecRequestId](/lib/bazel/build/build_event/build_event_id/exec_request_id)

`type` [PatternExpandedId](/lib/bazel/build/build_event/build_event_id/pattern_expanded_id)

`type` [NamedSetOfFilesId](/lib/bazel/build/build_event/build_event_id/named_set_of_files_id)

`type` [BuildFinishedId](/lib/bazel/build/build_event/build_event_id/build_finished_id)

`type` [ConfiguredLabelId](/lib/bazel/build/build_event/build_event_id/configured_label_id)

`type` [ConvenienceSymlinksIdentifiedId](/lib/bazel/build/build_event/build_event_id/convenience_symlinks_identified_id)

`type` [BuildMetadataId](/lib/bazel/build/build_event/build_event_id/build_metadata_id)

`type` [UnstructuredCommandLineId](/lib/bazel/build/build_event/build_event_id/unstructured_command_line_id)

`type` [UnconfiguredLabelId](/lib/bazel/build/build_event/build_event_id/unconfigured_label_id)

`type` [ProgressId](/lib/bazel/build/build_event/build_event_id/progress_id)

`type` [FetchId](/lib/bazel/build/build_event/build_event_id/fetch_id)

`type` [TargetCompletedId](/lib/bazel/build/build_event/build_event_id/target_completed_id)

`type` [ConfigurationId](/lib/bazel/build/build_event/build_event_id/configuration_id)

`type` [WorkspaceConfigId](/lib/bazel/build/build_event/build_event_id/workspace_config_id)

`type` [UnknownBuildEventId](/lib/bazel/build/build_event/build_event_id/unknown_build_event_id)

`property` **BuildEventId.id**

<pre class="language-python"><code><span class="source python"><span class="meta qualified-name python"><span class="meta generic-name python">BuildEventId</span><span class="punctuation accessor dot python">.</span><span class="meta generic-name python">id</span></span><span class="punctuation separator annotation variable python">:</span> <a href="/lib">None</a> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">action_completed_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">build_finished_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">build_metadata_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">build_metrics_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">build_started_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">build_tool_logs_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">configuration_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">configured_label_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">convenience_symlinks_identified_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">exec_request_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">fetch_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">named_set_of_files_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">options_parsed_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">pattern_expanded_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">progress_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">structured_command_line_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">target_completed_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">target_configured_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">target_summary_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">test_progress_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">test_result_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">test_summary_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">unconfigured_label_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">unknown_build_event_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">unstructured_command_line_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">workspace_config_id</span></span> <span class="keyword operator arithmetic python">|</span> <span class="meta qualified-name python"><span class="meta generic-name python">workspace_status_id</span></span></span></code></pre>
