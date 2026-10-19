//! Bazel's runtime hosted in this process. Package loading and queries are
//! driven on the `SkyframeExecutor` directly, the way a command would
//! underneath, with no command, dispatcher or `CommandEnvironment` involved.

use crate::bindings::v9_0_1 as bazel;
use crate::output_base;
use anyhow::{Result, bail};
use bazel::{BazelRepositoryModule, Object, OptionsBase};
use blazon::release::Install;
use blazon_rt::{Global, HandleExt, Jvm, vm};
use jni::JNIEnv;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct EngineOptions {
    pub install: Install,
    pub workspace: PathBuf,
    pub output_user_root: PathBuf,
    pub output_base: PathBuf,
    /// JVM `-Xmx` value, e.g. `2g`.
    pub heap: String,
    /// The invoking directory relative to the workspace; target patterns resolve against it.
    pub relative_working_directory: String,
}

/// A live object on the engine's heap, kept across calls.
pub type Live = Global<Object<'static>>;

/// What one `sync` of package loading needs, built once and reused per query.
struct Loading {
    skyframe: Global<bazel::SkyframeExecutor<'static>>,
    /// Rebuilt when the applied engine half's `package_path` changes.
    package_locator: Mutex<Global<bazel::PathPackageLocator<'static>>>,
    /// An `OptionsParser` over every option class the engine reads during
    /// loading, parsed with no arguments, so defaults throughout.
    options: Global<bazel::OptionsParser<'static>>,
    tsgm: Global<bazel::TimestampGranularityMonitor<'static>>,
    executors: Global<bazel::QuiescingExecutors<'static>>,
    reporter: Global<bazel::Reporter<'static>>,
    event_bus: Global<bazel::EventBus<'static>>,
    lockfile_module: Option<Global<bazel::BazelLockFileModule<'static>>>,
    client_env: Global<
        bazel::ImmutableMap<'static, bazel::JavaString<'static>, bazel::JavaString<'static>>,
    >,
    relative_working_directory: Global<bazel::PathFragment<'static>>,
}

/// A parsed configuration: the parser holding every flag's value (what
/// `SkyframeExecutor.sync` reads) and the `BuildOptions` the rule fragments
/// read.
pub struct EngineConfig {
    /// The engine half: applied before a call when it differs from the last.
    pub(crate) half: Half,
    parser: Global<bazel::OptionsParser<'static>>,
    options: Global<bazel::BuildOptions<'static>>,
}

impl EngineConfig {
    pub fn checksum(&self) -> Result<String> {
        let mut env = vm::env()?;
        Ok(self.options.get(&mut env)?.checksum()?)
    }

    /// Whether both configurations hold the same options, as `BuildOptions.equals` says.
    pub fn same_as(&self, other: &EngineConfig) -> Result<bool> {
        let mut env = vm::env()?;
        Ok(self.options.get(&mut env)? == other.options.get(&mut env)?)
    }
}

mod apply;
mod artifacts;
mod build;
mod contexts;
mod events;
mod metrics;
mod sandboxed;
mod status;
mod strategy;
pub use apply::Half;
pub use build::{ArtifactInfo, Batch};

/// What a script named to build or analyze: a pattern, run as a query in the
/// call's own synced graph, or a live target.
pub enum Spec {
    Pattern(String),
    Live(Live),
}

/// The target keys of the last target set analyzed, reused while the targets
/// and the top-level configuration are equal.
#[derive(Clone)]
struct KeysCache {
    targets: Global<bazel::Collection<'static, bazel::Target<'static>>>,
    top_key: Global<bazel::BuildConfigurationKey<'static>>,
    by_label: Global<bazel::ImmutableMap<'static, bazel::Label<'static>, bazel::Target<'static>>>,
    keys: Global<bazel::ImmutableList<'static, bazel::ConfiguredTargetKey<'static>>>,
    labels: Arc<Vec<Global<bazel::Label<'static>>>>,
}

/// The outcome of analysis for the requested targets, held on the Java side.
pub(crate) struct Core<'l> {
    pub labels: Arc<Vec<Global<bazel::Label<'static>>>>,
    /// The targets that analyzed, in order.
    pub configured: Global<bazel::ImmutableList<'static, bazel::ConfiguredTarget<'static>>>,
    /// For each requested target its place in `configured`, None when every one analyzed.
    pub positions: Option<Vec<Option<usize>>>,
    pub failures: HashMap<usize, String>,
    pub roots: bazel::PackageRoots<'l>,
}

impl Core<'_> {
    /// Where requested target `i` is in `configured`, None when it did not analyze.
    pub fn position(&self, i: usize) -> Option<usize> {
        match &self.positions {
            None => Some(i),
            Some(p) => p[i],
        }
    }
}

/// One requested target's analysis outcome.
pub struct Analyzed {
    pub label: Global<bazel::Label<'static>>,
    pub target: Option<Global<bazel::ConfiguredTarget<'static>>>,
    pub failure: Option<String>,
}

pub struct Engine {
    jvm: Jvm,
    runtime: Global<bazel::BlazeRuntime<'static>>,
    loading: Loading,
    default_config: Mutex<Option<Arc<EngineConfig>>>,
    /// The engine half the modules were last given; empty is the defaults they
    /// were seeded with.
    applied: Mutex<apply::Applied>,
    keys_cache: Mutex<Option<KeysCache>>,
    _lock: output_base::Lock,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Engine")
    }
}

const PACKAGE_OPTIONS: &str = "com/google/devtools/build/lib/pkgcache/PackageOptions";
const LANGUAGE_OPTIONS: &str =
    "com/google/devtools/build/lib/packages/semantics/BuildLanguageOptions";
const COMMON_OPTIONS: &str = "com/google/devtools/build/lib/runtime/CommonCommandOptions";
const REPOSITORY_OPTIONS: &str = "com/google/devtools/build/lib/bazel/repository/RepositoryOptions";
const QUERY_OPTIONS: &str = "com/google/devtools/build/lib/query2/common/CommonQueryOptions";

