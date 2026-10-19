//! The remote REAPI executor and the `cached` combinator, over a `Store`.

use crate::execution::{Done, ExecOutcome, Executor, SpawnView};
use crate::reapi::{self, Entry, Request};
use crate::store::Store;
use anyhow::{Result, anyhow, bail};
use axl_proto::build::bazel::remote::execution::v2::{
    ActionResult, Digest, Directory, ExecuteRequest, ExecuteResponse, OutputDirectory, OutputFile,
    Tree, execution_client::ExecutionClient,
};
use axl_proto::google::longrunning::operation;
use prost::Message;
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A blob's bytes, read when the blob turns out to be missing.
type Loader = Box<dyn Fn() -> Result<Arc<Vec<u8>>> + Send + Sync>;

/// Blobs with their digests, bytes in hand.
type Blobs = Vec<(Digest, Arc<Vec<u8>>)>;

/// The REAPI messages of a request as the blobs a server needs to run it.
fn request_blobs(request: &Request, exec_root: &Path) -> Result<Vec<(Digest, Loader)>> {
    let mut blobs: Vec<(Digest, Loader)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |digest: Digest, load: Loader| {
        if seen.insert(digest.hash.clone()) {
            blobs.push((digest, load));
        }
    };
    let command = Arc::new(request.command.encode_to_vec());
    add(
        request.command_digest.clone(),
        Box::new(move || Ok(command.clone())),
    );
    let action = Arc::new(request.action.encode_to_vec());
    add(
        request.action_digest.clone(),
        Box::new(move || Ok(action.clone())),
    );
    for d in std::iter::once(&request.input_tree.root).chain(&request.input_tree.children) {
        let bytes = Arc::new(d.encode_to_vec());
        add(reapi::digest_of(d), Box::new(move || Ok(bytes.clone())));
    }
    for input in &request.inputs {
        if let Entry::File { digest, size, .. } = &input.entry {
            let d = Digest {
                hash: digest.clone(),
                size_bytes: *size,
            };
            if let Some(bytes) = &input.bytes {
                let bytes = bytes.clone();
                add(d, Box::new(move || Ok(bytes.clone())));
            } else {
                let path = exec_root.join(input.source.as_ref().unwrap_or(&input.path));
                add(
                    d,
                    Box::new(move || {
                        std::fs::read(&path)
                            .map(Arc::new)
                            .map_err(|e| anyhow!("read input {}: {e}", path.display()))
                    }),
                );
            }
        }
    }
    Ok(blobs)
}

/// Uploads what the server lacks of the request's blobs.
pub fn upload_request(store: &Store, request: &Request, exec_root: &Path) -> Result<usize> {
    let blobs = request_blobs(request, exec_root)?;
    let digests: Vec<Digest> = blobs.iter().map(|(d, _)| d.clone()).collect();
    let missing: std::collections::HashSet<String> = store
        .find_missing(&digests)?
        .into_iter()
        .map(|d| d.hash)
        .collect();
    let mut upload = Vec::new();
    for (digest, load) in blobs {
        if missing.contains(&digest.hash) {
            upload.push((digest, load()?));
        }
    }
    let n = upload.len();
    store.upload(upload)?;
    Ok(n)
}

