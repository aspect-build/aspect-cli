//! Executors and routing.
//!
//! An `Executor` turns a spawn, seen as REAPI messages plus the Bazel-side
//! context, into a result. A `Router` picks one executor per spawn. The
//! built-in `LocalExecutor` is Bazel's own local spawn strategy: it runs where
//! Bazel runs it (so process-wrapper handling, outputs, failures and the
//! `ExecException` family stay Bazel's), and the Rust strategy in front of it
//! only decides.

use crate::bindings::v9_0_1 as bazel;
use crate::reapi::{Entry, Input, Request, SpawnData};
use crate::sandbox::{SandboxExecutor, Sandboxing};
use anyhow::{Result, anyhow, bail};
use blazon_rt::{Global, HandleExt, vm};
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// One spawn as routers and executors see it: its plain data at once, its REAPI
/// messages when first asked for (the input root needs the build's metadata).
pub struct SpawnView {
    pub data: SpawnData,
    spawn: Global<bazel::Spawn<'static>>,
    context: Global<bazel::ActionExecutionContext<'static>>,
    /// Bazel's own local strategy, for executors that wrap running locally.
    delegate: Global<bazel::StandaloneSpawnStrategy<'static>>,
    /// The same strategy over the sandboxed runner, when the build has one.
    sandbox: Option<Global<bazel::StandaloneSpawnStrategy<'static>>>,
    request: OnceLock<std::result::Result<Arc<Request>, String>>,
}

impl SpawnView {
    pub fn new(
        data: SpawnData,
        spawn: Global<bazel::Spawn<'static>>,
        context: Global<bazel::ActionExecutionContext<'static>>,
        delegate: Global<bazel::StandaloneSpawnStrategy<'static>>,
        sandbox: Option<Global<bazel::StandaloneSpawnStrategy<'static>>>,
    ) -> SpawnView {
        SpawnView {
            data,
            spawn,
            context,
            delegate,
            sandbox,
            request: OnceLock::new(),
        }
    }

    /// The execution root's path.
    pub fn exec_root(&self) -> Result<String> {
        let mut env = vm::env()?;
        Ok(self.context.get(&mut env)?.exec_root()?.path_string()?)
    }

    /// Runs the spawn with Bazel's own local strategy and returns its results
    /// unchanged. A failed spawn is that strategy's exception, as an error.
    pub fn run_in_bazel(
        &self,
    ) -> Result<Global<bazel::ImmutableList<'static, bazel::SpawnResult<'static>>>> {
        Self::run_with(&self.delegate, &self.spawn, &self.context)
    }

    /// Like `run_in_bazel`, in Bazel's sandboxed runner. An error when the
    /// build has none, which only a routing mistake can cause.
    pub fn run_in_sandbox(
        &self,
    ) -> Result<Global<bazel::ImmutableList<'static, bazel::SpawnResult<'static>>>> {
        let Some(sandbox) = &self.sandbox else {
            bail!("this build has no sandbox");
        };
        Self::run_with(sandbox, &self.spawn, &self.context)
    }

    fn run_with(
        strategy: &Global<bazel::StandaloneSpawnStrategy<'static>>,
        spawn: &Global<bazel::Spawn<'static>>,
        context: &Global<bazel::ActionExecutionContext<'static>>,
    ) -> Result<Global<bazel::ImmutableList<'static, bazel::SpawnResult<'static>>>> {
        let mut env = vm::env()?;
        let results = strategy
            .get(&mut env)?
            .exec(&spawn.get(&mut env)?, &context.get(&mut env)?)?;
        Ok(results.to_global()?)
    }

    /// Bazel's description of a failed command, as its own strategies report it.
    pub fn describe_failure(&self) -> Result<String> {
        let mut env = vm::env()?;
        let spawn = self.spawn.get(&mut env)?;
        let root = self.exec_root()?;
        Ok(bazel::CommandFailureUtils::describe_command_failure(
            &mut env,
            false,
            Some(&root),
            &spawn.upcast::<bazel::DescribableExecutionUnit>(),
        )?)
    }