/// The option classes of `bazel build` that are not rule fragments: the
/// command's own `options` plus the loading ones every command shares.
const BUILD_COMMAND_OPTIONS: [&str; 8] = [
    "com/google/devtools/build/lib/buildtool/BuildRequestOptions",
    "com/google/devtools/build/lib/exec/ExecutionOptions",
    "com/google/devtools/build/lib/exec/local/LocalExecutionOptions",
    "com/google/devtools/build/lib/analysis/AnalysisOptions",
    "com/google/devtools/build/lib/pkgcache/LoadingOptions",
    "com/google/devtools/build/lib/runtime/KeepGoingOption",
    "com/google/devtools/build/lib/runtime/LoadingPhaseThreadsOption",
    "com/google/devtools/build/lib/analysis/WorkspaceStatusAction$Options",
];

impl Engine {
    pub fn start(o: EngineOptions) -> Result<Engine> {
        let label = fs::read_to_string(o.install.root.join("build-label.txt"))
            .map(|l| l.trim().to_string())
            .unwrap_or_default();
        if label != bazel::BAZEL_VERSION {
            bail!(
                "no bindings for Bazel {label}: buildx is bound to {} (generated: {}); add {label} to `versions` in crates/buildx/bindings.toml and bind the engine to it",
                bazel::BAZEL_VERSION,
                crate::bindings::SUPPORTED.join(", ")
            );
        }
        fs::create_dir_all(&o.output_base)?;
        let lock = output_base::Lock::acquire(&o.output_base, &o.workspace)?;
        output_base::ensure_no_server(&o.output_base, &o.workspace)?;
        let logprops = o.output_base.join("javalog.properties");
        fs::write(&logprops, javalog(&o.output_base.join("java.log")))?;
        let jvm = Jvm::start(&o.install.libjvm(), &jvm_options(&o, &logprops))?;
        let mut env = vm::env()?;
        let runtime = boot(&mut env, &startup_options(&o))?;
        let loading = prepare_loading(&mut env, &runtime, &o.relative_working_directory)?;
        Ok(Engine {
            jvm,
            runtime: runtime.to_global()?,
            loading,
            default_config: Mutex::new(None),
            applied: Mutex::new(apply::Applied {
                fingerprint: String::new(),
                ignore_dev_deps: false,
            }),
            keys_cache: Mutex::new(None),
            _lock: lock,
        })
    }

