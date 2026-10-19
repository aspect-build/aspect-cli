//! The Build Event Protocol of a build: Bazel's own `BuildEventStreamer` on the
//! engine's event bus, feeding Bazel's own transports (`BinaryFormatFileTransport`
//! and `BuildEventServiceTransport` over its gRPC client). What a `bazel build`
//! command posts around the build and buildx bypasses (the started event, the
//! command lines, the options, the workspace status, the finish, the metrics and
//! the tool logs) is posted here as `BuildEvent`s written in Rust.

use super::*;
use crate::events::{Bes, Invocation, Sink};
use axl_proto::build_event_stream as pb;
use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::build_event_stream::build_event_id::Id;
use axl_proto::command_line as cl;
use blazon_rt::Result as RtResult;
use prost::Message;

/// A build event whose protocol buffer is known up front.
struct Synth {
    id: pb::BuildEventId,
    children: Vec<pb::BuildEventId>,
    payload: Payload,
}

fn id(i: Id) -> pb::BuildEventId {
    pb::BuildEventId { id: Some(i) }
}

fn java_id<'a>(env: &mut JNIEnv<'a>, i: &pb::BuildEventId) -> RtResult<bazel::BuildEventId<'a>> {
    bazel::BuildEventId::parse_from(env, &i.encode_to_vec())
}

impl bazel::BuildEventImpl for Synth {
    fn as_stream_proto<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        _context: bazel::BuildEventContext<'a>,
    ) -> RtResult<bazel::StreamEvent<'a>> {
        let event = pb::BuildEvent {
            id: Some(self.id.clone()),
            children: self.children.clone(),
            last_message: false,
            payload: Some(self.payload.clone()),
        };
        bazel::StreamEvent::parse_from(env, &event.encode_to_vec())
    }

    fn event_id<'a>(&self, env: &mut JNIEnv<'a>) -> RtResult<bazel::BuildEventId<'a>> {
        java_id(env, &self.id)
    }

    fn children_events<'a>(
        &self,
        env: &mut JNIEnv<'a>,
    ) -> RtResult<bazel::Collection<'a, bazel::BuildEventId<'static>>> {
        let set = bazel::List::<bazel::BuildEventId>::new(env)?;
        for c in &self.children {
            set.add(&java_id(env, c)?)?;
        }
        Ok(set.upcast())
    }
}

