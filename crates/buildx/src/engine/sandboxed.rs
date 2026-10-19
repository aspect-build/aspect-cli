//! Bazel's sandboxed local runner, built without a `CommandEnvironment`.
//!
//! `DarwinSandboxedSpawnRunner`, `ProcessWrapperSandboxedSpawnRunner` and
//! `LinuxSandboxedSpawnRunner` are package private and their constructors take
//! a `CommandEnvironment` (they read the options, the file system, the bin
//! tools, the exec root, the resource manager, the reporter and the client
//! environment from it), which a hosted engine has none of. So the runner
//! shell is Rust, a `SpawnRunner` that does what `AbstractSandboxSpawnRunner.exec`
//! does, in the same order, over the pieces that are public: Bazel's own
//! `SymlinkedSandboxedSpawn` builds, cleans and copies out of the sandbox
//! directory, `SandboxHelpers` maps the inputs and outputs,
//! `ProcessWrapper` builds the command line, and on macOS Bazel's own
//! `writeConfig` writes the `sandbox-exec` profile. Bazel's
//! `StandaloneSpawnStrategy` is then put over this runner exactly as it is
//! over `LocalSpawnRunner`, so the spawn context, caching hooks, output
//! handling and the `ExecException` family stay Bazel's.
//!
//! What is not reproduced: interrupting a running sandboxed process when the
//! build is interrupted, `--sandbox_debug`, the sandbox stash that reuses
//! directories, per-spawn resource statistics, and Linux's `linux-sandbox`
//! helper (a Linux host gets `process-wrapper` as the sandbox).

use super::*;
use crate::sandbox::{Kind, Sandboxing};
use blazon_rt::Result as RtResult;
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

const SANDBOX_OPTIONS: &str = "com/google/devtools/build/lib/sandbox/SandboxOptions";
const SANDBOX_DEBUG_SUGGESTION: &str = "\n\nUse --sandbox_debug to see verbose messages from the sandbox and retain the sandbox build root for debugging";

/// The sandboxed strategy of one build and what it takes to clean up after it.
/// Where the sandboxed spawns spent their time, in microseconds, for `BUILDX_TRACE`.
#[derive(Default)]
struct Timings {
    spawns: AtomicU64,
    prepare: AtomicU64,
    file_system: AtomicU64,
    process: AtomicU64,
    outputs: AtomicU64,
    delete: AtomicU64,
}

fn spent(slot: &AtomicU64, since: std::time::Instant) {
    slot.fetch_add(since.elapsed().as_micros() as u64, Ordering::Relaxed);
}

pub struct Sandbox {
    timings: Arc<Timings>,
    pub strategy: Global<bazel::StandaloneSpawnStrategy<'static>>,
    trash: Arc<Trash>,
    base: PathBuf,
}

/// What `local_executor` already holds and the runner needs.
pub struct Parts<'a, 'l> {
    pub runtime: &'a bazel::BlazeRuntime<'l>,
    pub exec_root: &'a bazel::Path<'l>,
    pub resources: &'a bazel::ResourceManager<'l>,
    pub execution: &'a bazel::ExecutionOptions<'l>,
    pub wrapper: &'a bazel::ProcessWrapper<'l>,
    pub env_provider: &'a bazel::LocalEnvProvider<'l>,
}