    /// The live `BlazeRuntime`.
    pub fn runtime(&self) -> &Global<bazel::BlazeRuntime<'static>> {
        &self.runtime
    }

    /// The calling thread's `JNIEnv`, for statics and constructors.
    pub fn env(&self) -> Result<JNIEnv<'_>> {
        Ok(vm::env()?)
    }

    /// Parses `arguments` with the engine's own `OptionsParser` over the
    /// option classes of a build and derives the `BuildOptions` from it, the way
    /// `BlazeRuntime.createBuildOptions` does for a command. Starlark build
    /// settings (`starlark`, in option-string form) are resolved by Skyframe's
    /// `ParsedFlagsFunction`, which needs the graph synced with this parser and
    /// knows only the configuration's own options (`fragment_arguments`).
    pub fn config(
        &self,
        arguments: &[String],
        fragment_arguments: &[String],
        starlark: &[String],
        half: Half,
    ) -> Result<EngineConfig> {
        // A build reports each target's failure as data, so every target is
        // attempted: the build-wide `keep_going` that the script never sees.
        let mut arguments = arguments.to_vec();
        arguments.push("--keep_going".into());
        let arguments = &arguments[..];
        let (parser, native) = {
            let mut env = vm::env()?;
            let runtime = self.runtime.get(&mut env)?;
            let parser = parse_flags(&mut env, &runtime, arguments)?.to_global()?;
            let options = runtime
                .create_build_options(&parser.get(&mut env)?)?
                .to_global()?;
            (parser, options)
        };
        if starlark.is_empty() {
            return Ok(EngineConfig {
                half,
                parser,
                options: native,
            });
        }
        let all: Vec<String> = fragment_arguments.iter().chain(starlark).cloned().collect();
        let options = self.synced(&parser, &Half::default(), "build", false, |env, handler| {
            let args = bazel::List::of_strs(env, &all)?;
            let args = bazel::ImmutableList::copy_of::<bazel::JavaString>(env, &args)?;
            Ok(self
                .loading
                .skyframe
                .get(env)?
                .create_build_options_for_testing(handler, &args)?
                .to_global()?)
        })?;
        Ok(EngineConfig {
            half,
            parser,
            options,
        })
    }

    /// Configures and analyzes the targets of `spec` in `config`: the steps of
    /// `BuildView.update` for a build that stops after analysis, driven on the
    /// `SkyframeExecutor` and `SkyframeBuildView`. One entry per requested
    /// target, in order: its configured target, or why it did not analyze.
    pub fn analyze(&self, spec: &[Spec], config: &EngineConfig) -> Result<Vec<Analyzed>> {
        self.synced(
            &config.parser,
            &config.half,
            "build",
            false,
            |env, handler| {
                let targets = self.resolve(env, handler, spec, config)?;
                let core = self.analysis_core(env, handler, &targets, config)?;
                let n = core.labels.len();
                let mut out = Vec::with_capacity(n);
                for i in 0..n {
                    let position = core.position(i);
                    out.push(Analyzed {
                        label: core.labels[i].clone(),
                        target: match position {
                            Some(p) => Some(
                                core.configured
                                    .get(env)?
                                    .upcast::<bazel::List<bazel::ConfiguredTarget>>()
                                    .get(p)?
                                    .to_global()?,
                            ),
                            None => None,
                        },
                        failure: core.failures.get(&i).cloned(),
                    });
                }
                Ok(out)
            },
        )
    }

    /// The targets a script named, as one Java collection: each pattern string
    /// evaluated as a query in the build's own synced graph, each live target
    /// as it is. A single pattern is the query's own result set, untouched.
    pub(crate) fn resolve(
        &self,
        env: &mut JNIEnv<'_>,
        handler: &bazel::ExtendedEventHandler,
        spec: &[Spec],
        config: &EngineConfig,
    ) -> Result<Global<bazel::Collection<'static, bazel::Target<'static>>>> {
        let _resolving = crate::phases::start("resolve targets");
        let l = &self.loading;
        let runtime = self.runtime.get(env)?;
        let skyframe = l.skyframe.get(env)?;
        let options = config.parser.get(env)?;
        type Found = Global<bazel::Collection<'static, bazel::Target<'static>>>;
        let pattern = |env: &mut JNIEnv<'_>, expr: &str| -> Result<Found> {
            let set = evaluate(env, &runtime, &skyframe, handler, &options, l, expr)?;
            Ok(set
                .get(env)?
                .downcast::<bazel::Collection<bazel::Target>>()?
                .to_global()?)
        };
        if let [Spec::Pattern(expr)] = spec {
            let found = pattern(env, expr)?;
            if found.get(env)?.is_empty()? {
                bail!("no targets given");
            }
            return Ok(found);
        }
        let all = bazel::List::<bazel::Target>::new(env)?;
        for item in spec {
            match item {
                Spec::Pattern(expr) => {
                    let found = pattern(env, expr)?.get(env)?;
                    for t in &found.upcast::<bazel::Iterable<bazel::Target>>() {
                        all.add(&t?)?;
                    }
                }
                Spec::Live(t) => {
                    all.add(&t.get(env)?.downcast::<bazel::Target>()?)?;
                }
            }
        }
        let all = all.upcast::<bazel::Collection<bazel::Target>>();
        if all.is_empty()? {
            bail!("no targets given");
        }
        Ok(all.to_global()?)
    }

    /// `BuildView.update` up to and including `SkyframeBuildView.configureTargets`,
    /// with `keep_going`: a target whose analysis fails does not stop the others,
    /// and its `ErrorInfo` becomes its failure. The configured target keys of
    /// an unchanged target set in an unchanged configuration are reused.
    pub(crate) fn analysis_core<'l>(
        &self,
        env: &mut JNIEnv<'l>,
        handler: &bazel::ExtendedEventHandler,
        targets: &Global<bazel::Collection<'static, bazel::Target<'static>>>,
        config: &EngineConfig,
    ) -> Result<Core<'l>> {
        let l = &self.loading;
        let skyframe = l.skyframe.get(env)?;
        let options = config.options.get(env)?;
        let parser = config.parser.get(env)?;
        let artifact_context = top_level_artifact_context(env, &parser)?;

        let configuration = crate::phases::start("analysis: configuration");
        skyframe.set_baseline_configuration(&options, handler)?;
        let view = skyframe.skyframe_build_view()?;
        let discard = view.should_discard_analysis_cache(
            handler,
            &options,
            MAX_CONFIG_CHANGES_TO_SHOW,
            true,
            None::<&bazel::Object>,
        )?;
        let top = skyframe.create_configuration(handler, &options, false)?;
        view.set_configuration(&top, &options, discard)?;
        drop(configuration);

        let keys_phase = crate::phases::start("analysis: target keys");
        let top_key = top.key()?;
        let collection = targets.get(env)?;
        let reusable = {
            let cache = self
                .keys_cache
                .lock()
                .map_err(|_| anyhow::anyhow!("keys cache lock poisoned"))?;
            match &*cache {
                Some(c) if c.targets.get(env)? == collection && c.top_key.get(env)? == top_key => {
                    Some(c.clone())
                }
                _ => None,
            }
        };
        let keys = match reusable {
            Some(k) => k,
            None => {
                let by_label = bazel::Map::<bazel::Label, bazel::Target>::new(env)?;
                let keys = bazel::List::<bazel::ConfiguredTargetKey>::new(env)?;
                let mut labels = Vec::new();
                for t in &collection.upcast::<bazel::Iterable<bazel::Target>>() {
                    let target = t?;
                    let label = target.label()?;
                    by_label.put(&label, &target)?;
                    keys.add(
                        &bazel::ConfiguredTargetKey::builder(env)?
                            .set_label(&label)?
                            .set_configuration_key(Some(&top_key))?
                            .build()?,
                    )?;
                    labels.push(label.to_global()?);
                }
                let built = KeysCache {
                    targets: targets.clone(),
                    top_key: top_key.to_global()?,
                    by_label: bazel::ImmutableMap::copy_of(env, &by_label)?.to_global()?,
                    keys: bazel::ImmutableList::copy_of(env, &keys)?.to_global()?,
                    labels: Arc::new(labels),
                };
                *self
                    .keys_cache
                    .lock()
                    .map_err(|_| anyhow::anyhow!("keys cache lock poisoned"))? =
                    Some(built.clone());
                built
            }
        };
        drop(keys_phase);
        let recorded = bazel::StoredEventHandler::new(env)?;
        let reporter = l.reporter.get(env)?;
        reporter.add_handler(&recorded)?;
        let configure_targets = crate::phases::start("analysis: configureTargets");
        let configured = view.configure_targets(
            handler,
            &keys.by_label.get(env)?,
            &keys.keys.get(env)?,
            &bazel::ImmutableList::empty::<bazel::TopLevelAspectsKey>(env)?,
            &artifact_context,
            &l.event_bus.get(env)?,
            &self.runtime.get(env)?.bug_reporter()?,
            true,
            &l.executors.get(env)?,
            true,
        );
        reporter.remove_handler(&recorded)?;
        drop(configure_targets);
        let _collect = crate::phases::start("analysis: collect results");
        let result = configured?;
        if result.has_action_conflicts()? {
            bail!("analysis found action conflicts; see the errors above");
        }
        let requested = keys.labels.len();
        let analyzed = result.configured_targets()?;
        let list = bazel::ImmutableList::copy_of::<bazel::ConfiguredTarget>(
            env,
            &analyzed.upcast::<bazel::Collection<bazel::ConfiguredTarget>>(),
        )?;
        let plain = list.upcast::<bazel::List<bazel::ConfiguredTarget>>();
        if plain.len()? == requested {
            // Every target analyzed, in the order of the keys.
            return Ok(Core {
                labels: keys.labels.clone(),
                configured: list.to_global()?,
                positions: None,
                failures: HashMap::new(),
                roots: result.package_roots()?,
            });
        }
        // Some did not: match the configured targets to the requested ones by label.
        let mut by_name: HashMap<String, usize> = HashMap::new();
        for i in 0..plain.len()? {
            by_name.insert(plain.get(i)?.label()?.to_string(), i);
        }
        let mut positions = Vec::with_capacity(requested);
        let mut failed = Vec::new();
        let failed_keys = bazel::List::<bazel::SkyKey>::new(env)?;
        for (i, label) in keys.labels.iter().enumerate() {
            match by_name.get(&label.get(env)?.to_string()) {
                Some(p) => positions.push(Some(*p)),
                None => {
                    positions.push(None);
                    failed.push(i);
                    failed_keys.add(
                        &keys
                            .keys
                            .get(env)?
                            .upcast::<bazel::List<bazel::ConfiguredTargetKey>>()
                            .get(i)?
                            .upcast::<bazel::SkyKey>(),
                    )?;
                }
            }
        }
        let mut failures = HashMap::new();
        if !failed.is_empty() {
            // The failed keys hold their errors in the graph, so asking again
            // reads them (with events silenced: they were reported once).
            let silent: bazel::ExtendedEventHandler =
                bazel::NullEventHandler::instance(env)?.upcast();
            let evaluated = skyframe.evaluate_sky_keys(&silent, &failed_keys, true)?;
            let mut errors = Vec::new();
            for e in &recorded.events()?.upcast::<bazel::Iterable<bazel::Event>>() {
                let e = e?;
                if e.kind()?.to_string() == "ERROR" {
                    errors.push(e.message()?);
                }
            }
            for i in failed {
                let key = keys
                    .keys
                    .get(env)?
                    .upcast::<bazel::List<bazel::ConfiguredTargetKey>>()
                    .get(i)?;
                let label = keys.labels[i].get(env)?.to_string();
                let about: Vec<&str> = errors
                    .iter()
                    .filter(|m| m.contains(&label))
                    .map(String::as_str)
                    .collect();
                let cause = match evaluated.get_error(&key.upcast::<bazel::SkyKey>())? {
                    Some(e) => failure_message(&e)?,
                    None => String::new(),
                };
                failures.insert(
                    i,
                    if !about.is_empty() {
                        about.join("\n")
                    } else if cause.is_empty() {
                        "analysis failed; see the errors above".to_string()
                    } else {
                        cause
                    },
                );
            }
        }
        Ok(Core {
            labels: keys.labels.clone(),
            configured: list.to_global()?,
            positions: Some(positions),
            failures,
            roots: result.package_roots()?,
        })
    }

    /// The configuration of no flags, parsed once.
    pub fn default_config(&self) -> Result<Arc<EngineConfig>> {
        let mut slot = self
            .default_config
            .lock()
            .map_err(|_| anyhow::anyhow!("configuration lock poisoned"))?;
        if let Some(c) = slot.as_ref() {
            return Ok(c.clone());
        }
        let c = Arc::new(self.config(&[], &[], &[], Half::default())?);
        *slot = Some(c.clone());
        Ok(c)
    }

    /// Evaluates a query expression on the heap and returns the live result
    /// set, the query's own aggregate of `Target`s in label order.
    pub fn query(&self, expr: &str) -> Result<Live> {
        self.query_synced(&self.loading.options, &Half::default(), expr)
    }

    /// `query`, with the graph synced to `config` (its repository environment
    /// and its loading options), as the targets of a build in `config` are.
    pub fn query_in(&self, expr: &str, config: &EngineConfig) -> Result<Live> {
        self.query_synced(&config.parser, &config.half, expr)
    }

    fn query_synced(
        &self,
        parser: &Global<bazel::OptionsParser<'static>>,
        half: &Half,
        expr: &str,
    ) -> Result<Live> {
        self.synced(parser, half, "query", false, |env, handler| {
            let l = &self.loading;
            let runtime = self.runtime.get(env)?;
            let skyframe = l.skyframe.get(env)?;
            let options = parser.get(env)?;
            evaluate(env, &runtime, &skyframe, handler, &options, l, expr)
        })
    }

    /// The live `Package` for `//pkg`, loaded on demand.
    pub fn package(&self, label: &str) -> Result<Live> {
        self.synced(
            &self.loading.options,
            &Half::default(),
            "query",
            false,
            |env, _| {
                let manager: bazel::PackageProvider =
                    self.loading.skyframe.get(env)?.package_manager()?.upcast();
                let handler = bazel::NullEventHandler::instance(env)?;
                let id = bazel::PackageIdentifier::parse(env, label.trim_start_matches("//"))?;
                Ok(manager
                    .get_package(&handler, &id)?
                    .upcast::<Object>()
                    .to_global()?)
            },
        )
    }

    /// What every command begins and ends with on the executor: module values
    /// injected and package loading synced, then `f`, then the completion
    /// notice. Nothing else of a command is run.
    fn synced<T>(
        &self,
        options: &Global<bazel::OptionsParser<'static>>,
        half: &Half,
        command: &str,
        executes: bool,
        f: impl FnOnce(&mut JNIEnv<'_>, &bazel::ExtendedEventHandler) -> Result<T>,
    ) -> Result<T> {
        let mut env = vm::env()?;
        let env = &mut env;
        let l = &self.loading;
        let skyframe = l.skyframe.get(env)?;
        let handler: bazel::ExtendedEventHandler = l.reporter.get(env)?.upcast();
        let prelude = crate::phases::start("sync: apply + inject + sync");
        self.ensure_applied(env, options, half)?;
        inject_module_values(&self.runtime.get(env)?, &skyframe)?;
        if self
            .applied
            .lock()
            .map_err(|_| anyhow::anyhow!("engine half lock poisoned"))?
            .ignore_dev_deps
        {
            inject_ignore_dev_deps(env, &skyframe)?;
        }
        let locator = l
            .package_locator
            .lock()
            .map_err(|_| anyhow::anyhow!("package locator lock poisoned"))?
            .get(env)?;
        skyframe.sync(
            &handler,
            &locator,
            &bazel::UUID::random(env)?,
            &l.client_env.get(env)?,
            &l.tsgm.get(env)?,
            &l.executors.get(env)?,
            &options.get(env)?,
            command,
            executes,
        )?;
        drop(prelude);
        let body = crate::phases::start("call body");
        let outcome = f(env, &handler);
        drop(body);
        let _after = crate::phases::start("sync: notify + after_command");
        let notified = skyframe.notify_command_complete(&handler);
        let result = outcome?;
        notified?;
        if let Some(module) = &l.lockfile_module {
            module
                .get(env)?
                .upcast::<bazel::BlazeModule>()
                .after_command()?;
        }
        Ok(result)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Ok(mut env) = vm::env() {
            if let Ok(runtime) = self.runtime.get(&mut env) {
                let _ = runtime.shutdown();
            }
        }
        let _ = &self.jvm;
    }
}

