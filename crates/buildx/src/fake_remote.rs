//! An in-memory REAPI server for tests and examples: a test fixture, not a
//! product. It serves `ContentAddressableStorage`, `ByteStream`, `ActionCache`,
//! `Capabilities` and `Execution`; `Execute` materializes the input root from
//! its CAS into a temporary directory, runs the command's arguments directly (no
//! shell), and collects the declared outputs back into the CAS.

use crate::reapi::{self, Entry, Input};
use axl_proto::build::bazel::remote::execution::v2::{
    Action, ActionResult, BatchReadBlobsRequest, BatchReadBlobsResponse, BatchUpdateBlobsRequest,
    BatchUpdateBlobsResponse, Command, Digest, Directory, ExecuteOperationMetadata, ExecuteRequest,
    ExecuteResponse, ExecutedActionMetadata, FindMissingBlobsRequest, FindMissingBlobsResponse,
    GetActionResultRequest, GetCapabilitiesRequest, GetTreeRequest, GetTreeResponse,
    OutputDirectory, OutputFile, ServerCapabilities, UpdateActionResultRequest,
    action_cache_server, batch_read_blobs_response, batch_update_blobs_response,
    capabilities_server, content_addressable_storage_server, execution_server,
};
use axl_proto::google::bytestream::{
    QueryWriteStatusRequest, QueryWriteStatusResponse, ReadRequest, ReadResponse, WriteRequest,
    WriteResponse, byte_stream_server,
};
use axl_proto::google::longrunning::{Operation, operation};
use axl_proto::google::rpc::Status as RpcStatus;
use prost::Message;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tonic::{Request, Response, Status, Streaming};