/// How a build ended, for the finish event.
pub(super) struct Outcome {
    pub exit: Exit,
    /// What went wrong first, for the failure detail.
    pub message: Option<String>,
    pub targets: usize,
    pub failed: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Exit {
    Success,
    BuildFailure,
    AnalysisFailure,
    PartialAnalysisFailure,
}

struct Transport {
    name: String,
    java: Global<bazel::BuildEventTransport<'static>>,
    client: Option<Global<bazel::BuildEventServiceGrpcClient<'static>>>,
    channel: Option<Global<bazel::ManagedChannel<'static>>>,
    /// Where this sink's results are, printed once it received everything.
    results: Option<String>,
}

/// A build's event stream while it runs.
pub(super) struct Bep {
    streamer: Global<bazel::BuildEventStreamer<'static>>,
    event_bus: Global<bazel::EventBus<'static>>,
    transports: Vec<Transport>,
    invocation: String,
    started: std::time::Instant,
    start_ms: i64,
    exec_root: String,
    workspace: String,
    patterns: Vec<String>,
    uploads: Option<Arc<artifacts::Report>>,
    options: Vec<Opt>,
    cpu_start: i64,
    metrics: Mutex<metrics::Collected>,
    /// Whether the workspace status action ran in this build.
    status: super::status::Ran,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn timestamp(ms: i64) -> axl_proto::Timestamp {
    axl_proto::Timestamp {
        seconds: ms.div_euclid(1000),
        nanos: (ms.rem_euclid(1000) * 1_000_000) as i32,
    }
}

/// One option of the command line the stream states.
///
/// buildx has no flags: these are not accepted anywhere. They state what the
/// build did in the words a Bazel UI reads (`--remote_cache`, `--bes_backend`,
/// the config settings), because BES consumers infer how a build ran from its
/// command line.
struct Opt {
    /// `combined_form` of the structured option.
    combined: String,
    name: String,
    value: String,
    /// `OptionEffectTag` and `OptionMetadataTag` numbers, empty for an option the
    /// generated schema does not know.
    effects: Vec<i32>,
    metadata: Vec<i32>,
}

/// What a header's value is shown as: Bazel's own command line events have no
/// redaction of options (a `--remote_header` goes into its BEP as written, a
/// `run` residue is shown as this word), so its word is borrowed and the value
/// is never shown at all.
const REDACTED: &str = "REDACTED";

fn effect_number(name: &str) -> Option<i32> {
    Some(match name {
        "UNKNOWN" => 0,
        "NO_OP" => 1,
        "LOSES_INCREMENTAL_STATE" => 2,
        "CHANGES_INPUTS" => 3,
        "AFFECTS_OUTPUTS" => 4,
        "BUILD_FILE_SEMANTICS" => 5,
        "BAZEL_INTERNAL_CONFIGURATION" => 6,
        "LOADING_AND_ANALYSIS" => 7,
        "EXECUTION" => 8,
        "HOST_MACHINE_RESOURCE_OPTIMIZATIONS" => 9,
        "EAGERNESS_TO_EXIT" => 10,
        "BAZEL_MONITORING" => 11,
        "TERMINAL_OUTPUT" => 12,
        "ACTION_COMMAND_LINES" => 13,
        "TEST_RUNNER" => 14,
        _ => return None,
    })
}

fn metadata_number(name: &str) -> Option<i32> {
    Some(match name {
        "EXPERIMENTAL" => 0,
        "INCOMPATIBLE_CHANGE" => 1,
        "DEPRECATED" => 2,
        "HIDDEN" => 3,
        "INTERNAL" => 4,
        "NON_CONFIGURABLE" => 8,
        _ => return None,
    })
}

/// A config setting `name=value`, with the tags the generated schema gives it.
fn setting_opt(setting: &str) -> Opt {
    let (name, value) = split(setting);
    let (effects, metadata) = match crate::settings::lookup(name) {
        Ok((def, _)) => (
            def.effect_tags
                .iter()
                .filter_map(|t| effect_number(t))
                .collect(),
            def.metadata_tags
                .iter()
                .filter_map(|t| metadata_number(t))
                .collect(),
        ),
        Err(_) => (Vec::new(), Vec::new()),
    };
    Opt {
        combined: format!("{name}={value}"),
        name: name.to_string(),
        value: value.to_string(),
        effects,
        metadata,
    }
}

/// An option Bazel spells `--name=value`.
fn flag_opt(name: &str, value: &str) -> Opt {
    Opt {
        combined: format!("--{name}={value}"),
        name: name.to_string(),
        value: value.to_string(),
        effects: Vec::new(),
        metadata: Vec::new(),
    }
}

/// The options the executors of the build imply.
///
/// Every remote service the executors can reach is stated, including every
/// executor in a `route`'s `executors` dict whether or not a spawn went there.
/// `cached` looks results up and stores local ones, so it states
/// `remote_accept_cached` and `remote_upload_local_results`. Outputs of remote
/// actions and cache hits are downloaded as the action finishes, which is
/// `remote_download_outputs=all`.
fn remote_options(used: &crate::events::Used) -> Vec<Opt> {
    let mut out = Vec::new();
    let mut instances: Vec<String> = Vec::new();
    let mut instance = |out: &mut Vec<Opt>, r: &crate::events::Remote| {
        if !r.instance.is_empty() && !instances.contains(&r.instance) {
            instances.push(r.instance.clone());
            out.push(flag_opt("remote_instance_name", &r.instance));
        }
    };
    for r in &used.caches {
        out.push(flag_opt("remote_cache", &r.endpoint));
        instance(&mut out, r);
    }
    for r in &used.executors {
        out.push(flag_opt("remote_executor", &r.endpoint));
        instance(&mut out, r);
    }
    if !used.caches.is_empty() {
        out.push(flag_opt("remote_accept_cached", "true"));
        out.push(flag_opt("remote_upload_local_results", "true"));
    }
    for r in &used.caches {
        for h in &r.headers {
            out.push(flag_opt("remote_cache_header", &format!("{h}={REDACTED}")));
        }
    }
    for r in &used.executors {
        for h in &r.headers {
            out.push(flag_opt("remote_exec_header", &format!("{h}={REDACTED}")));
        }
    }
    if !used.caches.is_empty() || !used.executors.is_empty() {
        out.push(flag_opt("remote_download_outputs", "all"));
    }
    out
}

/// The options the event sinks imply: `bes_backend` and friends for a BES sink,
/// `build_metadata` per metadata entry, `build_event_binary_file` for a file.
fn sink_options(sinks: &[Arc<Sink>]) -> Vec<Opt> {
    let mut out = Vec::new();
    for sink in sinks {
        match &**sink {
            Sink::File(path) => {
                out.push(flag_opt("build_event_binary_file", &path.to_string_lossy()))
            }
            Sink::Bes(b) => {
                out.push(flag_opt("bes_backend", &b.endpoint));
                if !b.results_url.is_empty() {
                    out.push(flag_opt("bes_results_url", &b.results_url));
                }
                if !b.instance.is_empty() {
                    out.push(flag_opt("bes_instance_name", &b.instance));
                }
                for (k, v) in &b.metadata {
                    out.push(flag_opt("build_metadata", &format!("{k}={v}")));
                }
                for (name, _) in &b.headers {
                    out.push(flag_opt("bes_header", &format!("{name}={REDACTED}")));
                }
            }
        }
    }
    out
}

fn proto_option(o: &Opt) -> cl::Option {
    cl::Option {
        combined_form: o.combined.clone(),
        option_name: o.name.clone(),
        option_value: o.value.clone(),
        effect_tags: o.effects.clone(),
        metadata_tags: o.metadata.clone(),
    }
}

/// `--name=value`, the unstructured and `options_parsed` spelling of an option.
fn flag(o: &Opt) -> String {
    format!("--{}={}", o.name, o.value)
}

fn chunks(label: &str, items: &[String]) -> cl::CommandLineSection {
    cl::CommandLineSection {
        section_label: label.to_string(),
        section_type: Some(cl::command_line_section::SectionType::ChunkList(
            cl::ChunkList {
                chunk: items.to_vec(),
            },
        )),
    }
}

fn options(label: &str, opts: Vec<cl::Option>) -> cl::CommandLineSection {
    cl::CommandLineSection {
        section_label: label.to_string(),
        section_type: Some(cl::command_line_section::SectionType::OptionList(
            cl::OptionList { option: opts },
        )),
    }
}

fn split(setting: &str) -> (&str, &str) {
    setting.split_once('=').unwrap_or((setting, ""))
}

fn host_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is valid for `buf.len()` bytes and gethostname terminates what it writes.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    if !ok {
        return String::new();
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
}

fn failure_detail(message: &str) -> Vec<u8> {
    axl_proto::failure_details::FailureDetail {
        message: message.to_string(),
        ..Default::default()
    }
    .encode_to_vec()
}

fn grpc_target(endpoint: &str) -> (String, bool) {
    match endpoint.strip_prefix("grpcs://") {
        Some(rest) => (rest.to_string(), true),
        None => (
            endpoint
                .strip_prefix("grpc://")
                .unwrap_or(endpoint)
                .to_string(),
            false,
        ),
    }
}

/// The option classes the stream's defaults come from, parsed with no arguments.
fn defaults<'l>(env: &mut JNIEnv<'l>) -> Result<bazel::OptionsParser<'l>> {
    let classes = bazel::Set::<bazel::Class<OptionsBase>>::new(env)?;
    for name in [
        "com/google/devtools/build/lib/buildeventstream/transports/BuildEventStreamOptions",
        "com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions",
        "com/google/devtools/build/lib/buildeventservice/BuildEventServiceOptions",
    ] {
        classes.add(&class::<OptionsBase>(env, name)?)?;
    }
    let parser = bazel::OptionsParser::builder(env)?
        .options_classes(&classes)?
        .build()?;
    parser.parse(&bazel::List::of_strs(env, &[] as &[&str])?)?;
    Ok(parser)
}