/// Builds the query environment the way `QueryEnvironmentBasedCommand` does,
/// from the executor rather than from a command, and collects the result on
/// the heap with the same aggregating callback cquery uses.
fn evaluate(
    env: &mut JNIEnv<'_>,
    runtime: &bazel::BlazeRuntime,
    skyframe: &bazel::SkyframeExecutor,
    handler: &bazel::ExtendedEventHandler,
    options: &bazel::OptionsParser,
    l: &Loading,
    expr: &str,
) -> Result<Live> {
    let repo_mapping = skyframe.get_main_repo_mapping(handler)?;
    let relative = l.relative_working_directory.get(env)?;
    let main_repo = bazel::RepositoryName::main(env)?;
    let parser = bazel::TargetPatternParser::new(env, &relative, &main_repo, &repo_mapping)?;
    let package_manager = skyframe.package_manager()?;
    let walkable = bazel::SkyframeExecutorWrappingWalkableGraph::of(env, skyframe)?;
    let target_provider =
        bazel::TargetProviderForQueryEnvironment::new(env, &walkable, &package_manager)?;
    let pattern_preloader = bazel::SkyframeTargetPatternEvaluator::new(env, skyframe)?;
    let query_options =
        options.get_options(&class::<bazel::CommonQueryOptions>(env, QUERY_OPTIONS)?)?;
    let language = options.get_options(&class::<bazel::BuildLanguageOptions>(
        env,
        LANGUAGE_OPTIONS,
    )?)?;
    let semantics = skyframe.get_effective_starlark_semantics(&language)?;
    let label_printer = query_options.get_label_printer(&semantics, &repo_mapping)?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4);
    let qenv = runtime.query_environment_factory()?.create(
        Some(&skyframe.query_transitive_package_preloader()?),
        skyframe,
        &target_provider,
        &package_manager,
        &pattern_preloader,
        &parser,
        &relative,
        false,
        true,
        true,
        &bazel::UniverseScope::empty(env)?,
        threads,
        &bazel::Rule::all_labels(env)?,
        handler,
        &query_options.to_settings()?,
        &runtime.query_functions()?,
        Some(&package_manager.package_path()?),
        false,
        &label_printer,
    )?;
    let evaluated = (|| {
        let parsed = bazel::QueryExpression::parse(env, expr, &qenv)?;
        let parsed = qenv.transform_parsed_query(&parsed)?;
        let callback =
            bazel::QueryUtil::new_ordered_aggregate_all_output_formatter_callback(env, &qenv)?;
        if !qenv.evaluate_query(&parsed, &callback)?.success()? {
            bail!("query {expr:?} did not complete; see the errors above");
        }
        Ok(callback
            .downcast::<bazel::AggregateAllCallback<bazel::Target, bazel::Set<bazel::Target>>>()?
            .result()?
            .upcast::<Object>()
            .to_global()?)
    })();
    let closed = qenv.close();
    let live = evaluated?;
    closed?;
    Ok(live)
}