impl Sandbox {
    /// Builds the sandboxed strategy, or `None` when no executor of the build needs one.
    /// An error when one is needed and cannot be had.
    pub fn start<'l>(
        env: &mut JNIEnv<'l>,
        need: Sandboxing,
        parts: &Parts<'_, 'l>,
    ) -> Result<Option<Sandbox>> {
        if need == Sandboxing::None {
            return Ok(None);
        }
        let kind = crate::sandbox::host().map_err(|e| anyhow::anyhow!("{e}"))?;
        let runtime = parts.runtime;
        let workspace = runtime.workspace()?;
        let file_system = runtime.file_system()?;
        let base = workspace.output_base()?.get_relative("sandbox")?;
        let base_dir = PathBuf::from(base.path_string()?);
        // A previous run that was killed leaves its sandboxes behind; nothing
        // else uses this directory, so it starts empty, as `SandboxModule` leaves it on a first build.
        match fs::remove_dir_all(&base_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => bail!("clear the sandbox base {}: {e}", base_dir.display()),
        }
        let trash_dir = base_dir.join("_moved_trash_dir");
        fs::create_dir_all(&trash_dir)
            .map_err(|e| anyhow::anyhow!("create {}: {e}", trash_dir.display()))?;
        let trash = Arc::new(Trash::start(trash_dir));
        let deleter = bazel::TreeDeleter::from_impl(env, Box::new(Deleter(trash.clone())))?;

        let options = sandbox_options(env)?;
        let inaccessible = options.get_inaccessible_paths(&file_system)?;
        let timings = Arc::new(Timings::default());
        let runner = Runner {
            timings: timings.clone(),
            kind,
            exec_root: parts.exec_root.to_global()?,
            exec_root_name: PathBuf::from(parts.exec_root.path_string()?)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            base: base.to_global()?,
            file_system: file_system.to_global()?,
            deleter: deleter.to_global()?,
            wrapper: parts.wrapper.to_global()?,
            env_provider: parts.env_provider.to_global()?,
            bin_tools: workspace.bin_tools()?.to_global()?,
            resources: parts.resources.to_global()?,
            default_allow_network: options.default_allow_network()?,
            inaccessible: inaccessible
                .upcast::<bazel::Set<bazel::Path>>()
                .to_global()?,
            always_writable: if kind == Kind::Darwin {
                darwin_writable_dirs()
            } else {
                Vec::new()
            },
        };
        let runner = bazel::SpawnRunner::from_impl(env, Box::new(runner))?;
        let strategy = bazel::StandaloneSpawnStrategy::new(env, &runner, parts.execution)?;
        Ok(Some(Sandbox {
            timings,
            strategy: strategy.to_global()?,
            trash,
            base: base_dir,
        }))
    }
}

impl Drop for Sandbox {
    /// Waits for the deletions the build queued and removes the sandbox base,
    /// so the output base does not grow across builds.
    fn drop(&mut self) {
        let t = &self.timings;
        let n = t.spawns.load(Ordering::Relaxed).max(1);
        if std::env::var_os("BUILDX_TRACE").is_some() {
            let us = |a: &AtomicU64| a.load(Ordering::Relaxed) / n;
            eprintln!(
                "sandbox: {n} spawns, per spawn: prepare {}us, createFileSystem {}us, process {}us, copyOutputs {}us, delete {}us",
                us(&t.prepare),
                us(&t.file_system),
                us(&t.process),
                us(&t.outputs),
                us(&t.delete)
            );
        }
        self.trash.shutdown();
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// `SandboxOptions` at Bazel's defaults: an `OptionsParser` over just that class.
fn sandbox_options<'l>(env: &mut JNIEnv<'l>) -> Result<bazel::SandboxOptions<'l>> {
    let classes = bazel::Set::<bazel::Class<OptionsBase>>::new(env)?;
    classes.add(&class::<OptionsBase>(env, SANDBOX_OPTIONS)?)?;
    let parser = bazel::OptionsParser::builder(env)?
        .options_classes(&classes)?
        .build()?;
    parser.parse(&bazel::List::of_strs(env, &[] as &[&str])?)?;
    Ok(parser.get_options(&class::<bazel::SandboxOptions>(env, SANDBOX_OPTIONS)?)?)
}