impl Bep {
    /// Builds the transports for `sinks`, a streamer over them, registers it on
    /// the event bus and posts the events that open the stream. A sink that
    /// cannot be set up is reported on stderr and left out; with none left there
    /// is no stream.
    pub(super) fn start(
        env: &mut JNIEnv<'_>,
        engine: &Engine,
        sinks: &[Arc<Sink>],
        invocation: &Invocation,
        status: &super::status::Ran,
    ) -> Result<Option<Bep>> {
        let l = &engine.loading;
        let runtime = engine.runtime.get(env)?;
        let started = std::time::Instant::now();
        let start_ms = now_ms();
        let parser = defaults(env)?;
        let bep_options = parser.get_options(&class::<bazel::BuildEventProtocolOptions>(
            env,
            "com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions",
        )?)?;
        let stream_options = parser.get_options(&class::<bazel::BuildEventStreamOptions>(
            env,
            "com/google/devtools/build/lib/buildeventstream/transports/BuildEventStreamOptions",
        )?)?;
        let bes_options = parser.get_options(&class::<bazel::BuildEventServiceOptions>(
            env,
            "com/google/devtools/build/lib/buildeventservice/BuildEventServiceOptions",
        )?)?;
        let namer = bazel::CountingArtifactGroupNamer::new(env)?;
        let invocation_id = bazel::UUID::random(env)?;
        let invocation_text = invocation_id.to_string();
        let event_bus = l.event_bus.get(env)?;
        let mut transports = Vec::new();
        let report = Arc::new(artifacts::Report::default());
        let mut uploads = false;
        let mut metadata: Vec<(String, String)> = Vec::new();
        for sink in sinks {
            let made = match &**sink {
                Sink::File(path) => {
                    file_transport(env, &runtime, path, &bep_options, &namer, start_ms)
                }
                Sink::Bes(b) => bes_transport(
                    env,
                    &runtime,
                    b,
                    &bes_options,
                    &bep_options,
                    &namer,
                    &event_bus,
                    &invocation_id,
                    start_ms,
                    &report,
                ),
            };
            match made {
                Ok(t) => {
                    if let Sink::Bes(b) = &**sink {
                        metadata.extend(b.metadata.iter().cloned());
                        uploads |= b.artifacts.is_some();
                    }
                    transports.push(t);
                }
                Err(e) => eprintln!("buildx: events: {}", one_line(&format!("{e:#}"))),
            }
        }
        if transports.is_empty() {
            return Ok(None);
        }
        let set = bazel::Set::<bazel::BuildEventTransport>::new(env)?;
        for t in &transports {
            set.add(&t.java.get(env)?)?;
        }
        let streamer = bazel::BuildEventStreamerBuilder::new(env)?
            .build_event_transports(&set)?
            .bes_stream_options(&stream_options)?
            .output_group_file_modes(&bep_options.output_group_file_modes_mapping()?)?
            .publish_target_summaries(false)?
            .artifact_group_namer(&namer)?
            .build()?;
        if uploads {
            let progress = bazel::OutErrProvider::from_impl(
                env,
                Box::new(artifacts::Progress(report.clone())),
            )?;
            streamer.register_out_err_provider(&progress)?;
        }
        event_bus.register(&streamer)?;
        let exec_root = runtime
            .workspace()?
            .directories()?
            .get_exec_root(&runtime.rule_class_provider()?.runfiles_prefix()?)?
            .path_string()?;
        let bep = Bep {
            streamer: streamer.to_global()?,
            event_bus: event_bus.to_global()?,
            transports,
            invocation: invocation_text,
            started,
            start_ms,
            exec_root,
            workspace: runtime.workspace()?.workspace()?.path_string()?,
            patterns: invocation.targets.clone(),
            uploads: uploads.then_some(report),
            cpu_start: metrics::cpu_ms(),
            metrics: Mutex::new(metrics::Collected::default()),
            status: status.clone(),
            options: invocation
                .settings
                .iter()
                .map(|s| setting_opt(s))
                .chain(remote_options(&invocation.used))
                .chain(sink_options(sinks))
                .collect(),
        };
        let _phase = crate::phases::start("events: set up and open stream");
        bep.open(env, invocation, &metadata)?;
        Ok(Some(bep))
    }