/// An `OptionsParser` over every option class a `build` command reads (the
/// modules' common options, the loading and query classes, the build command's
/// own and the rule fragments'), parsed with `flags` and nothing else: no rc
/// files, no residue.
fn parse_flags<'l>(
    env: &mut JNIEnv<'l>,
    runtime: &bazel::BlazeRuntime,
    flags: &[String],
) -> Result<bazel::OptionsParser<'l>> {
    let classes = bazel::Set::<bazel::Class<OptionsBase>>::new(env)?;
    let common = bazel::BlazeCommandUtils::get_common_options(
        env,
        &runtime
            .blaze_modules()?
            .downcast::<bazel::Iterable<bazel::OptionsSupplier>>()?,
    )?;
    for c in &common.upcast::<bazel::Iterable<bazel::Class<OptionsBase>>>() {
        classes.add(&c?)?;
    }
    for name in [PACKAGE_OPTIONS, LANGUAGE_OPTIONS, QUERY_OPTIONS]
        .into_iter()
        .chain(BUILD_COMMAND_OPTIONS)
    {
        classes.add(&class::<OptionsBase>(env, name)?)?;
    }
    let fragments = runtime
        .rule_class_provider()?
        .fragment_registry()?
        .options_classes()?;
    for c in &fragments.upcast::<bazel::Iterable<bazel::Class<bazel::FragmentOptions>>>() {
        classes.add(&c?.downcast::<bazel::Class<OptionsBase>>()?)?;
    }
    let parser = bazel::OptionsParser::builder(env)?
        .options_classes(&classes)?
        .build()?;
    parser.parse(&bazel::List::of_strs(env, flags)?)?;
    Ok(parser)
}

