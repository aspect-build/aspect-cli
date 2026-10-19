//! The engine half of a `Config`: the settings that are not configuration
//! fragment options but feed Skyframe all the same, namely Starlark semantics
//! (read by `sync` through the parser), package loading, repositories and
//! bzlmod. A `BlazeCommand` gets them applied by `BazelRepositoryModule.beforeCommand`,
//! `BazelLockFileModule.beforeCommand` and the dispatcher's package locator;
//! here the same values are written from the `Config`'s parser, once per
//! change.

use super::*;

/// What distinguishes one engine half from another, cheaply: `fingerprint` is
/// compared against the half last applied, and equal means nothing is done.
#[derive(Clone, Debug, Default)]
pub struct Half {
    pub fingerprint: String,
    /// `repo_env` assignments over the repository environment.
    pub repo_env: Vec<(String, String)>,
}

/// What an engine half applied, beyond what the modules' fields hold.
pub(super) struct Applied {
    pub fingerprint: String,
    /// `ignore_dev_dependency`: its module field is an `AtomicBoolean`, which
    /// is not bound, so it is injected after the module's own value each sync.
    pub ignore_dev_deps: bool,
}

impl Engine {
    /// Applies `half`, read from `parser`, when it differs from the half last
    /// applied. Equal halves cost one string comparison.
    pub(super) fn ensure_applied(
        &self,
        env: &mut JNIEnv<'_>,
        parser: &Global<bazel::OptionsParser<'static>>,
        half: &Half,
    ) -> Result<()> {
        let mut applied = self
            .applied
            .lock()
            .map_err(|_| anyhow::anyhow!("engine half lock poisoned"))?;
        if applied.fingerprint == half.fingerprint {
            return Ok(());
        }
        let l = &self.loading;
        let runtime = self.runtime.get(env)?;
        let parser = parser.get(env)?;
        let reporter = l.reporter.get(env)?;
        let workspace = runtime.workspace()?;
        let skyframe = l.skyframe.get(env)?;

        let repo =
            parser.get_options(&class::<bazel::RepositoryOptions>(env, REPOSITORY_OPTIONS)?)?;
        let package = parser.get_options(&class::<bazel::PackageOptions>(env, PACKAGE_OPTIONS)?)?;

        let ignore_dev_deps = match find_module::<BazelRepositoryModule>(&runtime)? {
            Some(module) => {
                install_download_manager(env, &runtime, &reporter, &parser, &module)?;
                apply_repository_options(env, &runtime, &module, &repo, &package)?;
                let strict = strict_repo_env(env, &parser)?;
                apply_repo_env(env, &runtime, strict, &half.repo_env)?;
                repo.ignore_dev_dependency()?
            }
            None => false,
        };
        if let Some(module) = &l.lockfile_module {
            module
                .get(env)?
                .set_options_lockfile_mode(&repo.lockfile_mode()?)?;
        }

        let workspace_path = workspace.workspace()?;
        let null_handler = bazel::NullEventHandler::instance(env)?;
        let locator = bazel::PathPackageLocator::create(
            env,
            &workspace.output_base()?,
            &package.package_path()?,
            &null_handler,
            &workspace_path.as_fragment()?,
            &workspace_path,
            &skyframe.build_files_by_priority()?,
        )?;
        *l.package_locator
            .lock()
            .map_err(|_| anyhow::anyhow!("package locator lock poisoned"))? =
            locator.to_global()?;

        applied.fingerprint = half.fingerprint.clone();
        applied.ignore_dev_deps = ignore_dev_deps;
        Ok(())
    }
}

/// An absolute path string for a setting's path: `%workspace%` expanded, a
/// relative path taken from the working directory, as
/// `BazelRepositoryModule.getAbsolutePath` does.
fn absolute_path(workspace: &str, path: &str) -> Result<String> {
    let path = path.replace("%workspace%", workspace);
    if path.starts_with('/') {
        return Ok(path);
    }
    Ok(format!("{}/{path}", std::env::current_dir()?.display()))
}

