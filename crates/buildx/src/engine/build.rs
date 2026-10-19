//! A blocking build over the local executor: what `ExecutionTool` and
//! `SkyframeBuilder` do for a `bazel build`, from the pieces a
//! `CommandEnvironment` would have held, built here by hand.

use super::*;

/// What one built artifact is, read from Skyframe once the build is done.
pub struct ArtifactInfo {
    pub artifact: Global<bazel::Artifact<'static>>,
    pub path: String,
    pub digest: Option<String>,
    pub size: i64,
    pub is_local: bool,
    /// The files of a tree artifact.
    pub children: Option<Vec<Arc<ArtifactInfo>>>,
}

/// A target's artifacts as read once, or why they could not be.
type ReadArtifacts = std::result::Result<Arc<Vec<Arc<ArtifactInfo>>>, String>;

/// The outcome of one build for every requested target, kept on the Java side:
/// a target's configured target, artifacts and metadata are read when asked
/// for, so a script that only counts results costs nothing per target.
///
/// What is read later is read from the build's graph as it is then: a later
/// build that changes an artifact is seen by results of an earlier one.
pub struct Batch {
    evaluator: Global<bazel::MemoizingEvaluator<'static>>,
    context: Global<bazel::TopLevelArtifactContext<'static>>,
    configured: Option<Global<bazel::ImmutableList<'static, bazel::ConfiguredTarget<'static>>>>,
    /// Requested targets' labels, in order.
    pub labels: Arc<Vec<Global<bazel::Label<'static>>>>,
    positions: Option<Vec<Option<usize>>>,
    failures: HashMap<usize, String>,
    /// How many of the failures are analysis failures.
    analysis_failures: usize,
    artifacts: Vec<std::sync::OnceLock<ReadArtifacts>>,
}

impl Batch {
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// Why requested target `i` did not build, if it did not.
    pub fn failure(&self, i: usize) -> Option<&str> {
        self.failures.get(&i).map(String::as_str)
    }

    fn position(&self, i: usize) -> Option<usize> {
        match &self.positions {
            None => Some(i),
            Some(p) => p[i],
        }
    }

    /// The configured target of requested target `i`, None when it did not analyze.
    pub fn target(&self, i: usize) -> Result<Option<Global<bazel::ConfiguredTarget<'static>>>> {
        let (Some(configured), Some(p)) = (&self.configured, self.position(i)) else {
            return Ok(None);
        };
        let mut env = vm::env()?;
        Ok(Some(
            configured
                .get(&mut env)?
                .upcast::<bazel::List<bazel::ConfiguredTarget>>()
                .get(p)?
                .to_global()?,
        ))
    }

    /// The artifacts requested target `i` built, with their metadata, read on first use.
    pub fn artifacts(&self, i: usize) -> Result<Arc<Vec<Arc<ArtifactInfo>>>> {
        if self.failures.contains_key(&i) {
            return Ok(Arc::new(Vec::new()));
        }
        self.artifacts[i]
            .get_or_init(|| {
                self.read_artifacts(i)
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"))
            })
            .clone()
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn read_artifacts(&self, i: usize) -> Result<Vec<Arc<ArtifactInfo>>> {
        let (Some(configured), Some(p)) = (&self.configured, self.position(i)) else {
            return Ok(Vec::new());
        };
        let mut env = vm::env()?;
        let target = configured
            .get(&mut env)?
            .upcast::<bazel::List<bazel::ConfiguredTarget>>()
            .get(p)?;
        let evaluator = self.evaluator.get(&mut env)?;
        let context = self.context.get(&mut env)?;
        artifact_infos(&mut env, &evaluator, &target, &context)
    }
}

/// The action cache's statistics of the build that just ran, as Bazel counted
/// them: `ExecutionTool` merges them into a builder the same way.
fn action_cache_statistics(
    env: &mut JNIEnv<'_>,
    cache: &bazel::ActionCache,
) -> Result<axl_proto::blaze::ActionCacheStatistics> {
    use prost::Message;
    let builder = bazel::ActionCacheStatistics::new_builder(env)?;
    cache.merge_into_action_cache_statistics(&builder)?;
    let bytes = builder
        .build()?
        .upcast::<bazel::AbstractMessageLite>()
        .to_byte_array()?;
    Ok(axl_proto::blaze::ActionCacheStatistics::decode(
        bytes.as_slice(),
    )?)
}