    /// The `workspace_status` event of a build in which the workspace status
    /// action did not run (nothing was built, so Bazel's own event was never
    /// posted): the keys every status carries.
    fn workspace_status_fallback(&self, env: &mut JNIEnv<'_>, empty: bool) -> Result<()> {
        let item = |k: &str, v: String| pb::workspace_status::Item {
            key: k.to_string(),
            value: v,
        };
        self.post(
            env,
            Synth {
                id: id(Id::WorkspaceStatus(Default::default())),
                children: Vec::new(),
                payload: Payload::WorkspaceStatus(pb::WorkspaceStatus {
                    item: if empty {
                        Vec::new()
                    } else {
                        vec![
                            item("BUILD_HOST", host_name()),
                            item("BUILD_USER", user_name()),
                            item("BUILD_TIMESTAMP", (self.start_ms / 1000).to_string()),
                        ]
                    },
                }),
            },
        )
    }

    /// Fills in what the metrics event states.
    pub(super) fn collect(&self, fill: impl FnOnce(&mut metrics::Collected)) {
        if let Ok(mut m) = self.metrics.lock() {
            fill(&mut m);
        }
    }

    pub(super) fn elapsed_ms(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
    }

    fn post(&self, env: &mut JNIEnv<'_>, event: Synth) -> Result<()> {
        let event = bazel::BuildEvent::from_impl(env, Box::new(event))?;
        self.event_bus.get(env)?.post(&event)?;
        Ok(())
    }