/// `DarwinSandboxedSpawnRunner.getAlwaysWritableDirs`: what every sandboxed
/// process may write to besides its own exec root.
fn darwin_writable_dirs() -> Vec<String> {
    let mut dirs = Vec::new();
    let mut add = |p: &FsPath| {
        if let Ok(resolved) = fs::canonicalize(p) {
            dirs.push(resolved.to_string_lossy().into_owned());
        }
    };
    for p in ["/dev", "/tmp", "/private/tmp", "/private/var/tmp"] {
        add(FsPath::new(p));
    }
    for var in ["DARWIN_USER_TEMP_DIR", "DARWIN_USER_CACHE_DIR"] {
        if let Ok(out) = std::process::Command::new("/usr/bin/getconf")
            .arg(var)
            .output()
        {
            let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !value.is_empty() {
                add(FsPath::new(&value));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for sub in ["Library/Caches", "Library/Logs", "Library/Developer"] {
            add(&home.join(sub));
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Deletes trees off the build's critical path, as `AsynchronousTreeDeleter`
/// does, but joinable: a tree is renamed into the trash directory and removed by
/// one thread, and `shutdown` returns when the trash is empty.
struct Trash {
    dir: PathBuf,
    next: std::sync::atomic::AtomicU64,
    tx: std::sync::Mutex<Option<mpsc::Sender<PathBuf>>>,
    worker: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Trash {
    fn start(dir: PathBuf) -> Trash {
        let (tx, rx) = mpsc::channel::<PathBuf>();
        let worker = std::thread::spawn(move || {
            for path in rx {
                let _ = fs::remove_dir_all(&path);
            }
        });
        Trash {
            dir,
            next: Default::default(),
            tx: std::sync::Mutex::new(Some(tx)),
            worker: std::sync::Mutex::new(Some(worker)),
        }
    }

    fn delete(&self, path: &FsPath) {
        if !path.exists() {
            return;
        }
        let n = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let moved = self.dir.join(n.to_string());
        let queued = fs::rename(path, &moved).is_ok()
            && self
                .tx
                .lock()
                .ok()
                .and_then(|tx| tx.as_ref().map(|tx| tx.send(moved.clone()).is_ok()))
                .unwrap_or(false);
        if !queued {
            let _ = fs::remove_dir_all(&moved);
            let _ = fs::remove_dir_all(path);
        }
    }

    fn shutdown(&self) {
        if let Ok(mut tx) = self.tx.lock() {
            tx.take();
        }
        if let Some(worker) = self.worker.lock().ok().and_then(|mut w| w.take()) {
            let _ = worker.join();
        }
    }
}

struct Deleter(Arc<Trash>);

impl bazel::TreeDeleterImpl for Deleter {
    fn delete_tree<'a>(&self, _env: &mut JNIEnv<'a>, path: bazel::Path<'a>) -> RtResult<()> {
        self.0.delete(FsPath::new(&path.path_string()?));
        Ok(())
    }

    fn shutdown<'a>(&self, _env: &mut JNIEnv<'a>) -> RtResult<()> {
        self.0.shutdown();
        Ok(())
    }
}

/// Closes the resource handle the way try-with-resources does.
struct Held<'a>(bazel::ResourceHandle<'a>);

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let _ = self.0.close();
    }
}

struct Runner {
    timings: Arc<Timings>,
    kind: Kind,
    exec_root: Global<bazel::Path<'static>>,
    exec_root_name: String,
    base: Global<bazel::Path<'static>>,
    file_system: Global<bazel::FileSystem<'static>>,
    deleter: Global<bazel::TreeDeleter<'static>>,
    wrapper: Global<bazel::ProcessWrapper<'static>>,
    env_provider: Global<bazel::LocalEnvProvider<'static>>,
    bin_tools: Global<bazel::BinTools<'static>>,
    resources: Global<bazel::ResourceManager<'static>>,
    default_allow_network: bool,
    inaccessible: Global<bazel::Set<'static, bazel::Path<'static>>>,
    always_writable: Vec<String>,
}

fn string_list<'a>(
    env: &mut JNIEnv<'a>,
    items: &[String],
) -> RtResult<bazel::ImmutableList<'a, bazel::JavaString<'static>>> {
    let list = bazel::List::<bazel::JavaString>::new(env)?;
    for item in items {
        list.add(&bazel::JavaString::new(env, item)?)?;
    }
    bazel::ImmutableList::copy_of::<bazel::JavaString>(env, &list)
}

/// A failure of the sandbox itself (not of the action), reported the way
/// `AbstractSandboxSpawnRunner` reports one: an exec exception.
fn sandbox_failure(
    env: &mut JNIEnv<'_>,
    runner: &str,
    what: &str,
    cause: impl std::fmt::Display,
) -> blazon_rt::Error {
    match strategy::failure(env, &format!("{what}: {cause}"), 0, runner) {
        Ok(e) | Err(e) => e,
    }
}

/// The failure detail of an action that ran and failed.
fn spawn_detail<'a>(
    env: &mut JNIEnv<'a>,
    message: &str,
    timed_out: bool,
) -> RtResult<bazel::FailureDetail<'a>> {
    let code = if timed_out {
        bazel::SpawnDetailCode::timeout(env)?
    } else {
        bazel::SpawnDetailCode::non_zero_exit(env)?
    };
    let spawn = bazel::SpawnDetail::new_builder(env)?
        .set_code(&code)?
        .build()?;
    bazel::FailureDetail::new_builder(env)?
        .set_message(message)?
        .set_spawn(&spawn)?
        .build()
}