/// Extra delay before each RPC, to stand in for a distant server.
static LATENCY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Makes every cache and storage RPC take `ms` longer.
pub fn set_latency_ms(ms: u64) {
    LATENCY_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

async fn lag() {
    let ms = LATENCY_MS.load(std::sync::atomic::Ordering::Relaxed);
    if ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

/// What the server did, for the example scripts to show.
#[derive(Default)]
pub struct Counters {
    pub executions: AtomicU64,
    pub executions_cached: AtomicU64,
    pub action_cache_hits: AtomicU64,
    pub action_cache_misses: AtomicU64,
    pub action_cache_updates: AtomicU64,
    pub blobs_uploaded: AtomicU64,
    pub blobs_downloaded: AtomicU64,
    pub bytes_uploaded: AtomicU64,
    pub find_missing_calls: AtomicU64,
    pub bytestream_writes: AtomicU64,
    pub bytestream_reads: AtomicU64,
}

impl Counters {
    pub fn json(&self) -> String {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        format!(
            "{{\"executions\":{},\"executions_cached\":{},\"action_cache_hits\":{},\"action_cache_misses\":{},\"action_cache_updates\":{},\"blobs_uploaded\":{},\"blobs_downloaded\":{},\"bytes_uploaded\":{},\"find_missing_calls\":{},\"bytestream_writes\":{},\"bytestream_reads\":{}}}",
            g(&self.executions),
            g(&self.executions_cached),
            g(&self.action_cache_hits),
            g(&self.action_cache_misses),
            g(&self.action_cache_updates),
            g(&self.blobs_uploaded),
            g(&self.blobs_downloaded),
            g(&self.bytes_uploaded),
            g(&self.find_missing_calls),
            g(&self.bytestream_writes),
            g(&self.bytestream_reads),
        )
    }
}

#[derive(Default)]
pub struct State {
    blobs: Mutex<HashMap<String, Vec<u8>>>,
    results: Mutex<HashMap<String, ActionResult>>,
    pub counters: Counters,
    /// Largest blob a batch call accepts; larger ones go through ByteStream.
    pub max_batch_bytes: i64,
}

#[derive(Clone)]
pub struct Server {
    pub state: Arc<State>,
}

impl Server {
    pub fn new() -> Server {
        Server {
            state: Arc::new(State {
                max_batch_bytes: 4 * 1024 * 1024,
                ..Default::default()
            }),
        }
    }

    pub fn put(&self, data: Vec<u8>) -> Digest {
        let digest = reapi::digest_bytes(&data);
        self.state
            .blobs
            .lock()
            .unwrap()
            .insert(digest.hash.clone(), data);
        digest
    }

    pub fn get(&self, digest: &Digest) -> Option<Vec<u8>> {
        self.state.blobs.lock().unwrap().get(&digest.hash).cloned()
    }

    /// Serves on the connections `incoming` yields until the future is dropped.
    pub async fn serve_incoming<I, IO, IE>(self, incoming: I) -> Result<(), tonic::transport::Error>
    where
        I: tokio_stream::Stream<Item = Result<IO, IE>> + Send + 'static,
        IO: tokio::io::AsyncRead
            + tokio::io::AsyncWrite
            + tonic::transport::server::Connected
            + Unpin
            + Send
            + 'static,
        IE: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        tonic::transport::Server::builder()
            .add_service(
                content_addressable_storage_server::ContentAddressableStorageServer::new(
                    self.clone(),
                ),
            )
            .add_service(byte_stream_server::ByteStreamServer::new(self.clone()))
            .add_service(action_cache_server::ActionCacheServer::new(self.clone()))
            .add_service(capabilities_server::CapabilitiesServer::new(self.clone()))
            .add_service(execution_server::ExecutionServer::new(self))
            .serve_with_incoming(incoming)
            .await
    }
}

impl Default for Server {
    fn default() -> Self {
        Server::new()
    }
}

fn hash_of(d: &Option<Digest>) -> Result<&Digest, Status> {
    d.as_ref()
        .ok_or_else(|| Status::invalid_argument("missing digest"))
}

#[tonic::async_trait]
impl content_addressable_storage_server::ContentAddressableStorage for Server {
    async fn find_missing_blobs(
        &self,
        request: Request<FindMissingBlobsRequest>,
    ) -> Result<Response<FindMissingBlobsResponse>, Status> {
        lag().await;
        self.state
            .counters
            .find_missing_calls
            .fetch_add(1, Ordering::Relaxed);
        let blobs = self.state.blobs.lock().unwrap();
        let missing = request
            .into_inner()
            .blob_digests
            .into_iter()
            .filter(|d| !blobs.contains_key(&d.hash))
            .collect();
        Ok(Response::new(FindMissingBlobsResponse {
            missing_blob_digests: missing,
        }))
    }

    async fn batch_update_blobs(
        &self,
        request: Request<BatchUpdateBlobsRequest>,
    ) -> Result<Response<BatchUpdateBlobsResponse>, Status> {
        lag().await;
        let mut responses = Vec::new();
        for r in request.into_inner().requests {
            let digest = hash_of(&r.digest)?.clone();
            let actual = reapi::digest_bytes(&r.data);
            let status = if actual.hash == digest.hash && actual.size_bytes == digest.size_bytes {
                self.state
                    .counters
                    .blobs_uploaded
                    .fetch_add(1, Ordering::Relaxed);
                self.state
                    .counters
                    .bytes_uploaded
                    .fetch_add(r.data.len() as u64, Ordering::Relaxed);
                self.put(r.data);
                RpcStatus::default()
            } else {
                RpcStatus {
                    code: 3,
                    message: format!("digest mismatch for {}", digest.hash),
                    details: vec![],
                }
            };
            responses.push(batch_update_blobs_response::Response {
                digest: Some(digest),
                status: Some(status),
            });
        }
        Ok(Response::new(BatchUpdateBlobsResponse { responses }))
    }

    async fn batch_read_blobs(
        &self,
        request: Request<BatchReadBlobsRequest>,
    ) -> Result<Response<BatchReadBlobsResponse>, Status> {
        lag().await;
        let mut responses = Vec::new();
        for d in request.into_inner().digests {
            let data = self.get(&d);
            let status = RpcStatus {
                code: if data.is_some() { 0 } else { 5 },
                ..Default::default()
            };
            if data.is_some() {
                self.state
                    .counters
                    .blobs_downloaded
                    .fetch_add(1, Ordering::Relaxed);
            }
            responses.push(batch_read_blobs_response::Response {
                digest: Some(d),
                data: data.unwrap_or_default(),
                status: Some(status),
                ..Default::default()
            });
        }
        Ok(Response::new(BatchReadBlobsResponse { responses }))
    }

    type GetTreeStream = tokio_stream::Iter<std::vec::IntoIter<Result<GetTreeResponse, Status>>>;

    async fn get_tree(
        &self,
        _: Request<GetTreeRequest>,
    ) -> Result<Response<Self::GetTreeStream>, Status> {
        Err(Status::unimplemented("GetTree"))
    }
}

fn resource_digest(resource: &str, marker: &str) -> Result<Digest, Status> {
    let parts: Vec<&str> = resource.split('/').collect();
    let at = parts
        .iter()
        .position(|p| *p == marker)
        .ok_or_else(|| Status::invalid_argument(format!("bad resource name {resource}")))?;
    let hash = parts
        .get(at + 1)
        .ok_or_else(|| Status::invalid_argument("no hash"))?;
    let size = parts
        .get(at + 2)
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| Status::invalid_argument("no size"))?;
    Ok(Digest {
        hash: (*hash).to_string(),
        size_bytes: size,
    })
}