    /// The patterns expanded: the event that announces each target's configuration.
    pub(super) fn pattern(&self, env: &mut JNIEnv<'_>, labels: Vec<String>) -> Result<()> {
        let children = labels
            .into_iter()
            .map(|label| {
                id(Id::TargetConfigured(
                    pb::build_event_id::TargetConfiguredId {
                        label,
                        aspect: String::new(),
                    },
                ))
            })
            .collect();
        self.post(
            env,
            Synth {
                id: id(Id::Pattern(pb::build_event_id::PatternExpandedId {
                    pattern: self.patterns.clone(),
                })),
                children,
                payload: Payload::Expanded(pb::PatternExpanded::default()),
            },
        )
    }

    fn open(
        &self,
        env: &mut JNIEnv<'_>,
        invocation: &Invocation,
        metadata: &[(String, String)],
    ) -> Result<()> {
        let workspace = self.workspace.clone();
        let label = |s: &str| {
            id(Id::StructuredCommandLine(
                pb::build_event_id::StructuredCommandLineId {
                    command_line_label: s.to_string(),
                },
            ))
        };
        let pattern = id(Id::Pattern(pb::build_event_id::PatternExpandedId {
            pattern: invocation.targets.clone(),
        }));
        let children = vec![
            id(Id::Progress(pb::build_event_id::ProgressId {
                opaque_count: 0,
            })),
            id(Id::UnstructuredCommandLine(Default::default())),
            label("original"),
            label("canonical"),
            label("tool"),
            id(Id::BuildMetadata(Default::default())),
            id(Id::OptionsParsed(Default::default())),
            id(Id::WorkspaceStatus(Default::default())),
            pattern,
            id(Id::BuildFinished(Default::default())),
        ];
        let options_description = self.options.iter().map(flag).collect::<Vec<_>>().join(" ");
        self.post(
            env,
            Synth {
                id: id(Id::Started(Default::default())),
                children,
                payload: Payload::Started(pb::BuildStarted {
                    uuid: self.invocation.clone(),
                    #[allow(deprecated)]
                    start_time_millis: self.start_ms,
                    start_time: Some(timestamp(self.start_ms)),
                    build_tool_version: bazel::BAZEL_VERSION.to_string(),
                    options_description,
                    command: "build".to_string(),
                    working_directory: workspace.clone(),
                    workspace_directory: workspace.clone(),
                    server_pid: i64::from(std::process::id()),
                }),
            },
        )?;

        let mut args = vec![
            "build".to_string(),
            format!("--buildx_script={}", invocation.script),
        ];
        args.extend(self.options.iter().map(flag));
        args.extend(invocation.targets.iter().cloned());
        self.post(
            env,
            Synth {
                id: id(Id::UnstructuredCommandLine(Default::default())),
                children: Vec::new(),
                payload: Payload::UnstructuredCommandLine(pb::UnstructuredCommandLine { args }),
            },
        )?;

        let executable = vec!["buildx".to_string(), invocation.script.clone()];
        let command = vec!["build".to_string()];
        let setting_options = || self.options.iter().map(proto_option).collect::<Vec<_>>();
        for (name, sections) in [
            (
                "original",
                vec![
                    chunks("executable", &executable),
                    options("startup options", Vec::new()),
                    chunks("command", &command),
                    options("command options", setting_options()),
                    chunks("residual", &invocation.targets),
                ],
            ),
            (
                "canonical",
                vec![
                    chunks("executable", &executable),
                    options("startup options", Vec::new()),
                    chunks("command", &command),
                    options("command options", setting_options()),
                    chunks("residual", &invocation.targets),
                ],
            ),
            ("tool", Vec::new()),
        ] {
            self.post(
                env,
                Synth {
                    id: label(name),
                    children: Vec::new(),
                    payload: Payload::StructuredCommandLine(cl::CommandLine {
                        command_line_label: name.to_string(),
                        sections,
                    }),
                },
            )?;
        }

        let mut meta: std::collections::HashMap<String, String> =
            metadata.iter().cloned().collect();
        meta.insert("buildx".to_string(), "true".to_string());
        meta.entry("BUILD_TOOL".to_string())
            .or_insert_with(|| "buildx".to_string());
        self.post(
            env,
            Synth {
                id: id(Id::BuildMetadata(Default::default())),
                children: Vec::new(),
                payload: Payload::BuildMetadata(pb::BuildMetadata { metadata: meta }),
            },
        )?;

        let cmd_line: Vec<String> = self.options.iter().map(flag).collect();
        self.post(
            env,
            Synth {
                id: id(Id::OptionsParsed(Default::default())),
                children: Vec::new(),
                payload: Payload::OptionsParsed(pb::OptionsParsed {
                    cmd_line: cmd_line.clone(),
                    explicit_cmd_line: cmd_line,
                    ..Default::default()
                }),
            },
        )?;

        self.post(
            env,
            Synth {
                id: id(Id::Workspace(Default::default())),
                children: Vec::new(),
                payload: Payload::WorkspaceInfo(pb::WorkspaceConfig {
                    local_exec_root: self.exec_root.clone(),
                }),
            },
        )?;
        Ok(())
    }