/// `getWritableDirs`: the sandbox exec root, and the `TEST_TMPDIR` and `TMPDIR`
/// the action will use, resolved when they are outside it.
fn writable_dirs(root: &str, env: &[(String, String)]) -> std::result::Result<Vec<String>, String> {
    let mut dirs = vec![root.to_string()];
    let mut add = |var: &str, required: bool| -> std::result::Result<(), String> {
        let Some((_, value)) = env.iter().find(|(k, _)| k == var) else {
            return if required {
                Err(format!("{var} is not set in the action's environment"))
            } else {
                Ok(())
            };
        };
        let path = if value.starts_with('/') {
            PathBuf::from(value)
        } else {
            FsPath::new(root).join(value)
        };
        if path.starts_with(root) {
            dirs.push(path.to_string_lossy().into_owned());
        } else if path.exists() {
            let resolved = fs::canonicalize(&path).map_err(|e| format!("resolve {var}: {e}"))?;
            dirs.push(resolved.to_string_lossy().into_owned());
        } else {
            return Err(format!(
                "Cannot resolve symlinks in {var} because it doesn't exist: \"{}\"",
                path.display()
            ));
        }
        Ok(())
    };
    add("TEST_TMPDIR", false)?;
    add("TMPDIR", true)?;
    Ok(dirs)
}

fn exit_code_of(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

impl Runner {
    fn paths<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        dirs: &[String],
    ) -> RtResult<bazel::Set<'a, bazel::Path<'static>>> {
        let set = bazel::Set::<bazel::Path>::new(env)?;
        let fs = self.file_system.get(env)?;
        for dir in dirs {
            set.add(&fs.get_path(dir)?)?;
        }
        Ok(set)
    }

    fn run<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        spawn: &bazel::Spawn<'a>,
        context: &bazel::SpawnExecutionContext<'a>,
    ) -> RtResult<bazel::SpawnResult<'a>> {
        let name = self.kind.runner();
        let began = std::time::Instant::now();
        self.timings.spawns.fetch_add(1, Ordering::Relaxed);
        let exec_root = self.exec_root.get(env)?;
        let sandbox_path = self
            .base
            .get(env)?
            .get_relative(name)?
            .get_relative(&context.id()?.to_string())?;
        sandbox_path.create_directory_and_parents()?;
        let sandbox_exec_root = sandbox_path
            .get_relative("execroot")?
            .get_relative(&self.exec_root_name)?;
        sandbox_exec_root.create_directory_and_parents()?;
        let root = sandbox_exec_root.path_string()?;

        let environment = self.env_provider.get(env)?.rewrite_local_env(
            &spawn.environment()?,
            &self.bin_tools.get(env)?,
            "/tmp",
        )?;
        let environment_pairs = strategy::map(&environment)?;
        let mut writable = writable_dirs(&root, &environment_pairs).map_err(|e| {
            sandbox_failure(env, name, "I/O exception during sandboxed execution", e)
        })?;
        let writable_set = self.paths(env, &writable)?;

        let mapping =
            context.get_input_mapping(&bazel::PathFragment::empty_fragment(env)?, true)?;
        let inputs = bazel::SandboxHelpers::process_input_files(
            env,
            &mapping.upcast::<bazel::Map<bazel::PathFragment, bazel::ActionInput>>(),
            &exec_root,
        )?;
        let outputs = bazel::SandboxHelpers::get_outputs(env, spawn)?;

        let stats = sandbox_path.get_relative("stats.out")?;
        let timeout = context.timeout()?;
        let line = self
            .wrapper
            .get(env)?
            .command_line_builder(&spawn.arguments()?)?
            .add_execution_info(&spawn.execution_info()?)?
            .set_timeout(&timeout)?
            .set_statistics_path(&stats.as_fragment()?)?
            .build()?;
        let mut arguments = Vec::new();
        let sandbox_config = sandbox_path.get_relative("sandbox.sb")?;
        if self.kind == Kind::Darwin {
            arguments.extend([
                "/usr/bin/sandbox-exec".to_string(),
                "-f".to_string(),
                sandbox_config.path_string()?,
            ]);
        }
        for a in &line.upcast::<bazel::Iterable<bazel::JavaString>>() {
            arguments.push(a?.to_rust()?);
        }
        let argv = string_list(env, &arguments)?;

        let execution_info = strategy::map(&spawn.execution_info()?)?;
        let allow_network =
            crate::sandbox::requires_network(&execution_info, self.default_allow_network);
        let label = spawn.target_label()?;
        let deleter = self.deleter.get(env)?;
        let sandboxed = bazel::SymlinkedSandboxedSpawn::new(
            env,
            &sandbox_path,
            &sandbox_exec_root,
            &argv,
            &environment,
            &inputs,
            &outputs,
            &writable_set,
            &deleter,
            None::<&bazel::Path>,
            Some(&stats),
            None::<&bazel::ImmutableList<bazel::JavaString>>,
            &spawn.mnemonic()?,
            label.as_ref(),
        )?;

        spent(&self.timings.prepare, began);
        let result = self.execute(
            env,
            spawn,
            context,
            &sandboxed,
            &Command {
                arguments,
                environment: environment_pairs,
            },
            &sandbox_config,
            &mut writable,
            &stats,
            allow_network,
            &root,
            &exec_root,
        );
        // `sandbox.delete()` of the finally block.
        let deleting = std::time::Instant::now();
        let _ = sandboxed.delete();
        spent(&self.timings.delete, deleting);
        result
    }
}