fn write_file(path: &Path, data: &[u8], executable: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(path);
    std::fs::write(path, data).map_err(|e| anyhow!("write {}: {e}", path.display()))?;
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn write_directory(
    store: &Store,
    directory: &Directory,
    children: &HashMap<String, Directory>,
    at: &Path,
) -> Result<()> {
    std::fs::create_dir_all(at)?;
    for f in &directory.files {
        let digest = f
            .digest
            .clone()
            .ok_or_else(|| anyhow!("file node without digest"))?;
        write_file(
            &at.join(&f.name),
            &store.download(&digest)?,
            f.is_executable,
        )?;
    }
    for d in &directory.directories {
        let digest = d
            .digest
            .clone()
            .ok_or_else(|| anyhow!("directory node without digest"))?;
        let child = children
            .get(&digest.hash)
            .ok_or_else(|| anyhow!("tree lacks directory {}", digest.hash))?;
        write_directory(store, child, children, &at.join(&d.name))?;
    }
    for s in &directory.symlinks {
        let _ = std::fs::remove_file(at.join(&s.name));
        std::os::unix::fs::symlink(&s.target, at.join(&s.name))?;
    }
    Ok(())
}

/// Downloads an action's outputs into the execution root: every output file
/// and directory, parents created, executable bits set.
pub fn materialize(store: &Store, exec_root: &Path, result: &ActionResult) -> Result<()> {
    for f in &result.output_files {
        let path = exec_root.join(&f.path);
        let data = if f.contents.is_empty() {
            let digest = f
                .digest
                .clone()
                .ok_or_else(|| anyhow!("output {} has no digest", f.path))?;
            store.download(&digest)?
        } else {
            f.contents.clone()
        };
        write_file(&path, &data, f.is_executable)?;
    }
    for d in &result.output_directories {
        let digest = d
            .tree_digest
            .clone()
            .ok_or_else(|| anyhow!("output directory {} has no tree digest", d.path))?;
        let tree = Tree::decode(store.download(&digest)?.as_slice())?;
        let children: HashMap<String, Directory> = tree
            .children
            .iter()
            .map(|c| (reapi::digest_of(c).hash, c.clone()))
            .collect();
        let at = exec_root.join(&d.path);
        let _ = std::fs::remove_dir_all(&at);
        write_directory(store, &tree.root.unwrap_or_default(), &children, &at)?;
    }
    Ok(())
}

/// The declared outputs found in the execution root, as an `ActionResult`
/// and the blobs it names. An output that does not exist is left out.
pub fn collect_outputs(exec_root: &Path, output_paths: &[String]) -> Result<(ActionResult, Blobs)> {
    let mut result = ActionResult::default();
    let mut blobs = Vec::new();
    for path in output_paths {
        let on_disk: PathBuf = exec_root.join(path);
        let Ok(meta) = std::fs::symlink_metadata(&on_disk) else {
            continue;
        };
        if meta.is_dir() {
            let mut inputs = Vec::new();
            collect_dir(&on_disk, &on_disk, &mut inputs, &mut blobs)?;
            let tree = reapi::input_tree(&inputs);
            for d in std::iter::once(&tree.root).chain(&tree.children) {
                let bytes = d.encode_to_vec();
                blobs.push((reapi::digest_bytes(&bytes), Arc::new(bytes)));
            }
            let message = reapi::tree_message(&tree).encode_to_vec();
            let digest = reapi::digest_bytes(&message);
            blobs.push((digest.clone(), Arc::new(message)));
            result.output_directories.push(OutputDirectory {
                path: path.clone(),
                tree_digest: Some(digest),
                ..Default::default()
            });
        } else {
            let data = std::fs::read(&on_disk)
                .map_err(|e| anyhow!("read output {}: {e}", on_disk.display()))?;
            let digest = reapi::digest_bytes(&data);
            result.output_files.push(OutputFile {
                path: path.clone(),
                digest: Some(digest.clone()),
                is_executable: meta.permissions().mode() & 0o111 != 0,
                ..Default::default()
            });
            blobs.push((digest, Arc::new(data)));
        }
    }
    Ok((result, blobs))
}

fn collect_dir(
    root: &Path,
    dir: &Path,
    inputs: &mut Vec<reapi::Input>,
    blobs: &mut Vec<(Digest, Arc<Vec<u8>>)>,
) -> Result<()> {
    let mut any = false;
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        any = true;
        let path = e.path();
        let rel = path.strip_prefix(root)?.to_string_lossy().into_owned();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            collect_dir(root, &path, inputs, blobs)?;
        } else if meta.is_symlink() {
            inputs.push(reapi::Input {
                path: rel,
                entry: Entry::Symlink {
                    target: std::fs::read_link(&path)?.to_string_lossy().into_owned(),
                },
                bytes: None,
                source: None,
            });
        } else {
            let data = std::fs::read(&path)?;
            let digest = reapi::digest_bytes(&data);
            inputs.push(reapi::Input {
                path: rel,
                entry: Entry::File {
                    digest: digest.hash.clone(),
                    size: digest.size_bytes,
                    executable: meta.permissions().mode() & 0o111 != 0,
                },
                bytes: None,
                source: None,
            });
            blobs.push((digest, Arc::new(data)));
        }
    }
    if !any && dir != root {
        inputs.push(reapi::Input {
            path: dir.strip_prefix(root)?.to_string_lossy().into_owned(),
            entry: Entry::Directory,
            bytes: None,
            source: None,
        });
    }
    Ok(())
}