    /// Posts the events that close the stream, then waits for every transport to
    /// finish. A transport that failed is reported on stderr; nothing here fails
    /// the build.
    pub(super) fn finish(self, env: &mut JNIEnv<'_>, outcome: &Outcome) {
        let _phase = crate::phases::start("events: finish stream and wait");
        if let Err(e) = self.close_stream(env, outcome) {
            eprintln!(
                "buildx: events: closing the event stream: {}",
                one_line(&format!("{e:#}"))
            );
        }
        let mut results = Vec::new();
        for t in &self.transports {
            match wait(env, t) {
                Ok(()) => results.extend(t.results.clone()),
                Err(e) => eprintln!(
                    "buildx: events: {}: {}",
                    t.name,
                    one_line(&format!("{e:#}"))
                ),
            }
        }
        if let Ok(bus) = self.event_bus.get(env)
            && let Ok(streamer) = self.streamer.get(env)
        {
            let _ = bus.unregister(&streamer);
        }
        for t in &self.transports {
            if let Some(c) = &t.client
                && let Ok(c) = c.get(env)
            {
                let _ = c.shutdown();
            }
            if let Some(c) = &t.channel
                && let Ok(c) = c.get(env)
            {
                let _ = c.shutdown_now();
            }
        }
        if let Some(summary) = self.uploads.as_ref().and_then(|r| r.summary()) {
            eprintln!("buildx: events: {summary}");
        }
        for url in &results {
            eprintln!("buildx: results: {url}");
        }
    }

