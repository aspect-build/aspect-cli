impl super::build_event_stream::build_event_id::Id {
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Self::Unknown(_) => "unknown",
            Self::Progress(_) => "progress",
            Self::Started(_) => "build_started",
            Self::BuildFinished(_) => "build_finished",
            Self::UnstructuredCommandLine(_) => "unstructured_command_line",
            Self::StructuredCommandLine(_) => "structured_command_line",
            Self::WorkspaceStatus(_) => "workspace_status",
            Self::OptionsParsed(_) => "options_parsed",
            Self::Fetch(_) => "fetch",
            Self::Configuration(_) => "configuration",
            Self::TargetConfigured(_) => "target_configured",
            Self::Pattern(_) => "pattern_expanded",
            Self::PatternSkipped(_) => "pattern_skipped",
            Self::NamedSet(_) => "named_set_of_files",
            Self::TargetCompleted(_) => "target_completed",
            Self::ActionCompleted(_) => "action_completed",
            Self::UnconfiguredLabel(_) => "unconfigured_label",
            Self::ConfiguredLabel(_) => "configured_label",
            Self::TestResult(_) => "test_result",
            Self::TestProgress(_) => "test_progress",
            Self::TestSummary(_) => "test_summary",
            Self::TargetSummary(_) => "target_summary",
            Self::BuildToolLogs(_) => "build_tool_logs",
            Self::BuildMetrics(_) => "build_metrics",
            Self::Workspace(_) => "workspace_config",
            Self::BuildMetadata(_) => "build_metadata",
            Self::ConvenienceSymlinksIdentified(_) => "convenience_symlinks_identified",
            Self::ExecRequest(_) => "exec_request",
        }
    }
}