    /// Writes to the action's stdout and stderr.
    pub fn write_streams(&self, stdout: &[u8], stderr: &[u8]) -> Result<()> {
        let mut env = vm::env()?;
        let out = self
            .context
            .get(&mut env)?
            .file_out_err()?
            .upcast::<bazel::OutErr>();
        if !stdout.is_empty() {
            out.print_out(&String::from_utf8_lossy(stdout))?;
        }
        if !stderr.is_empty() {
            out.print_err(&String::from_utf8_lossy(stderr))?;
        }
        Ok(())
    }

    /// The `Command`, input root and `Action`, built once per view.
    pub fn request(&self) -> Result<Arc<Request>> {
        self.request
            .get_or_init(|| {
                self.build_request()
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"))
            })
            .clone()
            .map_err(|e| anyhow!("{e}"))
    }

    fn build_request(&self) -> Result<Request> {
        let started = std::time::Instant::now();
        let inputs = self.inputs()?;
        let gathered = started.elapsed();
        let request = crate::reapi::request(&self.data, &inputs);
        if std::env::var_os("BUILDX_TRACE").is_some() {
            eprintln!(
                "reapi {}: {} inputs gathered in {:?}, tree of {} files and {} directories built in {:?}",
                self.data.mnemonic,
                inputs.len(),
                gathered,
                request.input_tree.files,
                request.input_tree.children.len() + 1,
                started.elapsed() - gathered
            );
        }
        Ok(request)
    }

    /// The input root's entries: the spawn's inputs expanded the way Bazel's
    /// remote execution does (tree artifacts to their files, runfiles trees to
    /// the artifacts they hold), with the digests Skyframe recorded.
    fn inputs(&self) -> Result<Vec<Input>> {
        let mut env = vm::env()?;
        let spawn = self.spawn.get(&mut env)?;
        let context = self.context.get(&mut env)?;
        let provider = context.input_metadata_provider()?;
        let exec_root = context.exec_root()?.path_string()?;
        // Bazel's own input mapping: tree artifacts expanded to their files,
        // runfiles in their `.runfiles` layout, filesets as their links.
        let expander = bazel::SpawnInputExpander::new(&mut env)?;
        let base = bazel::PathFragment::empty_fragment(&mut env)?;
        let mapping = expander.get_input_mapping(&spawn, &provider, &base)?;
        let mapping = mapping.upcast::<bazel::Map<bazel::PathFragment, bazel::ActionInput>>();
        let mut out = Vec::new();
        for entry in mapping.entries()? {
            let entry = entry?;
            let path = entry.key()?.path_string()?;
            add_input(&provider, &exec_root, path, &entry.value()?, &mut out)?;
        }
        Ok(out)
    }
}