const MAX_CONFIG_CHANGES_TO_SHOW: i32 = 3;

/// `BuildRequest.getTopLevelArtifactContext` from the parsed options.
pub(crate) fn top_level_artifact_context<'l>(
    env: &mut JNIEnv<'l>,
    options: &bazel::OptionsParser,
) -> Result<bazel::TopLevelArtifactContext<'l>> {
    let request = options.get_options(&class::<bazel::BuildRequestOptions>(
        env,
        "com/google/devtools/build/lib/buildtool/BuildRequestOptions",
    )?)?;
    let execution = options.get_options(&class::<bazel::ExecutionOptions>(
        env,
        "com/google/devtools/build/lib/exec/ExecutionOptions",
    )?)?;
    let protocol = options.get_options(&class::<bazel::BuildEventProtocolOptions>(
        env,
        "com/google/devtools/build/lib/buildeventstream/BuildEventProtocolOptions",
    )?)?;
    let validation = if !request.run_validation_actions()? {
        bazel::ValidationMode::off(env)?
    } else if request.use_validation_aspect()? {
        bazel::ValidationMode::aspect(env)?
    } else {
        bazel::ValidationMode::output_group(env)?
    };
    let groups = bazel::OutputGroupInfo::determine_output_groups(
        env,
        &request.output_groups()?,
        &validation,
        false,
    )?;
    Ok(bazel::TopLevelArtifactContext::new(
        env,
        execution.test_strategy()? == "exclusive",
        protocol.expand_filesets()?,
        &groups,
    )?)
}

/// `Class` of an option class, found by its internal name.
pub(crate) fn class<'l, T>(env: &mut JNIEnv<'l>, name: &str) -> Result<bazel::Class<'l, T>> {
    Ok(bazel::Class::find(env, name)?)
}

/// What `BlazeRuntime.main` does before it would start a gRPC server or exit:
/// modules, then `newRuntime`.
fn boot<'l>(env: &mut JNIEnv<'l>, startup: &[String]) -> Result<bazel::BlazeRuntime<'l>> {
    let module_classes = bazel::Bazel::bazel_modules(env)?;
    let modules = bazel::BlazeRuntime::create_blaze_modules(env, &module_classes)?;
    let services = bazel::Bazel::bazel_services(env)?;
    let args = bazel::List::of_strs(env, startup)?;
    Ok(bazel::BlazeRuntime::new_runtime(
        env,
        &modules,
        &services,
        &args,
        &bazel::Runnable::null(),
    )?)
}

/// The inputs `CommandEnvironment` would hand to `SkyframeExecutor.sync`,
/// built from the runtime and the workspace instead.
fn prepare_loading(
    env: &mut JNIEnv<'_>,
    runtime: &bazel::BlazeRuntime,
    relative_working_directory: &str,
) -> Result<Loading> {
    let workspace = runtime.workspace()?;
    let skyframe = workspace.skyframe_executor()?;
    let event_bus = bazel::EventBus::new(env)?;
    skyframe.set_event_bus(Some(&event_bus))?;
    let reporter = bazel::Reporter::new(env, &event_bus)?;
    let out_err = bazel::OutErr::system_out_err(env)?;
    let kinds = bazel::EventKind::errors_and_warnings(env)?;
    let printing = bazel::PrintingEventHandler::new(env, &out_err, &kinds)?;
    reporter.add_handler(&printing)?;

    let options = parse_flags(env, runtime, &[])?;

    let package_options =
        options.get_options(&class::<bazel::PackageOptions>(env, PACKAGE_OPTIONS)?)?;
    let workspace_path = workspace.workspace()?;
    let null_handler = bazel::NullEventHandler::instance(env)?;
    let package_locator = bazel::PathPackageLocator::create(
        env,
        &workspace.output_base()?,
        &package_options.package_path()?,
        &null_handler,
        &workspace_path.as_fragment()?,
        &workspace_path,
        &skyframe.build_files_by_priority()?,
    )?;

    let relative = if relative_working_directory.is_empty() {
        bazel::PathFragment::empty_fragment(env)?
    } else {
        bazel::PathFragment::create(env, relative_working_directory)?
    };
    seed_repository_module(env, runtime, &reporter, &options)?;
    Ok(Loading {
        skyframe: skyframe.to_global()?,
        package_locator: Mutex::new(package_locator.to_global()?),
        options: options.to_global()?,
        tsgm: bazel::TimestampGranularityMonitor::new(env, &runtime.clock()?)?.to_global()?,
        executors: bazel::QuiescingExecutorsImpl::for_testing(env)?.to_global()?,
        reporter: reporter.to_global()?,
        event_bus: event_bus.to_global()?,
        lockfile_module: seed_lockfile_module(runtime, &skyframe, &options, env)?,
        client_env: client_env(env, false)?.to_global()?,
        relative_working_directory: relative.to_global()?,
    })
}

fn strict_repo_env(env: &mut JNIEnv<'_>, options: &bazel::OptionsParser) -> Result<bool> {
    Ok(options
        .get_options(&class::<bazel::CommonCommandOptions>(env, COMMON_OPTIONS)?)?
        .use_strict_repo_env()?)
}