struct Command {
    arguments: Vec<String>,
    environment: Vec<(String, String)>,
}

impl Runner {
    #[allow(clippy::too_many_arguments)]
    fn execute<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        spawn: &bazel::Spawn<'a>,
        context: &bazel::SpawnExecutionContext<'a>,
        sandboxed: &bazel::SymlinkedSandboxedSpawn<'a>,
        command: &Command,
        sandbox_config: &bazel::Path<'a>,
        writable: &mut Vec<String>,
        stats: &bazel::Path<'a>,
        allow_network: bool,
        root: &str,
        exec_root: &bazel::Path<'a>,
    ) -> RtResult<bazel::SpawnResult<'a>> {
        let name = self.kind.runner();
        let filling = std::time::Instant::now();
        sandboxed
            .create_file_system()
            .map_err(|e| sandbox_failure(env, name, "Could not copy inputs into sandbox", e))?;
        if self.kind == Kind::Darwin {
            writable.extend(self.always_writable.iter().cloned());
            let set = self.paths(env, writable)?;
            let inaccessible = self.inaccessible.get(env)?;
            bazel::DarwinSandboxedSpawnRunner::write_config(
                env,
                sandbox_config,
                &set,
                &inaccessible,
                allow_network,
                Some(stats),
            )?;
        }

        spent(&self.timings.file_system, filling);
        let out_err = context.file_out_err()?;
        let out_path = out_err.output_path()?.path_string()?;
        let err_path = out_err.error_path()?.path_string()?;
        let timeout = context.timeout()?.to_std()?;
        let started_at = bazel::Instant::now(env)?;
        let started = std::time::Instant::now();

        let spawned = (|| -> std::io::Result<std::process::ExitStatus> {
            let mut cmd = std::process::Command::new(&command.arguments[0]);
            cmd.args(&command.arguments[1..])
                .current_dir(root)
                .env_clear()
                .envs(command.environment.iter().map(|(k, v)| (k, v)))
                .stdin(std::process::Stdio::null())
                .stdout(fs::File::create(&out_path)?)
                .stderr(fs::File::create(&err_path)?);
            cmd.status()
        })();
        let wall = started.elapsed();
        self.timings
            .process
            .fetch_add(wall.as_micros() as u64, Ordering::Relaxed);
        let copying = std::time::Instant::now();

        let builder = bazel::SpawnResultBuilder::new(env)?.set_runner_name(name)?;
        let (status, exit_code, message, detail) = match spawned {
            Err(e) => {
                let _ = fs::write(
                    &err_path,
                    format!("Action failed to execute: java.io.IOException: {e}\n"),
                );
                let message = format!(
                    "{}{}",
                    spawn_failure_text(env, spawn, root)?,
                    SANDBOX_DEBUG_SUGGESTION
                );
                (
                    bazel::SpawnStatus::execution_failed(env)?,
                    -1,
                    message.clone(),
                    Some(spawn_detail(env, &message, false)?),
                )
            }
            Ok(status) => {
                let code = exit_code_of(status);
                let timed_out = !timeout.is_zero() && wall > timeout;
                if timed_out {
                    let message = format!(
                        "{}{}",
                        spawn_failure_text(env, spawn, root)?,
                        SANDBOX_DEBUG_SUGGESTION
                    );
                    (
                        bazel::SpawnStatus::timeout(env)?,
                        bazel::SpawnResult::POSIX_TIMEOUT_EXIT_CODE,
                        message.clone(),
                        Some(spawn_detail(env, &message, true)?),
                    )
                } else if code == 0 {
                    (bazel::SpawnStatus::success(env)?, 0, String::new(), None)
                } else {
                    let message = format!(
                        "{}{}",
                        spawn_failure_text(env, spawn, root)?,
                        SANDBOX_DEBUG_SUGGESTION
                    );
                    (
                        bazel::SpawnStatus::non_zero_exit(env)?,
                        code,
                        message.clone(),
                        Some(spawn_detail(env, &message, false)?),
                    )
                }
            }
        };
        let builder = builder
            .set_status(&status)?
            .set_exit_code(exit_code)?
            .set_start_time(&started_at)?
            .set_wall_time_in_ms(wall.as_millis() as i32)?
            .set_failure_message(&message)?;
        if let Some(detail) = &detail {
            builder.set_failure_detail(detail)?;
        }
        let result = builder.build()?;

        context.lock_output_files(exit_code, &message, &out_err)?;
        // The outputs are copied out even when the command failed.
        sandboxed
            .copy_outputs(exec_root)
            .map_err(|e| sandbox_failure(env, name, "Could not copy outputs from sandbox", e))?;
        spent(&self.timings.outputs, copying);
        Ok(result)
    }
}