    fn close_stream(&self, env: &mut JNIEnv<'_>, outcome: &Outcome) -> Result<()> {
        let now = now_ms();
        if !self.status.get() {
            self.workspace_status_fallback(env, false)?;
        } else if self.status.did_fail() {
            // Bazel posts an empty status for a build whose status action failed.
            self.workspace_status_fallback(env, true)?;
        }
        self.post(
            env,
            Synth {
                id: id(Id::ConvenienceSymlinksIdentified(Default::default())),
                children: Vec::new(),
                payload: Payload::ConvenienceSymlinksIdentified(Default::default()),
            },
        )?;
        let result = bazel::BuildResult::new(env, self.start_ms)?;
        result.set_stop_time(now)?;
        let (exit_code, detail) = match (outcome.exit, &outcome.message) {
            (Exit::Success, _) => (bazel::ExitCode::success(env)?, None),
            (Exit::BuildFailure, m) => (bazel::ExitCode::build_failure(env)?, m.as_deref()),
            (Exit::AnalysisFailure, m) => (bazel::ExitCode::analysis_failure(env)?, m.as_deref()),
            (Exit::PartialAnalysisFailure, m) => (
                bazel::ExitCode::partial_analysis_failure(env)?,
                m.as_deref(),
            ),
        };
        let detailed = match detail {
            None => bazel::DetailedExitCode::success(env)?,
            Some(m) => {
                let detail = bazel::FailureDetail::parse_from(env, &failure_detail(m))?;
                bazel::DetailedExitCode::of(env, &exit_code, &detail)?
            }
        };
        result.set_detailed_exit_code(&detailed)?;
        let children = bazel::List::<bazel::BuildEventId>::new(env)?;
        children.add(&java_id(env, &id(Id::BuildToolLogs(Default::default())))?)?;
        children.add(&java_id(env, &id(Id::BuildMetrics(Default::default())))?)?;
        let complete = bazel::BuildCompleteEvent::new(
            env,
            &result,
            &children.upcast::<bazel::Collection<_>>(),
        )?;
        self.event_bus.get(env)?.post(&complete)?;

        let elapsed = self.started.elapsed();
        let file = |name: &str, text: String| pb::File {
            name: name.to_string(),
            file: Some(pb::file::File::Contents(text.into_bytes())),
            ..Default::default()
        };
        self.post(
            env,
            Synth {
                id: id(Id::BuildToolLogs(Default::default())),
                children: Vec::new(),
                payload: Payload::BuildToolLogs(pb::BuildToolLogs {
                    log: vec![
                        file("elapsed time", format!("{:.6}", elapsed.as_secs_f64())),
                        file(
                            "targets",
                            format!("{} requested, {} failed", outcome.targets, outcome.failed),
                        ),
                    ],
                }),
            },
        )?;
        self.post(
            env,
            Synth {
                id: id(Id::BuildMetrics(Default::default())),
                children: Vec::new(),
                payload: Payload::BuildMetrics({
                    let mut m = self.metrics.lock().map(|m| m.clone()).unwrap_or_default();
                    m.cpu_ms = metrics::cpu_ms() - self.cpu_start;
                    m.proto(elapsed.as_millis() as i64)
                }),
            },
        )?;
        self.streamer.get(env)?.close()?;
        Ok(())
    }
}

/// The reason of a Java failure on one line: what Bazel's `AbruptExitException`
/// says when it is there, else the first line.
fn one_line(message: &str) -> String {
    let at = message
        .rfind("AbruptExitException: ")
        .map(|i| i + "AbruptExitException: ".len())
        .unwrap_or(0);
    let line = message[at..].lines().next().unwrap_or_default().trim();
    line.replace("UNAVAILABLE: UNAVAILABLE:", "UNAVAILABLE:")
        .replace("UNAVAILABLE: UNAVAILABLE", "UNAVAILABLE")
}

fn wait(env: &mut JNIEnv<'_>, t: &Transport) -> Result<()> {
    let future = t.java.get(env)?.close()?;
    let fluent = bazel::FluentFuture::from(env, &future)?;
    fluent
        .downcast::<bazel::AbstractFuture<bazel::Object>>()?
        .get()?;
    Ok(())
}

