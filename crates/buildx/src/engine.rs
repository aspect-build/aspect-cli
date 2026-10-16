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
use std::fs;
use std::path::PathBuf;

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
    package_locator: Global<bazel::PathPackageLocator<'static>>,
    /// An `OptionsParser` over every option class the engine reads during
    /// loading, parsed with no arguments, so defaults throughout.
    options: Global<bazel::OptionsParser<'static>>,
    tsgm: Global<bazel::TimestampGranularityMonitor<'static>>,
    executors: Global<bazel::QuiescingExecutors<'static>>,
    reporter: Global<bazel::Reporter<'static>>,
    lockfile_module: Option<Global<bazel::BazelLockFileModule<'static>>>,
    client_env: Global<
        bazel::ImmutableMap<'static, bazel::JavaString<'static>, bazel::JavaString<'static>>,
    >,
    relative_working_directory: Global<bazel::PathFragment<'static>>,
}

pub struct Engine {
    jvm: Jvm,
    runtime: Global<bazel::BlazeRuntime<'static>>,
    loading: Loading,
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

    /// Evaluates a query expression on the heap and returns the live result
    /// set, the query's own aggregate of `Target`s in label order.
    pub fn query(&self, expr: &str) -> Result<Live> {
        self.synced(|env, handler| {
            let l = &self.loading;
            let runtime = self.runtime.get(env)?;
            let skyframe = l.skyframe.get(env)?;
            let options = l.options.get(env)?;
            evaluate(env, &runtime, &skyframe, handler, &options, l, expr)
        })
    }

    /// The live `Package` for `//pkg`, loaded on demand.
    pub fn package(&self, label: &str) -> Result<Live> {
        self.synced(|env, _| {
            let manager: bazel::PackageProvider =
                self.loading.skyframe.get(env)?.package_manager()?.upcast();
            let handler = bazel::NullEventHandler::instance(env)?;
            let id = bazel::PackageIdentifier::parse(env, label.trim_start_matches("//"))?;
            Ok(manager
                .get_package(&handler, &id)?
                .upcast::<Object>()
                .to_global()?)
        })
    }

    /// What every command begins and ends with on the executor: module values
    /// injected and package loading synced, then `f`, then the completion
    /// notice. Nothing else of a command is run.
    fn synced<T>(
        &self,
        f: impl FnOnce(&mut JNIEnv<'_>, &bazel::ExtendedEventHandler) -> Result<T>,
    ) -> Result<T> {
        let mut env = vm::env()?;
        let env = &mut env;
        let l = &self.loading;
        let skyframe = l.skyframe.get(env)?;
        let handler: bazel::ExtendedEventHandler = l.reporter.get(env)?.upcast();
        inject_module_values(&self.runtime.get(env)?, &skyframe)?;
        skyframe.sync(
            &handler,
            &l.package_locator.get(env)?,
            &bazel::UUID::random(env)?,
            &l.client_env.get(env)?,
            &l.tsgm.get(env)?,
            &l.executors.get(env)?,
            &l.options.get(env)?,
            "query",
            false,
        )?;
        let outcome = f(env, &handler);
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

/// `Class` of an option class, found by its internal name.
fn class<'l, T>(env: &mut JNIEnv<'l>, name: &str) -> Result<bazel::Class<'l, T>> {
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
    for name in [PACKAGE_OPTIONS, LANGUAGE_OPTIONS, QUERY_OPTIONS] {
        classes.add(&class::<OptionsBase>(env, name)?)?;
    }
    let options = bazel::OptionsParser::builder(env)?
        .options_classes(&classes)?
        .build()?;
    options.parse(&bazel::List::of_strs::<&str>(env, &[])?)?;

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
        package_locator: package_locator.to_global()?,
        options: options.to_global()?,
        tsgm: bazel::TimestampGranularityMonitor::new(env, &runtime.clock()?)?.to_global()?,
        executors: bazel::QuiescingExecutorsImpl::for_testing(env)?.to_global()?,
        reporter: reporter.to_global()?,
        lockfile_module: seed_lockfile_module(runtime, &skyframe, &options, env)?,
        client_env: client_env(env, false)?.to_global()?,
        relative_working_directory: relative.to_global()?,
    })
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
    let repo = options.get_options(&class::<bazel::RepositoryOptions>(env, REPOSITORY_OPTIONS)?)?;

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
    cache
        .download_cache()?
        .set_hardlink(repo.use_hardlinks()?)?;
    if cache.repo_contents_cache()?.is_enabled()? {
        cache.repo_contents_cache()?.acquire_shared_lock()?;
    }

    let scaling = common.http_timeout_scaling()?;
    let http_scaling = if scaling > 0.0 { scaling as f32 } else { 1.0 };
    let http = bazel::HttpDownloader::new(
        env,
        common.http_connector_attempts()?,
        &common.http_connector_retry_max_timeout()?,
        common.http_max_parallel_downloads()?,
        http_scaling,
    )?;
    let delegating = bazel::DelegatingDownloader::new(env, &http)?;
    let manager =
        bazel::DownloadManager::new(env, &cache.download_cache()?, &delegating, &http, reporter)?;
    manager.set_disable_download(repo.disable_download()?)?;
    if repo.repository_downloader_retries()? >= 0 {
        manager.set_retries(repo.repository_downloader_retries()?)?;
    }

    let repo_scaling = repo.experimental_scale_timeouts()?;
    let repo_scaling = if repo_scaling > 0.0 {
        repo_scaling
    } else {
        1.0
    };
    let process_wrapper = process_wrapper(env, runtime)?;
    let fetch = module.repository_fetch_function()?.ok_or_else(unset)?;
    fetch.set_download_manager(&manager)?;
    fetch.set_timeout_scaling(repo_scaling)?;
    fetch.set_process_wrapper(process_wrapper.as_ref())?;
    fetch.set_syscall_cache(&bazel::SyscallCache::no_cache(env)?)?;
    let extension = module.single_extension_eval_function()?.ok_or_else(unset)?;
    extension.set_download_manager(&manager)?;
    extension.set_timeout_scaling(repo_scaling)?;
    extension.set_process_wrapper(process_wrapper.as_ref())?;
    module
        .module_file_function()?
        .ok_or_else(unset)?
        .set_download_manager(&manager)?;
    module
        .repo_spec_function()?
        .ok_or_else(unset)?
        .set_download_manager(&manager)?;
    module
        .yanked_versions_function()?
        .ok_or_else(unset)?
        .set_download_manager(&manager)?;
    module.vendor_command()?.set_download_manager(&manager)?;
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