fn spawn_failure_text(
    env: &mut JNIEnv<'_>,
    spawn: &bazel::Spawn<'_>,
    root: &str,
) -> RtResult<String> {
    bazel::CommandFailureUtils::describe_command_failure(
        env,
        false,
        Some(root),
        &spawn.upcast::<bazel::DescribableExecutionUnit>(),
    )
}

impl bazel::SpawnRunnerImpl for Runner {
    fn exec<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        spawn: bazel::Spawn<'a>,
        context: bazel::SpawnExecutionContext<'a>,
    ) -> RtResult<bazel::SpawnResult<'a>> {
        let name = self.kind.runner();
        context.report(&bazel::SpawnSchedulingEvent::create(env, name)?)?;
        context.prefetch_inputs_and_wait()?;
        let priority = if context.speculating()? {
            bazel::ResourcePriority::dynamic_standalone(env)?
        } else {
            bazel::ResourcePriority::local(env)?
        };
        let _held = Held(self.resources.get(env)?.acquire_resources(
            &spawn.resource_owner()?,
            &spawn.local_resources()?,
            &priority,
        )?);
        context.report(&bazel::SpawnExecutingEvent::create(env, name)?)?;
        self.run(env, &spawn, &context)
    }

    fn can_exec<'a>(&self, _env: &mut JNIEnv<'a>, spawn: bazel::Spawn<'a>) -> RtResult<bool> {
        Ok(crate::sandbox::may_be_sandboxed(&strategy::map(
            &spawn.execution_info()?,
        )?))
    }

    fn handles_caching<'a>(&self, _env: &mut JNIEnv<'a>) -> RtResult<bool> {
        Ok(false)
    }

    fn name<'a>(&self, _env: &mut JNIEnv<'a>) -> RtResult<String> {
        Ok(self.kind.runner().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_exec_root_is_always_writable() {
        let dirs = writable_dirs("/sb/execroot/_main", &env(&[("TMPDIR", "tmp")])).unwrap();
        assert_eq!(dirs, ["/sb/execroot/_main", "/sb/execroot/_main/tmp"]);
    }

    #[test]
    fn a_missing_tmpdir_is_an_error() {
        let err = writable_dirs("/sb/execroot/_main", &env(&[])).unwrap_err();
        assert!(err.contains("TMPDIR"), "{err}");
        let err =
            writable_dirs("/sb/execroot/_main", &env(&[("TMPDIR", "/nonexistent/x")])).unwrap_err();
        assert!(err.contains("doesn't exist"), "{err}");
    }

    #[test]
    fn a_tmpdir_outside_the_root_is_resolved() {
        let dir = std::env::temp_dir();
        let dirs = writable_dirs(
            "/sb/execroot/_main",
            &env(&[("TMPDIR", dir.to_str().unwrap())]),
        )
        .unwrap();
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[1], fs::canonicalize(dir).unwrap().to_string_lossy());
    }

    #[test]
    fn the_trash_empties_on_shutdown() {
        let base = std::env::temp_dir().join(format!("buildx-trash-{}", std::process::id()));
        let trash_dir = base.join("trash");
        fs::create_dir_all(&trash_dir).unwrap();
        let victim = base.join("victim");
        fs::create_dir_all(victim.join("a/b")).unwrap();
        fs::write(victim.join("a/b/f"), "x").unwrap();
        let trash = Trash::start(trash_dir.clone());
        trash.delete(&victim);
        assert!(!victim.exists());
        trash.shutdown();
        assert_eq!(fs::read_dir(&trash_dir).unwrap().count(), 0);
        fs::remove_dir_all(&base).unwrap();
    }
}