/// Runs spawns on a REAPI server: uploads what it lacks, calls `Execute`,
/// follows the operation to its end and downloads the outputs.
pub struct RemoteExecutor {
    pub name: String,
    pub store: Arc<Store>,
}

impl RemoteExecutor {
    fn execute(&self, request: &Request) -> Result<ExecuteResponse> {
        let store = &self.store;
        let mut client = ExecutionClient::new(store.channel());
        let message = ExecuteRequest {
            instance_name: store.instance().to_string(),
            skip_cache_lookup: request.action.do_not_cache,
            action_digest: Some(request.action_digest.clone()),
            ..Default::default()
        };
        let call = store.request_for(message)?;
        store.runtime().block_on(async {
            let mut stream = client
                .execute(call)
                .await
                .map_err(|e| {
                    anyhow!(
                        "Execute on {}: {}: {}",
                        store.endpoint(),
                        e.code(),
                        e.message()
                    )
                })?
                .into_inner();
            let mut last = None;
            while let Some(op) = stream.message().await.map_err(|e| {
                anyhow!(
                    "Execute on {}: {}: {}",
                    store.endpoint(),
                    e.code(),
                    e.message()
                )
            })? {
                let done = op.done;
                last = Some(op);
                if done {
                    break;
                }
            }
            let Some(op) = last.filter(|o| o.done) else {
                bail!(
                    "Execute on {} ended before the operation finished",
                    store.endpoint()
                );
            };
            match op.result {
                Some(operation::Result::Error(status)) => {
                    bail!(
                        "remote execution failed: code {}: {}",
                        status.code,
                        status.message
                    )
                }
                Some(operation::Result::Response(any)) => {
                    Ok(ExecuteResponse::decode(any.value.as_slice())?)
                }
                None => bail!("the operation finished without a result"),
            }
        })
    }
}

impl Executor for RemoteExecutor {
    fn name(&self) -> &str {
        &self.name
    }

    fn describe(&self, used: &mut crate::events::Used) {
        used.executor(&self.store);
    }

    fn runs_in_bazel(&self) -> bool {
        false
    }

    fn exec(&self, spawn: &Arc<SpawnView>) -> Result<ExecOutcome> {
        let started = std::time::Instant::now();
        let request = spawn.request()?;
        let converted = started.elapsed();
        let exec_root = PathBuf::from(spawn.exec_root()?);
        let uploaded = upload_request(&self.store, &request, &exec_root)?;
        let upload_time = started.elapsed() - converted;
        let response = self.execute(&request)?;
        let executed = started.elapsed() - converted - upload_time;
        if let Some(status) = &response.status {
            if status.code != 0 {
                bail!(
                    "remote execution failed: code {}: {}",
                    status.code,
                    status.message
                );
            }
        }
        let result = response
            .result
            .ok_or_else(|| anyhow!("the server returned no action result"))?;
        let (stdout, stderr) = streams(&self.store, &result)?;
        if result.exit_code == 0 {
            materialize(&self.store, &exec_root, &result)?;
        }
        if std::env::var_os("BUILDX_TRACE").is_some() {
            eprintln!(
                "remote {}: converted in {converted:?}, {uploaded} of {} blobs uploaded in {upload_time:?}, executed in {executed:?}, outputs downloaded in {:?}",
                spawn.data.mnemonic,
                request.inputs.len() + 2 + request.input_tree.children.len() + 1,
                started.elapsed() - converted - upload_time - executed
            );
        }
        Ok(ExecOutcome::Done(Box::new(Done {
            exit_code: result.exit_code,
            stdout,
            stderr,
            runner: format!("remote:{}", self.name),
            cache_hit: response.cached_result,
            action_result: Some(result),
        })))
    }
}