/// How a build ended, for the event stream.
fn outcome_of(built: &Result<Batch>, targets: usize) -> events::Outcome {
    match built {
        Err(e) => events::Outcome {
            exit: events::Exit::BuildFailure,
            message: Some(format!("{e:#}")),
            targets,
            failed: targets,
        },
        Ok(batch) => {
            let first = (0..batch.len()).find_map(|i| batch.failure(i).map(str::to_string));
            let failed = (0..batch.len())
                .filter(|i| batch.failure(*i).is_some())
                .count();
            events::Outcome {
                exit: if failed == 0 {
                    events::Exit::Success
                } else if batch.analysis_failures < failed {
                    events::Exit::BuildFailure
                } else if failed == batch.len() {
                    events::Exit::AnalysisFailure
                } else {
                    events::Exit::PartialAnalysisFailure
                },
                message: first,
                targets: batch.len(),
                failed: (0..batch.len())
                    .filter(|i| batch.failure(*i).is_some())
                    .count(),
            }
        }
    }
}

impl Engine {
    /// Builds the targets of `spec` in `config` with the default local executor,
    /// to completion. A target that fails is a failure in the batch; the
    /// others still build.
    pub fn build(
        &self,
        spec: &[Spec],
        config: &EngineConfig,
        router: Arc<dyn crate::execution::Router>,
        sinks: &[Arc<crate::events::Sink>],
        invocation: &crate::events::Invocation,
    ) -> Result<Batch> {
        self.synced(
            &config.parser,
            &config.half,
            "build",
            true,
            |env, handler| {
                let status = status::Ran::default();
                let bep = if sinks.is_empty() {
                    None
                } else {
                    events::Bep::start(env, self, sinks, invocation, &status)?
                };
                let packages = self.loading.skyframe.get(env)?.package_manager()?;
                packages.get_and_clear_statistics()?;
                let built =
                    self.build_synced(env, handler, spec, config, router, bep.as_ref(), &status);
                if let Some(bep) = &bep {
                    let loaded = packages
                        .get_and_clear_statistics()?
                        .packages_successfully_loaded()?;
                    bep.collect(|m| m.packages_loaded = i64::from(loaded));
                }
                if let Some(bep) = bep {
                    bep.finish(env, &outcome_of(&built, invocation.targets.len()));
                }
                built
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_synced(
        &self,
        env: &mut JNIEnv<'_>,
        handler: &bazel::ExtendedEventHandler,
        spec: &[Spec],
        config: &EngineConfig,
        router: Arc<dyn crate::execution::Router>,
        bep: Option<&events::Bep>,
        status: &status::Ran,
    ) -> Result<Batch> {
        let l = &self.loading;
        let runtime = self.runtime.get(env)?;
        let skyframe = l.skyframe.get(env)?;
        let parser = config.parser.get(env)?;
        let reporter = l.reporter.get(env)?;

        let targets = self.resolve(env, handler, spec, config)?;
        if let Some(bep) = bep {
            let mut labels = Vec::new();
            for t in &targets.get(env)?.upcast::<bazel::Iterable<bazel::Target>>() {
                labels.push(t?.label()?.to_string());
            }
            bep.collect(|m| m.targets_loaded = labels.len() as i64);
            bep.pattern(env, labels)?;
        }
        let core = self.analysis_core(env, handler, &targets, config)?;
        let roots = &core.roots;
        if let Some(bep) = bep {
            let analyzed = (0..core.labels.len())
                .filter(|i| core.position(*i).is_some())
                .count();
            bep.collect(|m| m.targets_configured = analyzed as i64);
        }
        let view = skyframe.skyframe_build_view()?;
        let requested = core.labels.len();
        let evaluator = skyframe.evaluator()?;
        let artifact_context = top_level_artifact_context(env, &parser)?;
        let analyzed_any = (0..requested).any(|i| core.position(i).is_some());
        let empty_artifacts = || (0..requested).map(|_| std::sync::OnceLock::new()).collect();
        if !analyzed_any {
            return Ok(Batch {
                evaluator: evaluator.to_global()?,
                context: artifact_context.to_global()?,
                configured: None,
                labels: core.labels.clone(),
                positions: core.positions.clone(),
                failures: core.failures.clone(),
                analysis_failures: core.failures.len(),
                artifacts: empty_artifacts(),
            });
        }
        let configured = core.configured.get(env)?;

        let mut phase = crate::phases::start("setup: execroot + symlink forest");
        // BlazeWorkspace and the directories a CommandEnvironment hands out.
        let workspace = runtime.workspace()?;
        let directories = workspace.directories()?;
        let workspace_name = runtime.rule_class_provider()?.runfiles_prefix()?;
        let exec_root = directories.get_exec_root(&workspace_name)?;
        exec_root.create_directory_and_parents()?;

        // BuildView: artifact roots from the analysis; ExecutionTool.prepare: the
        // source forest under the execution root.
        view.artifact_factory()?
            .set_package_roots(&roots.package_root_lookup()?)?;
        let language = parser.get_options(&class::<bazel::BuildLanguageOptions>(
            env,
            LANGUAGE_OPTIONS,
        )?)?;
        bazel::SymlinkForest::new(
            env,
            &roots.package_roots_map()?,
            &exec_root,
            &runtime.product_name()?,
            language.experimental_sibling_repository_layout()?,
        )?
        .plant_symlink_forest()?;

        crate::phases::next(&mut phase, "setup: output service + startBuild");
        // CommandEnvironment.beforeCommand: the output service.
        let output_service = bazel::LocalOutputService::new(env, &directories)?;
        skyframe.set_output_service(Some(&output_service))?;
        let request =
            parser.get_options(&class::<bazel::BuildRequestOptions>(env, REQUEST_OPTIONS)?)?;
        let execution =
            parser.get_options(&class::<bazel::ExecutionOptions>(env, EXECUTION_OPTIONS)?)?;
        let modified = output_service.start_build(
            &bazel::UUID::random(env)?,
            &workspace_name,
            handler,
            true,
        )?;

        crate::phases::next(&mut phase, "setup: action log directory");
        // ExecutionTool.createActionLogDirectory.
        let temps = directories.get_action_temps_directory(&exec_root)?;
        if temps.exists()? {
            temps.delete_tree()?;
        }
        temps.create_directory_and_parents()?;

        crate::phases::next(&mut phase, "setup: action cache load");
        // ExecutionTool.createBuilder: action cache and its checker.
        let cache = if request.use_action_cache()? {
            Some(workspace.get_or_load_persistent_action_cache(&reporter)?)
        } else {
            None
        };
        skyframe.set_action_output_root(&temps)?;
        let artifact_factory = view.artifact_factory()?;
        crate::phases::next(&mut phase, "setup: checker + file cache");
        let up_to_date = bazel::CheckUpToDateFilter::from_options(env, &execution)?;
        let cache_config = bazel::CacheConfig::builder(env)?
            .set_enabled(request.use_action_cache()?)?
            .set_store_output_metadata(
                output_service.should_store_remote_output_metadata_in_action_cache()?,
            )?
            .build()?;
        let checker = bazel::ActionCacheChecker::new(
            env,
            cache.as_ref(),
            &artifact_factory,
            &skyframe.action_key_context()?,
            &up_to_date,
            &output_service.proxy_metadata_factory()?,
            Some(&cache_config),
        )?;
        let directory_helper = bazel::ActionOutputDirectoryHelper::new(
            env,
            &request.directory_creation_cache_spec()?,
        )?;
        let output_path = bazel::PathFragment::create(env, &directories.relative_output_path()?)?;
        let syscalls = bazel::SyscallCache::no_cache(env)?;
        let file_cache = bazel::SingleBuildFileCache::new(
            env,
            &exec_root.path_string()?,
            &output_path,
            &runtime.file_system()?,
            &syscalls,
        )?;

        crate::phases::next(&mut phase, "setup: executor + resources");
        // SkyframeBuilder.buildArtifacts.
        let route_errors: strategy::RouteErrors = Default::default();
        let counters = Arc::new(metrics::Counters::default());
        let resources = bazel::ResourceManager::new(env)?;
        resources.set_available_resources(&bazel::ResourceSet::create(
            env,
            &execution.local_resources()?,
            i32::MAX,
        )?)?;
        resources.set_allow_one_action_on_resource_unavailable(
            execution.allow_one_action_on_resource_unavailable()?,
        )?;
        let (executor, _sandbox) = local_executor(
            env,
            &runtime,
            &reporter,
            &parser,
            &exec_root,
            &workspace_name,
            &output_service,
            &resources,
            &artifact_factory.upcast::<bazel::ArtifactResolver>(),
            router,
            route_errors.clone(),
            counters.clone(),
            status,
        )?;
        crate::phases::next(&mut phase, "setup: detect modified outputs");
        skyframe.detect_modified_output_files(
            &modified,
            None::<&bazel::Range<blazon_rt::java::lang::Long>>,
            &output_service
                .upcast::<bazel::OutputService>()
                .output_checker()?,
            FSVC_THREADS,
        )?;
        let salt = action_execution_salt(env)?;
        skyframe.configure_action_executor(
            &file_cache,
            &bazel::ActionInputPrefetcher::none(env)?,
            &salt,
            MAX_STDOUT_ERR_BYTES,
        )?;
        skyframe.set_salt_and_delete_actions_if_changed(&salt)?;

        crate::phases::next(&mut phase, "setup: configure executor + target list");
        // Bazel's own counter of completed and enqueued actions, and a handler
        // that keeps the errors reported while the build runs, for the progress line.
        let event_bus = l.event_bus.get(env)?;
        let counter = bazel::ExecutionProgressReceiver::new(env, 0, &event_bus)?;
        let errors = bazel::StoredEventHandler::new(env)?;
        reporter.add_handler(&errors)?;
        let progress = crate::progress::Progress::start(counter.to_global()?, errors.to_global()?);
        crate::phases::next(&mut phase, "build: buildArtifacts");
        if let Some(cache) = &cache {
            cache.reset_statistics()?;
        }
        let execution_started = std::time::Instant::now();
        if let Some(bep) = bep {
            let analysis = bep.elapsed_ms();
            bep.collect(|m| {
                m.analysis_ms = analysis;
                m.execution_start_ms = analysis;
            });
        }
        // BuildView.update: the workspace status artifacts are built with every build.
        let status_artifacts = bazel::Set::<bazel::Artifact>::new(env)?;
        for artifact in &skyframe
            .get_workspace_status_artifacts(handler)?
            .upcast::<bazel::Iterable<bazel::Artifact>>()
        {
            status_artifacts.add(&artifact?)?;
        }
        let built = skyframe.build_artifacts(
            &reporter,
            &resources,
            &executor,
            &status_artifacts,
            &configured,
            &bazel::ImmutableSet::empty::<bazel::Object>(env)?,
            &bazel::Set::<bazel::ConfiguredTarget>::new(env)?,
            &bazel::Set::<bazel::ConfiguredTarget>::new(env)?,
            &parser,
            &checker,
            &directory_helper,
            Some(&counter),
            &artifact_context,
        );
        let progress_text = counter.progress_string().unwrap_or_default();
        progress.finish();
        reporter.remove_handler(&errors)?;
        if let Some(bep) = bep {
            let stats = match &cache {
                Some(cache) => Some(action_cache_statistics(env, cache)?),
                None => None,
            };
            let enqueued = crate::progress::parse(&progress_text).map_or(0, |(_, total)| total);
            bep.collect(|m| {
                m.take(&counters);
                m.status_ran = status.get();
                m.cache = stats;
                m.actions_enqueued = enqueued;
                m.execution_ms = execution_started.elapsed().as_millis() as i64;
            });
        }
        crate::phases::next(&mut phase, "build: finish + cache.save");
        let result = built?;
        if let Some(cache) = &cache {
            cache.save()?;
        }
        if result.has_error()? {
            // A failed workspace status action belongs to no target: as for a
            // command, the build fails with its message.
            for artifact in &status_artifacts.upcast::<bazel::Iterable<bazel::Artifact>>() {
                let key = bazel::Artifact::key(env, &artifact?)?;
                if let Some(e) = result.get_error(&key)? {
                    status.failed();
                    bail!("{}", failure_message(&e)?);
                }
            }
        }

        crate::phases::next(&mut phase, "results: failures");
        let mut failures = core.failures.clone();
        if result.has_error()? {
            // Only a build with an error asks each target what became of it.
            for i in 0..requested {
                let Some(p) = core.position(i) else {
                    continue;
                };
                let target = configured
                    .upcast::<bazel::List<bazel::ConfiguredTarget>>()
                    .get(p)?;
                let target_key = bazel::ConfiguredTargetKey::from_configured_target(env, &target)?;
                let key =
                    bazel::TargetCompletionKey::create(env, &target_key, &artifact_context, false)?;
                if let Some(e) = result.get_error(&key)? {
                    let message = failure_message(&e)?;
                    failures.insert(i, explain(&route_errors, &core.labels[i], message, env));
                }
            }
        }
        drop(phase);
        Ok(Batch {
            evaluator: evaluator.to_global()?,
            context: artifact_context.to_global()?,
            configured: Some(core.configured.clone()),
            labels: core.labels.clone(),
            positions: core.positions.clone(),
            failures,
            analysis_failures: core.failures.len(),
            artifacts: empty_artifacts(),
        })
    }
}

const REQUEST_OPTIONS: &str = "com/google/devtools/build/lib/buildtool/BuildRequestOptions";
const EXECUTION_OPTIONS: &str = "com/google/devtools/build/lib/exec/ExecutionOptions";
const LOCAL_EXECUTION_OPTIONS: &str =
    "com/google/devtools/build/lib/exec/local/LocalExecutionOptions";
const FSVC_THREADS: i32 = 200;
const MAX_STDOUT_ERR_BYTES: i32 = 1_048_576;

/// The executor `ExecutionTool` would assemble from `StandaloneModule`: action
/// contexts for the actions Bazel executes itself (file writes, template
/// expansion, symlink trees), spawns resolved to the local runner, and a
/// `BlazeExecutor` over both.
#[allow(clippy::too_many_arguments)]
fn local_executor<'l>(
    env: &mut JNIEnv<'l>,
    runtime: &bazel::BlazeRuntime<'l>,
    reporter: &bazel::Reporter,
    parser: &bazel::OptionsParser<'l>,
    exec_root: &bazel::Path<'l>,
    workspace_name: &str,
    output_service: &bazel::LocalOutputService,
    resources: &bazel::ResourceManager<'l>,
    artifact_resolver: &bazel::ArtifactResolver,
    mut router: Arc<dyn crate::execution::Router>,
    route_errors: strategy::RouteErrors,
    counters: Arc<metrics::Counters>,
    status: &status::Ran,
) -> Result<(bazel::BlazeExecutor<'l>, Option<sandboxed::Sandbox>)> {
    let execution =
        parser.get_options(&class::<bazel::ExecutionOptions>(env, EXECUTION_OPTIONS)?)?;
    let local = parser.get_options(&class::<bazel::LocalExecutionOptions>(
        env,
        LOCAL_EXECUTION_OPTIONS,
    )?)?;

    let environment = client_env(env, false)?;
    let env_provider = bazel::LocalEnvProvider::for_current_os(env, &environment)?;
    let wrapper = process_wrapper(env, runtime)?
        .ok_or_else(|| anyhow::anyhow!("the release ships no process-wrapper"))?;
    let syscalls = bazel::SyscallCache::no_cache(env)?;
    let runfiles = bazel::RunfilesTreeUpdater::new(env, exec_root, &syscalls)?;
    let spawn_runner = bazel::LocalSpawnRunner::new(
        env,
        exec_root,
        &local,
        resources,
        &env_provider,
        &runtime.workspace()?.bin_tools()?,
        &wrapper,
        &runfiles,
    )?;
    let strategies = bazel::SpawnStrategyRegistry::builder(env)?;
    let standalone = bazel::StandaloneSpawnStrategy::new(env, &spawn_runner, &execution)?;
    // Bazel's sandboxed runner, for executors that ask for it. The default
    // executor can do without; one that was asked for cannot.
    let need = router.sandboxing();
    let sandbox = match sandboxed::Sandbox::start(
        env,
        need,
        &sandboxed::Parts {
            runtime,
            exec_root,
            resources,
            execution: &execution,
            wrapper: &wrapper,
            env_provider: &env_provider,
        },
    ) {
        Ok(sandbox) => sandbox,
        Err(e) if need == crate::sandbox::Sandboxing::Optional => {
            eprintln!("buildx: no sandbox ({e:#}); actions run unsandboxed");
            router = crate::execution::local_router();
            None
        }
        Err(e) => return Err(e.context("ctx.execution.sandbox()")),
    };
    strategies.register_strategy(
        &bazel::SpawnStrategy::from_impl(
            env,
            Box::new(strategy::Strategy {
                router,
                errors: route_errors,
                delegate: standalone.to_global()?,
                sandbox: sandbox.as_ref().map(|s| s.strategy.clone()),
                counters,
            }),
        )?,
        &["route"],
    )?;
    strategies.register_strategy(&standalone, &["standalone", "local"])?;
    strategies.set_default_strategies(&bazel::List::of_strs(env, &["route", "standalone"])?)?;
    let strategies = strategies.build()?;

    let contexts = bazel::ModuleActionContextRegistry::builder(env)?;
    contexts.register(
        &class::<bazel::SpawnStrategyResolver>(env, SPAWN_STRATEGY_RESOLVER)?,
        &bazel::SpawnStrategyResolver::new(env)?,
        &[] as &[&str],
    )?;
    contexts.register(
        &class::<bazel::SymlinkTreeActionContext>(env, SYMLINK_TREE_CONTEXT)?,
        &bazel::SymlinkTreeStrategy::new(env, output_service, workspace_name)?,
        &[] as &[&str],
    )?;
    contexts.register(
        &class::<bazel::FileWriteActionContext>(env, FILE_WRITE_CONTEXT)?,
        &bazel::FileWriteStrategy::new(env)?,
        &["local"],
    )?;
    contexts.register(
        &class::<bazel::TemplateExpansionContext>(env, TEMPLATE_EXPANSION_CONTEXT)?,
        &bazel::LocalTemplateExpansionStrategy::new(env)?,
        &["local"],
    )?;
    contexts.register(
        &class::<bazel::WorkspaceStatusContext>(env, status::WORKSPACE_STATUS_CONTEXT)?,
        &bazel::WorkspaceStatusContext::from_impl(
            env,
            Box::new(status::Context {
                parser: parser.to_global()?,
                workspace: runtime.workspace()?.workspace()?.to_global()?,
                ran: status.clone(),
            }),
        )?,
        &[] as &[&str],
    )?;
    contexts.register(
        &class::<bazel::CppIncludeExtractionContext>(env, CPP_INCLUDE_EXTRACTION_CONTEXT)?,
        &bazel::CppIncludeExtractionContext::from_impl(
            env,
            Box::new(contexts::Extraction {
                resolver: artifact_resolver.to_global()?,
            }),
        )?,
        &[] as &[&str],
    )?;
    contexts.register(
        &class::<bazel::CppIncludeScanningContext>(env, CPP_INCLUDE_SCANNING_CONTEXT)?,
        &bazel::CppIncludeScanningContext::from_impl(env, Box::new(contexts::Scanning))?,
        &[] as &[&str],
    )?;
    contexts.register(
        &class::<bazel::SpawnStrategyRegistry>(env, SPAWN_STRATEGY_REGISTRY)?,
        &strategies,
        &[] as &[&str],
    )?;
    contexts.restrict_to(&class::<bazel::Object>(env, SYMLINK_TREE_CONTEXT)?, "")?;
    contexts.restrict_to(
        &class::<bazel::Object>(env, status::WORKSPACE_STATUS_CONTEXT)?,
        "",
    )?;

    let executor = bazel::BlazeExecutor::new(
        env,
        &runtime.file_system()?,
        exec_root,
        reporter,
        &runtime.clock()?,
        &runtime.bug_reporter()?,
        parser,
        &contexts.build()?,
        &strategies,
    )?;
    Ok((executor, sandbox))
}

const SPAWN_STRATEGY_RESOLVER: &str = "com/google/devtools/build/lib/exec/SpawnStrategyResolver";
const SYMLINK_TREE_CONTEXT: &str =
    "com/google/devtools/build/lib/analysis/actions/SymlinkTreeActionContext";
const FILE_WRITE_CONTEXT: &str =
    "com/google/devtools/build/lib/analysis/actions/FileWriteActionContext";
const TEMPLATE_EXPANSION_CONTEXT: &str =
    "com/google/devtools/build/lib/analysis/actions/TemplateExpansionContext";
const CPP_INCLUDE_EXTRACTION_CONTEXT: &str =
    "com/google/devtools/build/lib/rules/cpp/CppIncludeExtractionContext";
const CPP_INCLUDE_SCANNING_CONTEXT: &str =
    "com/google/devtools/build/lib/rules/cpp/CppIncludeScanningContext";
const SPAWN_STRATEGY_REGISTRY: &str = "com/google/devtools/build/lib/exec/SpawnStrategyRegistry";

/// The artifacts a target's completion built (its default outputs and the
/// requested output groups), with the metadata Skyframe recorded for them.
fn artifact_infos(
    env: &mut JNIEnv<'_>,
    evaluator: &bazel::MemoizingEvaluator,
    target: &bazel::ConfiguredTarget,
    context: &bazel::TopLevelArtifactContext,
) -> Result<Vec<Arc<ArtifactInfo>>> {
    let all = bazel::TopLevelArtifactHelper::get_all_artifacts_to_build(env, target, context)?
        .all_artifacts()?
        .to_list()?;
    let mut out = Vec::new();
    for a in &all.upcast::<bazel::Iterable<bazel::Artifact>>() {
        out.push(Arc::new(artifact_info(env, evaluator, &a?)?));
    }
    Ok(out)
}

fn artifact_info(
    env: &mut JNIEnv<'_>,
    evaluator: &bazel::MemoizingEvaluator,
    artifact: &bazel::Artifact,
) -> Result<ArtifactInfo> {
    let path = artifact.path()?.path_string()?;
    let value = evaluator.get_existing_value(&bazel::Artifact::key(env, artifact)?)?;
    let global = artifact.to_global()?;
    let Some(value) = value else {
        bail!("Skyframe holds no value for built artifact {path}");
    };
    if let Ok(tree) = value.downcast::<bazel::TreeArtifactValue>() {
        let mut children = Vec::new();
        let values = tree
            .child_values()?
            .upcast::<bazel::Map<bazel::TreeFileArtifact, bazel::FileArtifactValue>>();
        for e in values.entries()? {
            let e = e?;
            let child = e.key()?.upcast::<bazel::Artifact>();
            children.push(Arc::new(file_info(
                child.to_global()?,
                child.path()?.path_string()?,
                &e.value()?,
            )?));
        }
        let mut info = file_info(global, path, &tree.metadata()?)?;
        info.children = Some(children);
        return Ok(info);
    }
    if let Ok(action) = value.downcast::<bazel::ActionExecutionValue>() {
        return file_info(
            global,
            path,
            &action.get_existing_file_artifact_value(artifact)?,
        );
    }
    let file = value.downcast::<bazel::FileArtifactValue>()?;
    file_info(global, path, &file)
}

fn file_info(
    artifact: Global<bazel::Artifact<'static>>,
    path: String,
    value: &bazel::FileArtifactValue,
) -> Result<ArtifactInfo> {
    // `getDigest` is null for some artifacts (symlinks).
    let bytes = value.digest()?.unwrap_or_default();
    let digest = (!bytes.is_empty()).then(|| bytes.iter().map(|b| format!("{b:02x}")).collect());
    Ok(ArtifactInfo {
        artifact,
        path,
        digest,
        size: value.size()?,
        is_local: !value.is_remote()?,
        children: None,
    })
}

/// A failed action's message with the routing error that caused it, when
/// there was one: Bazel only says the outputs were not created.
fn explain(
    errors: &strategy::RouteErrors,
    label: &Global<bazel::Label<'static>>,
    message: String,
    env: &mut JNIEnv<'_>,
) -> String {
    let Ok(errors) = errors.lock() else {
        return message;
    };
    if errors.is_empty() {
        return message;
    }
    let own = label.get(env).map(|l| l.to_string()).unwrap_or_default();
    let ours: Vec<&str> = errors
        .iter()
        .filter(|(l, _)| *l == own)
        .map(|(_, m)| m.as_str())
        .collect();
    if ours.is_empty() {
        format!(
            "{message} (routing failed for {}: {})",
            errors[0].0, errors[0].1
        )
    } else {
        ours.join("\n")
    }
}

/// The action execution salt `RemoteModule` hands the executor builder: a
/// digest of "remote cache enabled" and the default remote exec properties.
/// No remote option exists here, so it is the digest of `false` and no
/// properties, which is what Bazel computes without `--remote_*` flags. The
/// salt is part of every action's cache entry, so a different one (the empty
/// string, say) makes Bazel see each action as changed.
fn action_execution_salt(env: &mut JNIEnv<'_>) -> Result<String> {
    let fingerprint = bazel::Fingerprint::new(env)?;
    fingerprint.add_boolean(false)?;
    let none = bazel::ImmutableMap::empty::<bazel::JavaString, bazel::JavaString>(env)?;
    fingerprint.add_string_map(&none)?;
    Ok(fingerprint.hex_digest_and_reset()?)
}