/// The repository environment of a command: the client environment (only
/// `PATH` when strict) with the `Config`'s `repo_env` assignments on top, for
/// the repository rules and the non-strict one for credential helpers.
/// `CommandEnvironment` computes the same from `--repo_env`.
fn apply_repo_env(
    env: &mut JNIEnv<'_>,
    runtime: &bazel::BlazeRuntime,
    strict: bool,
    assignments: &[(String, String)],
) -> Result<()> {
    let Some(module) = find_module::<BazelRepositoryModule>(runtime)? else {
        return Ok(());
    };
    for (supplier, only_path) in [
        (module.repo_env_supplier()?, strict),
        (module.nonstrict_repo_env_supplier()?, false),
    ] {
        let map = bazel::Map::<bazel::JavaString, bazel::JavaString>::new(env)?;
        for (k, v) in std::env::vars_os() {
            let (Ok(k), Ok(v)) = (k.into_string(), v.into_string()) else {
                continue;
            };
            if only_path && k != "PATH" {
                continue;
            }
            map.put(
                &bazel::JavaString::new(env, &k)?,
                &bazel::JavaString::new(env, &v)?,
            )?;
        }
        for (k, v) in assignments {
            map.put(
                &bazel::JavaString::new(env, k)?,
                &bazel::JavaString::new(env, v)?,
            )?;
        }
        supplier.set(&bazel::ImmutableMap::copy_of(env, &map)?)?;
    }
    Ok(())
}

/// Each module's precomputed values (repository overrides, registries, the
/// vendor directory), which the dispatcher injects before every command and
/// package loading reads: `IgnoredSubdirectoriesFunction` fails without them.
fn inject_module_values(
    runtime: &bazel::BlazeRuntime,
    skyframe: &bazel::SkyframeExecutor,
) -> Result<()> {
    for module in &runtime.blaze_modules()? {
        let values = module?.precomputed_values()?;
        skyframe.inject_extra_precomputed_values(&values)?;
    }
    Ok(())
}

/// `BazelRepositoryModule.beforeCommand` fills in what its functions and its
/// `getPrecomputedValues` read, and takes a `CommandEnvironment`, which this
/// engine has none of. The same values are built here from the option
/// defaults (`options` is parsed with no arguments) and the process
/// environment: repo env, the `DownloadManager` with its `HttpDownloader` and
/// retry, hardlink and timeout settings, the repository cache under the
/// output user root, the process wrapper and syscall cache of the fetch
/// functions, and the module's defaults for registries and mirrors.
fn seed_repository_module(
    env: &mut JNIEnv<'_>,
    runtime: &bazel::BlazeRuntime,
    reporter: &bazel::Reporter,
    options: &bazel::OptionsParser,
) -> Result<()> {
    let Some(module) = find_module::<BazelRepositoryModule>(runtime)? else {
        return Ok(());
    };
    let common =
        options.get_options(&class::<bazel::CommonCommandOptions>(env, COMMON_OPTIONS)?)?;

    module.set_clock(Some(&runtime.clock()?))?;
    module.set_registries(Some(&BazelRepositoryModule::default_registries(env)?))?;
    module.set_module_mirrors(Some(&BazelRepositoryModule::default_module_mirrors(env)?))?;

    let strict = common.use_strict_repo_env()?;
    let client = client_env(env, false)?;
    let repo_env = client_env(env, strict)?;
    module.repo_env_supplier()?.set(&repo_env)?;
    module.nonstrict_repo_env_supplier()?.set(&client)?;

    let cache = module.repository_cache()?;
    let user_root = runtime
        .workspace()?
        .output_base()?
        .parent_directory()?
        .ok_or_else(|| anyhow::anyhow!("the output base has no parent directory"))?;
    cache.set_path(Some(&user_root.get_relative("cache/repos/v1")?))?;
    if cache.repo_contents_cache()?.is_enabled()? {
        cache.repo_contents_cache()?.acquire_shared_lock()?;
    }

    apply::install_download_manager(env, runtime, reporter, options, &module)?;
    Ok(())
}

/// The runtime's module of class `T`, when it has one.
fn find_module<'l, T: blazon_rt::Handle<'l>>(
    runtime: &bazel::BlazeRuntime<'l>,
) -> Result<Option<T>> {
    for m in &runtime.blaze_modules()? {
        let m = m?;
        if m.is_a::<T>()? {
            return Ok(Some(m.downcast::<T>()?));
        }
    }
    Ok(None)
}

/// `BazelLockFileModule.beforeCommand` records what its `afterCommand` needs
/// to write `MODULE.bazel.lock` after module resolution, as a command does.
fn seed_lockfile_module(
    runtime: &bazel::BlazeRuntime,
    skyframe: &bazel::SkyframeExecutor,
    options: &bazel::OptionsParser,
    env: &mut JNIEnv<'_>,
) -> Result<Option<Global<bazel::BazelLockFileModule<'static>>>> {
    let Some(module) = find_module::<bazel::BazelLockFileModule>(runtime)? else {
        return Ok(None);
    };
    let workspace = runtime.workspace()?;
    let repo = options.get_options(&class::<bazel::RepositoryOptions>(env, REPOSITORY_OPTIONS)?)?;
    module.set_executor(skyframe)?;
    module.set_workspace_root(&workspace.workspace()?)?;
    module.set_output_base(&workspace.output_base()?)?;
    module.set_options_lockfile_mode(&repo.lockfile_mode()?)?;
    Ok(Some(module.to_global()?))
}