fn add_input(
    provider: &bazel::InputMetadataProvider,
    exec_root: &str,
    path: String,
    input: &bazel::ActionInput,
    out: &mut Vec<Input>,
) -> Result<()> {
    if input.is_directory()? {
        out.push(Input {
            path,
            entry: Entry::Directory,
            bytes: None,
            source: None,
        });
        return Ok(());
    }
    if input.is_a::<bazel::VirtualActionInput>()? {
        let bytes = input
            .downcast::<bazel::VirtualActionInput>()?
            .bytes()?
            .to_byte_array()?;
        let digest = crate::reapi::digest_bytes(&bytes);
        out.push(Input {
            path,
            entry: Entry::File {
                digest: digest.hash,
                size: digest.size_bytes,
                executable: false,
            },
            bytes: Some(Arc::new(bytes)),
            source: None,
        });
        return Ok(());
    }
    if input.is_symlink()? {
        if let Some(target) = provider
            .get_input_metadata(input)?
            .and_then(|m| m.unresolved_symlink_target().ok())
        {
            out.push(Input {
                path,
                entry: Entry::Symlink { target },
                bytes: None,
                source: None,
            });
            return Ok(());
        }
    }
    // Where the file's bytes are: its own execution path, which differs from
    // `path` for a runfiles entry.
    let on_disk = Path::new(exec_root).join(input.exec_path_string()?);
    let executable = executable(&on_disk);
    let (digest, size) = match provider.get_input_metadata(input)? {
        Some(m) => {
            let bytes = m.digest()?.unwrap_or_default();
            if bytes.is_empty() {
                hash_file(&on_disk)?
            } else {
                (
                    bytes.iter().map(|b| format!("{b:02x}")).collect(),
                    m.size()?,
                )
            }
        }
        None => hash_file(&on_disk)?,
    };
    out.push(Input {
        path,
        entry: Entry::File {
            digest,
            size,
            executable,
        },
        bytes: None,
        source: Some(input.exec_path_string()?),
    });
    Ok(())
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

fn hash_file(path: &Path) -> Result<(String, i64)> {
    let bytes = std::fs::read(path).map_err(|e| anyhow!("read input {}: {e}", path.display()))?;
    let digest = crate::reapi::digest_bytes(&bytes);
    Ok((digest.hash, digest.size_bytes))
}

/// What an executor that does not run in Bazel hands back.
pub struct Done {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub runner: String,
    pub cache_hit: bool,
    /// The REAPI result, when the executor has one to store.
    pub action_result: Option<axl_proto::build::bazel::remote::execution::v2::ActionResult>,
}

pub enum ExecOutcome {
    /// Bazel's own strategy ran it: its results, unchanged.
    Bazel(Global<bazel::ImmutableList<'static, bazel::SpawnResult<'static>>>),
    Done(Box<Done>),
}

/// Where a spawn runs.
pub trait Executor: Send + Sync {
    fn name(&self) -> &str;

    /// Bazel's own spawn strategy runs it (and keeps its exceptions and
    /// output handling). Only an executor that returns false is asked to `exec`.
    fn runs_in_bazel(&self) -> bool {
        true
    }

    fn exec(&self, spawn: &Arc<SpawnView>) -> Result<ExecOutcome> {
        let _ = spawn;
        bail!("executor {} runs in Bazel", self.name())
    }

    /// Whether this executor needs the build to have Bazel's sandboxed runner.
    fn sandboxing(&self) -> Sandboxing {
        Sandboxing::None
    }

    /// Adds the remote services this executor reaches, for the event stream's command line.
    fn describe(&self, used: &mut crate::events::Used) {
        let _ = used;
    }
}

/// Bazel's local spawn strategy, under a name scripts can tell apart.
pub struct LocalExecutor {
    pub name: String,
}

impl Executor for LocalExecutor {
    fn name(&self) -> &str {
        &self.name
    }

    fn exec(&self, spawn: &Arc<SpawnView>) -> Result<ExecOutcome> {
        Ok(ExecOutcome::Bazel(spawn.run_in_bazel()?))
    }
}

/// Picks an executor per spawn.
pub trait Router: Send + Sync {
    fn route(&self, spawn: &Arc<SpawnView>) -> Result<Arc<dyn Executor>>;

    /// The strongest sandbox need of any executor this router can pick.
    fn sandboxing(&self) -> Sandboxing {
        Sandboxing::None
    }

    /// Adds the remote services of every executor this router can pick.
    fn describe(&self, used: &mut crate::events::Used) {
        let _ = used;
    }
}

/// Every spawn to one executor.
pub struct Fixed(pub Arc<dyn Executor>);

impl Router for Fixed {
    fn sandboxing(&self) -> Sandboxing {
        self.0.sandboxing()
    }

    fn route(&self, _: &Arc<SpawnView>) -> Result<Arc<dyn Executor>> {
        Ok(self.0.clone())
    }

    fn describe(&self, used: &mut crate::events::Used) {
        self.0.describe(used);
    }
}

pub fn local_router() -> Arc<dyn Router> {
    Arc::new(Fixed(Arc::new(LocalExecutor {
        name: "local".into(),
    })))
}

/// What a build with no executor runs with: the sandbox, as Bazel does, or
/// `local()` with a line on stderr when this host cannot sandbox.
pub fn default_router() -> Arc<dyn Router> {
    match crate::sandbox::host() {
        Ok(_) => Arc::new(Fixed(SandboxExecutor::optional())),
        Err(why) => {
            eprintln!("buildx: no sandbox works on this host ({why}); actions run unsandboxed");
            local_router()
        }
    }
}