fn trim_slashes(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// `latest wins, empty path removes` over (name, path) assignments.
fn assignments(items: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (name, path) in items {
        out.retain(|(n, _)| *n != name);
        if !path.is_empty() {
            out.push((name, path));
        }
    }
    out
}

/// The part of `BazelRepositoryModule.beforeCommand` that reads the command's
/// options and fills the module's fields, which its `getPrecomputedValues`
/// then injects: overrides, injections, module overrides, registries and
/// mirrors, dependency checks, lockfile mode, vendor directory, yanked
/// versions, fetch, and the repository cache path.
fn apply_repository_options(
    env: &mut JNIEnv<'_>,
    runtime: &bazel::BlazeRuntime,
    module: &BazelRepositoryModule,
    repo: &bazel::RepositoryOptions,
    package: &bazel::PackageOptions,
) -> Result<()> {
    let workspace = runtime.workspace()?;
    let workspace_path = workspace.workspace()?;
    let workspace_string = workspace_path.path_string()?;

    module.set_fetch_disabled(!package.fetch()?)?;

    let mut overrides = Vec::new();
    for o in &repo
        .repository_overrides()?
        .upcast::<bazel::Iterable<bazel::RepositoryOverride>>()
    {
        let o = o?;
        overrides.push((o.repository_name()?, o.path()?));
    }
    let map = bazel::Map::<bazel::JavaString, bazel::PathFragment>::new(env)?;
    for (name, path) in assignments(overrides) {
        map.put(
            &bazel::JavaString::new(env, &name)?,
            &bazel::PathFragment::create(env, &absolute_path(&workspace_string, &path)?)?,
        )?;
    }
    module.set_overrides(&bazel::ImmutableMap::copy_of(env, &map)?)?;

    let mut injections = Vec::new();
    for o in &repo
        .repository_injections()?
        .upcast::<bazel::Iterable<bazel::RepositoryInjection>>()
    {
        let o = o?;
        injections.push((o.apparent_name()?, o.path()?));
    }
    let map = bazel::Map::<bazel::JavaString, bazel::PathFragment>::new(env)?;
    for (name, path) in assignments(injections) {
        map.put(
            &bazel::JavaString::new(env, &name)?,
            &bazel::PathFragment::create(env, &absolute_path(&workspace_string, &path)?)?,
        )?;
    }
    module.set_injections(&bazel::ImmutableMap::copy_of(env, &map)?)?;

    let mut module_overrides = Vec::new();
    for o in &repo
        .module_overrides()?
        .upcast::<bazel::Iterable<bazel::ModuleOptionOverride>>()
    {
        let o = o?;
        module_overrides.push((o.module_name()?, o.path()?));
    }
    let map = bazel::Map::<bazel::JavaString, bazel::ModuleOverride>::new(env)?;
    for (name, path) in assignments(module_overrides) {
        let spec =
            bazel::LocalPathRepoSpecs::create(env, &absolute_path(&workspace_string, &path)?)?;
        map.put(
            &bazel::JavaString::new(env, &name)?,
            &bazel::NonRegistryOverride::new(env, &spec)?.upcast::<bazel::ModuleOverride>(),
        )?;
    }
    module.set_module_overrides(&bazel::ImmutableMap::copy_of(env, &map)?)?;

    let mut requested = Vec::new();
    for r in &repo
        .registries()?
        .upcast::<bazel::Iterable<bazel::JavaString>>()
    {
        requested.push(trim_slashes(&r?.to_rust()?));
    }
    let registry_set = if requested.is_empty() {
        BazelRepositoryModule::default_registries(env)?
    } else {
        let list = bazel::List::<bazel::JavaString>::new(env)?;
        for r in &requested {
            list.add(&bazel::JavaString::new(env, r)?)?;
        }
        bazel::ImmutableSet::copy_of::<bazel::JavaString>(env, &list)?
    };
    module.set_registries(Some(&registry_set))?;
    let mut registries = Vec::new();
    for r in &registry_set.upcast::<bazel::Iterable<bazel::JavaString>>() {
        registries.push(r?.to_rust()?);
    }

    let mut mirrors: Vec<(String, Vec<String>)> = Vec::new();
    for e in &repo
        .module_mirrors()?
        .upcast::<bazel::Iterable<bazel::Entry<bazel::JavaString, bazel::List<bazel::JavaString>>>>(
        )
    {
        let e = e?;
        let mut urls = Vec::new();
        for u in &e.value()?.upcast::<bazel::Iterable<bazel::JavaString>>() {
            urls.push(trim_slashes(&u?.to_rust()?));
        }
        let key = trim_slashes(&e.key()?.to_rust()?);
        mirrors.retain(|(k, _)| *k != key);
        mirrors.push((key, urls));
    }
    if mirrors.is_empty() {
        module.set_module_mirrors(Some(&BazelRepositoryModule::default_module_mirrors(env)?))?;
    } else {
        for (k, _) in &mirrors {
            if !k.is_empty() && !registries.contains(k) {
                bail!("module_mirrors names the registry {k:?}, which is not in registries");
            }
        }
        let default = mirrors
            .iter()
            .find(|(k, _)| k.is_empty())
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let by_registry =
            bazel::Map::<bazel::JavaString, bazel::ImmutableSet<bazel::JavaString>>::new(env)?;
        for r in &registries {
            let urls = mirrors
                .iter()
                .find(|(k, _)| k == r)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| default.clone());
            let list = bazel::List::<bazel::JavaString>::new(env)?;
            for u in &urls {
                list.add(&bazel::JavaString::new(env, u)?)?;
            }
            by_registry.put(
                &bazel::JavaString::new(env, r)?,
                &bazel::ImmutableSet::copy_of::<bazel::JavaString>(env, &list)?,
            )?;
        }
        module.set_module_mirrors(Some(&bazel::ImmutableMap::copy_of(env, &by_registry)?))?;
    }

    module.set_check_direct_deps_mode(&repo.check_direct_dependencies()?)?;
    module.set_bazel_compatibility_mode(&repo.bazel_compatibility_mode()?)?;
    module.set_bazel_lockfile_mode(&repo.lockfile_mode()?)?;
    module.set_allowed_yanked_versions(&repo.allowed_yanked_versions()?)?;
    match repo.vendor_directory()? {
        Some(dir) if !dir.is_empty()? => {
            module
                .set_vendor_directory(Some(&workspace_path.get_relative(&dir.path_string()?)?))?;
        }
        _ => module.set_vendor_directory(None::<&bazel::Path>)?,
    }

    let cache = module.repository_cache()?;
    match repo.repository_cache()? {
        Some(dir) if !dir.is_empty()? => {
            cache.set_path(Some(&workspace_path.get_relative(&dir.path_string()?)?))?;
        }
        _ => {
            let user_root = workspace
                .output_base()?
                .parent_directory()?
                .ok_or_else(|| anyhow::anyhow!("the output base has no parent directory"))?;
            cache.set_path(Some(&user_root.get_relative("cache/repos/v1")?))?;
        }
    }
    Ok(())
}

/// The `DownloadManager` a command's `beforeCommand` creates, from the
/// options, and the functions that use it; also the fetch and extension
/// timeout scaling and the distdir.
pub(super) fn install_download_manager(
    env: &mut JNIEnv<'_>,
    runtime: &bazel::BlazeRuntime,
    reporter: &bazel::Reporter,
    options: &bazel::OptionsParser,
    module: &BazelRepositoryModule,
) -> Result<()> {
    let common =
        options.get_options(&class::<bazel::CommonCommandOptions>(env, COMMON_OPTIONS)?)?;
    let repo = options.get_options(&class::<bazel::RepositoryOptions>(env, REPOSITORY_OPTIONS)?)?;
    let cache = module.repository_cache()?;
    cache
        .download_cache()?
        .set_hardlink(repo.use_hardlinks()?)?;

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
    let workspace_path = runtime.workspace()?.workspace()?;
    let distdir = bazel::List::<bazel::Path>::new(env)?;
    if let Some(dirs) = repo.experimental_distdir()? {
        for d in &dirs.upcast::<bazel::Iterable<bazel::PathFragment>>() {
            distdir.add(&workspace_path.get_relative(&d?.path_string()?)?)?;
        }
    }
    manager.set_distdir(&distdir)?;

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