fn streams(store: &Store, result: &ActionResult) -> Result<(Vec<u8>, Vec<u8>)> {
    let get = |raw: &Vec<u8>, digest: &Option<Digest>| -> Result<Vec<u8>> {
        if !raw.is_empty() {
            return Ok(raw.clone());
        }
        match digest {
            Some(d) if d.size_bytes > 0 => store.download(d),
            _ => Ok(Vec::new()),
        }
    };
    Ok((
        get(&result.stdout_raw, &result.stdout_digest)?,
        get(&result.stderr_raw, &result.stderr_digest)?,
    ))
}

/// Looks the action up in the action cache: a hit downloads the outputs and is
/// the result; a miss runs `inner` and, when it succeeded, stores the outputs
/// and the result. A failed result is never stored.
pub struct Cached {
    pub store: Arc<Store>,
    pub inner: Arc<dyn Executor>,
}

impl Executor for Cached {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn describe(&self, used: &mut crate::events::Used) {
        used.cache(&self.store);
        self.inner.describe(used);
    }

    fn sandboxing(&self) -> crate::sandbox::Sandboxing {
        self.inner.sandboxing()
    }

    fn runs_in_bazel(&self) -> bool {
        false
    }

    fn exec(&self, spawn: &Arc<SpawnView>) -> Result<ExecOutcome> {
        let request = spawn.request()?;
        let exec_root = PathBuf::from(spawn.exec_root()?);
        if let Some(result) = self.store.get_action_result(&request.action_digest)? {
            if result.exit_code == 0 {
                match materialize(&self.store, &exec_root, &result) {
                    Ok(()) => {
                        let (stdout, stderr) = streams(&self.store, &result).unwrap_or_default();
                        return Ok(ExecOutcome::Done(Box::new(Done {
                            exit_code: 0,
                            stdout,
                            stderr,
                            runner: "cache".into(),
                            cache_hit: true,
                            action_result: Some(result),
                        })));
                    }
                    Err(e) => {
                        if std::env::var_os("BUILDX_TRACE").is_some() {
                            eprintln!("cache entry unusable, running instead: {e:#}");
                        }
                    }
                }
            }
        }
        let outcome = self.inner.exec(spawn)?;
        let succeeded = match &outcome {
            ExecOutcome::Bazel(_) => true,
            ExecOutcome::Done(d) => d.exit_code == 0,
        };
        if !succeeded {
            return Ok(outcome);
        }
        match &outcome {
            ExecOutcome::Done(done) if done.action_result.is_some() => {
                if let Some(result) = &done.action_result {
                    self.store
                        .update_action_result(&request.action_digest, result)?;
                }
            }
            _ => {
                let (mut result, blobs) =
                    collect_outputs(&exec_root, &request.command.output_paths)?;
                let digests: Vec<Digest> = blobs.iter().map(|(d, _)| d.clone()).collect();
                let missing: std::collections::HashSet<String> = self
                    .store
                    .find_missing(&digests)?
                    .into_iter()
                    .map(|d| d.hash)
                    .collect();
                self.store.upload(
                    blobs
                        .into_iter()
                        .filter(|(d, _)| missing.contains(&d.hash))
                        .collect(),
                )?;
                result.exit_code = 0;
                self.store
                    .update_action_result(&request.action_digest, &result)?;
            }
        }
        Ok(outcome)
    }
}