#[tonic::async_trait]
impl byte_stream_server::ByteStream for Server {
    type ReadStream = tokio_stream::Iter<std::vec::IntoIter<Result<ReadResponse, Status>>>;

    async fn read(
        &self,
        request: Request<ReadRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        lag().await;
        let digest = resource_digest(&request.get_ref().resource_name, "blobs")?;
        let data = self
            .get(&digest)
            .ok_or_else(|| Status::not_found(digest.hash.clone()))?;
        self.state
            .counters
            .bytestream_reads
            .fetch_add(1, Ordering::Relaxed);
        self.state
            .counters
            .blobs_downloaded
            .fetch_add(1, Ordering::Relaxed);
        let chunks: Vec<Result<ReadResponse, Status>> = if data.is_empty() {
            vec![Ok(ReadResponse { data: vec![] })]
        } else {
            data.chunks(64 * 1024)
                .map(|c| Ok(ReadResponse { data: c.to_vec() }))
                .collect()
        };
        Ok(Response::new(tokio_stream::iter(chunks)))
    }

    async fn write(
        &self,
        request: Request<Streaming<WriteRequest>>,
    ) -> Result<Response<WriteResponse>, Status> {
        lag().await;
        let mut stream = request.into_inner();
        let mut resource = String::new();
        let mut data = Vec::new();
        while let Some(message) = stream.message().await? {
            if resource.is_empty() {
                resource = message.resource_name.clone();
            }
            data.extend_from_slice(&message.data);
            if message.finish_write {
                break;
            }
        }
        let expected = resource_digest(&resource, "blobs")?;
        let actual = reapi::digest_bytes(&data);
        if actual.hash != expected.hash || actual.size_bytes != expected.size_bytes {
            return Err(Status::invalid_argument("digest mismatch"));
        }
        self.state
            .counters
            .bytestream_writes
            .fetch_add(1, Ordering::Relaxed);
        self.state
            .counters
            .blobs_uploaded
            .fetch_add(1, Ordering::Relaxed);
        self.state
            .counters
            .bytes_uploaded
            .fetch_add(data.len() as u64, Ordering::Relaxed);
        let size = data.len() as i64;
        self.put(data);
        Ok(Response::new(WriteResponse {
            committed_size: size,
        }))
    }

    async fn query_write_status(
        &self,
        _: Request<QueryWriteStatusRequest>,
    ) -> Result<Response<QueryWriteStatusResponse>, Status> {
        Err(Status::unimplemented("QueryWriteStatus"))
    }
}

#[tonic::async_trait]
impl action_cache_server::ActionCache for Server {
    async fn get_action_result(
        &self,
        request: Request<GetActionResultRequest>,
    ) -> Result<Response<ActionResult>, Status> {
        lag().await;
        let digest = hash_of(&request.get_ref().action_digest)?.clone();
        match self.state.results.lock().unwrap().get(&digest.hash) {
            Some(r) => {
                self.state
                    .counters
                    .action_cache_hits
                    .fetch_add(1, Ordering::Relaxed);
                Ok(Response::new(r.clone()))
            }
            None => {
                self.state
                    .counters
                    .action_cache_misses
                    .fetch_add(1, Ordering::Relaxed);
                Err(Status::not_found("no such action result"))
            }
        }
    }

    async fn update_action_result(
        &self,
        request: Request<UpdateActionResultRequest>,
    ) -> Result<Response<ActionResult>, Status> {
        lag().await;
        let request = request.into_inner();
        let digest = hash_of(&request.action_digest)?.clone();
        let result = request
            .action_result
            .ok_or_else(|| Status::invalid_argument("no action result"))?;
        self.state
            .counters
            .action_cache_updates
            .fetch_add(1, Ordering::Relaxed);
        self.state
            .results
            .lock()
            .unwrap()
            .insert(digest.hash, result.clone());
        Ok(Response::new(result))
    }
}