fn file_transport(
    env: &mut JNIEnv<'_>,
    runtime: &bazel::BlazeRuntime,
    path: &std::path::Path,
    bep_options: &bazel::BuildEventProtocolOptions,
    namer: &bazel::CountingArtifactGroupNamer,
    _start_ms: i64,
) -> Result<Transport> {
    // One uploader per transport: each is reference counted and released when its transport closes.
    let uploader = bazel::LocalFilesArtifactUploader::new(env)?;
    let text = path.to_string_lossy().into_owned();
    let java_path = runtime.file_system()?.get_path(&text)?;
    let stream = java_path
        .output_stream()
        .map_err(|e| anyhow::anyhow!("events.file: cannot write {text}: {e}"))?;
    let buffered = blazon_rt::java::io::BufferedOutputStream::new(env, &stream)?;
    let mode = bazel::BesUploadMode::wait_for_upload_complete(env)?;
    let transport = bazel::BinaryFormatFileTransport::new(
        env,
        &buffered,
        bep_options,
        &uploader,
        namer,
        &mode,
    )?;
    Ok(Transport {
        name: format!("file {text}"),
        java: transport
            .upcast::<bazel::BuildEventTransport>()
            .to_global()?,
        client: None,
        channel: None,
        results: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn bes_transport(
    env: &mut JNIEnv<'_>,
    runtime: &bazel::BlazeRuntime,
    bes: &Bes,
    bes_options: &bazel::BuildEventServiceOptions,
    bep_options: &bazel::BuildEventProtocolOptions,
    namer: &bazel::CountingArtifactGroupNamer,
    event_bus: &bazel::EventBus,
    invocation_id: &bazel::UUID,
    start_ms: i64,
    report: &Arc<artifacts::Report>,
) -> Result<Transport> {
    let uploader = match &bes.artifacts {
        Some(store) => artifacts::StoreUploader::create(env, store.clone(), report.clone())?,
        None => bazel::LocalFilesArtifactUploader::new(env)?
            .upcast::<bazel::BuildEventArtifactUploader>(),
    };
    let (target, tls) = grpc_target(&bes.endpoint);
    let builder = bazel::NettyChannelBuilder::for_target(env, &target)?;
    let builder = if tls {
        builder.use_transport_security()?
    } else {
        builder.use_plaintext()?
    };
    let channel = builder.build()?;
    let headers = bazel::Metadata::new(env)?;
    for (k, v) in &bes.headers {
        let marshaller = bazel::Metadata::ascii_string_marshaller(env)?;
        let key = bazel::MetadataKey::of(env, k, &marshaller)?;
        headers.put(&key, &bazel::JavaString::new(env, v)?)?;
    }
    let interceptor = bazel::MetadataUtils::new_attach_headers_interceptor(env, &headers)?;
    let build_id = bazel::UUID::random(env)?.to_string();
    let client = bazel::BuildEventServiceGrpcClient::new(
        env,
        &channel,
        None::<&bazel::CallCredentials>,
        &interceptor,
        &build_id,
        invocation_id,
    )?;
    let keywords = bazel::Set::<blazon_rt::java::lang::String>::new(env)?;
    for k in ["command_name=build", "protocol_name=BEP"] {
        keywords.add(&blazon_rt::java::lang::String::new(env, k)?)?;
    }
    let context = bazel::CommandContext::new(
        env,
        &build_id,
        &invocation_id.to_string(),
        1,
        &keywords,
        Some(bes.instance.as_str()).filter(|i| !i.is_empty()),
        false,
    )?;
    let transport = bazel::BuildEventServiceTransportBuilder::new(env)?
        .bes_client(&client)?
        .local_file_uploader(&uploader)?
        .bep_options(bep_options)?
        .bes_options(bes_options)?
        .clock(&runtime.clock()?)?
        .artifact_group_namer(namer)?
        .event_bus(event_bus)?
        .command_context(&context)?
        .command_start_time(&blazon_rt::java::time::Instant::of_epoch_milli(
            env, start_ms,
        )?)?
        .build()?;
    Ok(Transport {
        name: format!("bes {}", bes.endpoint),
        java: transport
            .upcast::<bazel::BuildEventTransport>()
            .to_global()?,
        client: Some(client.to_global()?),
        channel: Some(channel.to_global()?),
        results: Some(bes.results_url.clone())
            .filter(|u| !u.is_empty())
            .map(|u| format!("{u}{invocation_id}")),
    })
}