/// `ProcessWrapper.fromCommandEnvironment` with the defaults it uses when the
/// command has no local execution options: no kill delay, no graceful SIGTERM.
/// None when the release ships no `process-wrapper`.
fn process_wrapper<'l>(
    env: &mut JNIEnv<'l>,
    runtime: &bazel::BlazeRuntime,
) -> Result<Option<bazel::ProcessWrapper<'l>>> {
    const BINARY: &str = "process-wrapper";
    let tools = runtime.workspace()?.bin_tools()?;
    let (Some(input), Some(path)) = (
        tools.get_action_input(BINARY)?,
        tools.get_embedded_path(BINARY)?,
    ) else {
        return Ok(None);
    };
    Ok(Some(bazel::ProcessWrapper::new(
        env,
        &path.as_fragment()?,
        &input,
        None::<&bazel::Duration>,
        false,
    )?))
}

/// The process environment as the `ImmutableMap` Bazel's client would have
/// sent as `--client_env`; with `only_path`, just what a strict repo env
/// inherits.
fn client_env<'l>(
    env: &mut JNIEnv<'l>,
    only_path: bool,
) -> Result<bazel::ImmutableMap<'l, bazel::JavaString<'static>, bazel::JavaString<'static>>> {
    let map = bazel::Map::<bazel::JavaString, bazel::JavaString>::new(env)?;
    for (k, v) in std::env::vars_os() {
        let (Ok(k), Ok(v)) = (k.into_string(), v.into_string()) else {
            continue;
        };
        if only_path && k != "PATH" {
            continue;
        }
        map.put(
            &bazel::JavaString::new(env, &k)?,
            &bazel::JavaString::new(env, &v)?,
        )?;
    }
    Ok(bazel::ImmutableMap::copy_of(env, &map)?)
}

fn unset() -> anyhow::Error {
    anyhow::anyhow!("BazelRepositoryModule has not created its functions")
}

fn jvm_options(o: &EngineOptions, logprops: &std::path::Path) -> Vec<String> {
    let jh = o.install.java_home.display();
    vec![
        format!("-Djava.class.path={}", o.install.server_jar.display()),
        format!("-Xmx{}", o.heap),
        "--add-opens=java.base/java.lang=ALL-UNNAMED".into(),
        "--add-opens=java.base/java.lang.invoke=ALL-UNNAMED".into(),
        "--add-exports=java.base/jdk.internal.misc=ALL-UNNAMED".into(),
        "--add-exports=java.base/jdk.internal.vm=ALL-UNNAMED".into(),
        "--add-exports=java.base/sun.nio.ch=ALL-UNNAMED".into(),
        "-XX:-MaxFDLimit".into(),
        "-Djava.lang.Thread.allowVirtualThreads=true".into(),
        "-Dsun.misc.unsafe.memory.access=allow".into(),
        "-Dfile.encoding=ISO-8859-1".into(),
        "-Duser.country=".into(),
        "-Duser.language=".into(),
        "-Duser.variant=".into(),
        "-Djdk.nio.file.WatchService.maxEventsPerPoll=10000".into(),
        format!("-Djava.library.path={jh}/lib:{jh}/lib/server:{}", o.install.root.display()),
        format!("-Djava.util.logging.config.file={}", logprops.display()),
        "-Dcom.google.devtools.build.lib.util.LogHandlerQuerier.class=com.google.devtools.build.lib.util.SimpleLogHandler$HandlerQuerier".into(),
    ]
}

/// Only what `BlazeRuntime.newRuntime` requires. No rc files reach the engine:
/// the client is what expands them into `--default_override`, and there is no client.
fn startup_options(o: &EngineOptions) -> Vec<String> {
    vec![
        "--batch".into(),
        format!("--install_base={}", o.install.root.display()),
        format!("--output_base={}", o.output_base.display()),
        format!("--output_user_root={}", o.output_user_root.display()),
        format!("--workspace_directory={}", o.workspace.display()),
        format!(
            "--failure_detail_out={}",
            o.output_base.join("failure_detail.rawproto").display()
        ),
        format!(
            "--default_system_javabase={}",
            o.install.java_home.display()
        ),
        "--product_name=Bazel".into(),
    ]
}

fn javalog(log: &std::path::Path) -> String {
    format!(
        "handlers=com.google.devtools.build.lib.util.SimpleLogHandler\n\
         .level=INFO\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.level=INFO\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.prefix={}\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.limit=1024000\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.total_limit=20971520\n\
         com.google.devtools.build.lib.util.SimpleLogHandler.formatter=com.google.devtools.build.lib.util.SingleLineFormatter\n",
        log.display()
    )
}

/// What an `ErrorInfo` says: its exception's message, then each cause's
/// message that differs, outermost first; the `ErrorInfo` text when it has no
/// exception (a cycle).
pub(crate) fn failure_message(info: &bazel::ErrorInfo) -> Result<String> {
    let mut messages: Vec<String> = Vec::new();
    let mut next = info
        .exception()?
        .map(|e| e.upcast::<blazon_rt::java::lang::Throwable>());
    while let Some(t) = next {
        if let Some(m) = t.message()? {
            let m = m.trim().to_string();
            if !m.is_empty() && !messages.iter().any(|x| x.contains(&m)) {
                messages.push(m);
            }
        }
        next = t.cause()?;
    }
    Ok(if messages.is_empty() {
        info.to_string()
    } else {
        messages.join(" <- ")
    })
}

/// `ignore_dev_dependency` as a precomputed value, injected after the
/// repository module's own (which holds its `AtomicBoolean` at false): Skyframe
/// keeps the last injection of a key before an evaluation, so the pair costs
/// nothing when the value did not change.
fn inject_ignore_dev_deps(env: &mut JNIEnv<'_>, skyframe: &bazel::SkyframeExecutor) -> Result<()> {
    let key = bazel::ModuleFileFunction::ignore_dev_deps(env)?;
    let on = blazon_rt::java::lang::Boolean::of(env, true)?;
    let injected = bazel::PrecomputedValue::injected(env, &key, &on)?;
    let list = bazel::List::<bazel::InjectedPrecomputedValue>::new(env)?;
    list.add(&injected)?;
    skyframe.inject_extra_precomputed_values(&list)?;
    Ok(())
}