#[tonic::async_trait]
impl capabilities_server::Capabilities for Server {
    async fn get_capabilities(
        &self,
        _: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<ServerCapabilities>, Status> {
        lag().await;
        use axl_proto::build::bazel::remote::execution::v2::{
            ActionCacheUpdateCapabilities, CacheCapabilities, ExecutionCapabilities,
        };
        Ok(Response::new(ServerCapabilities {
            cache_capabilities: Some(CacheCapabilities {
                digest_functions: vec![1],
                action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                    update_enabled: true,
                }),
                max_batch_total_size_bytes: self.state.max_batch_bytes,
                ..Default::default()
            }),
            execution_capabilities: Some(ExecutionCapabilities {
                digest_function: 1,
                exec_enabled: true,
                digest_functions: vec![1],
                ..Default::default()
            }),
            low_api_version: Some(axl_proto::build::bazel::semver::SemVer {
                major: 2,
                ..Default::default()
            }),
            high_api_version: Some(axl_proto::build::bazel::semver::SemVer {
                major: 2,
                minor: 3,
                ..Default::default()
            }),
            ..Default::default()
        }))
    }
}

fn any_of<M: Message>(type_name: &str, m: &M) -> axl_proto::Any {
    axl_proto::Any {
        type_url: format!("type.googleapis.com/{type_name}"),
        value: m.encode_to_vec(),
    }
}

fn materialize(server: &Server, directory: &Directory, at: &Path) -> Result<(), Status> {
    std::fs::create_dir_all(at).map_err(|e| Status::internal(e.to_string()))?;
    for f in &directory.files {
        let digest = hash_of(&f.digest)?;
        let data = server.get(digest).ok_or_else(|| {
            Status::failed_precondition(format!("missing input blob {}", digest.hash))
        })?;
        let path = at.join(&f.name);
        std::fs::write(&path, data).map_err(|e| Status::internal(e.to_string()))?;
        if f.is_executable {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
    }
    for d in &directory.directories {
        let digest = hash_of(&d.digest)?;
        let bytes = server.get(digest).ok_or_else(|| {
            Status::failed_precondition(format!("missing directory {}", digest.hash))
        })?;
        let child =
            Directory::decode(bytes.as_slice()).map_err(|e| Status::internal(e.to_string()))?;
        materialize(server, &child, &at.join(&d.name))?;
    }
    for s in &directory.symlinks {
        let _ = std::os::unix::fs::symlink(&s.target, at.join(&s.name));
    }
    Ok(())
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>, bool)>, dirs: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut any = false;
    for e in entries.flatten() {
        any = true;
        let path = e.path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            walk(root, &path, out, dirs);
        } else if let Ok(data) = std::fs::read(&path) {
            use std::os::unix::fs::PermissionsExt;
            let exec = std::fs::metadata(&path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
            out.push((rel, data, exec));
        }
    }
    if !any && dir != root {
        dirs.push(
            dir.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
    }
}

fn run(server: &Server, action: &Action, id: u64) -> Result<ActionResult, Status> {
    let command_digest = hash_of(&action.command_digest)?;
    let command = Command::decode(
        server
            .get(command_digest)
            .ok_or_else(|| Status::failed_precondition("missing command"))?
            .as_slice(),
    )
    .map_err(|e| Status::internal(e.to_string()))?;
    let root_digest = hash_of(&action.input_root_digest)?;
    let root = Directory::decode(
        server
            .get(root_digest)
            .ok_or_else(|| Status::failed_precondition("missing input root"))?
            .as_slice(),
    )
    .map_err(|e| Status::internal(e.to_string()))?;

    let work: PathBuf =
        std::env::temp_dir().join(format!("fake-remote-{}-{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&work);
    materialize(server, &root, &work)?;
    for out in &command.output_paths {
        if let Some(parent) = Path::new(out).parent() {
            let _ = std::fs::create_dir_all(work.join(parent));
        }
    }
    let cwd = if command.working_directory.is_empty() {
        work.clone()
    } else {
        work.join(&command.working_directory)
    };
    let started = std::time::Instant::now();
    let Some((program, args)) = command.arguments.split_first() else {
        return Err(Status::invalid_argument("empty command"));
    };
    let mut process = std::process::Command::new(program);
    process.args(args).env_clear().current_dir(&cwd);
    for e in &command.environment_variables {
        process.env(&e.name, &e.value);
    }
    let output = process
        .output()
        .map_err(|e| Status::failed_precondition(format!("cannot run {program}: {e}")))?;

    let mut result = ActionResult {
        exit_code: output.status.code().unwrap_or(-1),
        execution_metadata: Some(ExecutedActionMetadata {
            worker: "fake-remote".into(),
            virtual_execution_duration: Some(axl_proto::Duration {
                seconds: started.elapsed().as_secs() as i64,
                nanos: started.elapsed().subsec_nanos() as i32,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    result.stdout_digest = Some(server.put(output.stdout.clone()));
    result.stderr_digest = Some(server.put(output.stderr.clone()));
    for path in &command.output_paths {
        let on_disk = work.join(path);
        if on_disk.is_dir() {
            let mut files = Vec::new();
            let mut empty = Vec::new();
            walk(&on_disk, &on_disk, &mut files, &mut empty);
            let mut inputs = Vec::new();
            for (rel, data, exec) in files {
                let d = server.put(data);
                inputs.push(Input {
                    path: rel,
                    entry: Entry::File {
                        digest: d.hash,
                        size: d.size_bytes,
                        executable: exec,
                    },
                    bytes: None,
                    source: None,
                });
            }
            for rel in empty {
                inputs.push(Input {
                    path: rel,
                    entry: Entry::Directory,
                    bytes: None,
                    source: None,
                });
            }
            let tree = reapi::input_tree(&inputs);
            for d in std::iter::once(&tree.root).chain(&tree.children) {
                server.put(d.encode_to_vec());
            }
            let message = reapi::tree_message(&tree);
            result.output_directories.push(OutputDirectory {
                path: path.clone(),
                tree_digest: Some(server.put(message.encode_to_vec())),
                ..Default::default()
            });
        } else if let Ok(data) = std::fs::read(&on_disk) {
            use std::os::unix::fs::PermissionsExt;
            let exec =
                std::fs::metadata(&on_disk).is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
            result.output_files.push(OutputFile {
                path: path.clone(),
                digest: Some(server.put(data)),
                is_executable: exec,
                ..Default::default()
            });
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    Ok(result)
}

#[tonic::async_trait]
impl execution_server::Execution for Server {
    type ExecuteStream = tokio_stream::Iter<std::vec::IntoIter<Result<Operation, Status>>>;

    async fn execute(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        let request = request.into_inner();
        let digest = hash_of(&request.action_digest)?.clone();
        let action = Action::decode(
            self.get(&digest)
                .ok_or_else(|| Status::failed_precondition("missing action"))?
                .as_slice(),
        )
        .map_err(|e| Status::internal(e.to_string()))?;
        let name = format!("operations/{}", digest.hash);
        let metadata = |stage: i32| {
            Some(any_of(
                "build.bazel.remote.execution.v2.ExecuteOperationMetadata",
                &ExecuteOperationMetadata {
                    stage,
                    action_digest: Some(digest.clone()),
                    ..Default::default()
                },
            ))
        };
        let queued = Operation {
            name: name.clone(),
            metadata: metadata(1),
            done: false,
            result: None,
        };

        let cached = if request.skip_cache_lookup || action.do_not_cache {
            None
        } else {
            self.state
                .results
                .lock()
                .unwrap()
                .get(&digest.hash)
                .cloned()
        };
        let (result, was_cached) = match cached {
            Some(r) => {
                self.state
                    .counters
                    .executions_cached
                    .fetch_add(1, Ordering::Relaxed);
                (r, true)
            }
            None => {
                let server = self.clone();
                let a = action.clone();
                let r = tokio::task::spawn_blocking(move || {
                    let id = server
                        .state
                        .counters
                        .executions
                        .fetch_add(1, Ordering::Relaxed);
                    run(&server, &a, id)
                })
                .await
                .map_err(|e| Status::internal(e.to_string()))??;
                if r.exit_code == 0 && !action.do_not_cache {
                    self.state
                        .results
                        .lock()
                        .unwrap()
                        .insert(digest.hash.clone(), r.clone());
                }
                (r, false)
            }
        };
        eprintln!(
            "fake-remote: execute {} -> exit {}{}",
            &digest.hash[..12],
            result.exit_code,
            if was_cached { " (cached)" } else { "" }
        );
        let response = ExecuteResponse {
            result: Some(result),
            cached_result: was_cached,
            ..Default::default()
        };
        let done = Operation {
            name,
            metadata: metadata(4),
            done: true,
            result: Some(operation::Result::Response(any_of(
                "build.bazel.remote.execution.v2.ExecuteResponse",
                &response,
            ))),
        };
        Ok(Response::new(tokio_stream::iter(vec![
            Ok(queued),
            Ok(done),
        ])))
    }

    type WaitExecutionStream = tokio_stream::Iter<std::vec::IntoIter<Result<Operation, Status>>>;

    async fn wait_execution(
        &self,
        _: Request<axl_proto::build::bazel::remote::execution::v2::WaitExecutionRequest>,
    ) -> Result<Response<Self::WaitExecutionStream>, Status> {
        Err(Status::unimplemented("WaitExecution"))
    }
}
