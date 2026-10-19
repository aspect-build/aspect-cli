//! A REAPI store: the content-addressable storage and the action cache of one
//! endpoint and instance, over the `axl-proto` clients.
//!
//! Calls are blocking: Bazel's threads call in, and a runtime of the store's
//! own drives the connection, one lazily established channel per endpoint.
//! Small blobs use the batch calls, blobs over the server's batch limit use
//! ByteStream. Compression is off. Headers are sent as given and never logged.

use anyhow::{Result, anyhow, bail};
use axl_proto::build::bazel::remote::execution::v2::{
    ActionResult, BatchReadBlobsRequest, BatchUpdateBlobsRequest, Digest, FindMissingBlobsRequest,
    GetActionResultRequest, GetCapabilitiesRequest, UpdateActionResultRequest,
    action_cache_client::ActionCacheClient, batch_update_blobs_request,
    capabilities_client::CapabilitiesClient,
    content_addressable_storage_client::ContentAddressableStorageClient,
};
use axl_proto::google::bytestream::{
    ReadRequest, WriteRequest, byte_stream_client::ByteStreamClient,
};
use std::sync::{Arc, OnceLock};
use tokio::runtime::Runtime;
use tonic::Request;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

const DEFAULT_BATCH_BYTES: i64 = 4 * 1024 * 1024;
const CHUNK: usize = 256 * 1024;
const FIND_MISSING_BATCH: usize = 5000;

pub struct Store {
    runtime: Runtime,
    channel: Channel,
    endpoint: String,
    instance: String,
    headers: Vec<(String, String)>,
    max_batch: OnceLock<i64>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Store({}, instance {:?})", self.endpoint, self.instance)
    }
}

/// `grpc://host:port` and `grpcs://host:port` as the URI tonic wants.
fn uri(endpoint: &str) -> String {
    if let Some(rest) = endpoint.strip_prefix("grpcs://") {
        format!("https://{rest}")
    } else if let Some(rest) = endpoint.strip_prefix("grpc://") {
        format!("http://{rest}")
    } else {
        endpoint.to_string()
    }
}

impl Store {
    pub fn connect(
        endpoint: &str,
        instance: &str,
        headers: Vec<(String, String)>,
    ) -> Result<Arc<Store>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("buildx-store")
            .build()?;
        let channel = {
            let _guard = runtime.enter();
            let target = uri(endpoint);
            let mut builder = Endpoint::from_shared(target.clone())
                .map_err(|e| anyhow!("invalid endpoint {endpoint:?}: {e}"))?;
            if target.starts_with("https://") {
                builder = builder
                    .tls_config(ClientTlsConfig::new().with_native_roots())
                    .map_err(|e| anyhow!("TLS setup for {endpoint:?}: {e}"))?;
            }
            builder.connect_lazy()
        };
        Ok(Arc::new(Store {
            runtime,
            channel,
            endpoint: endpoint.to_string(),
            instance: instance.to_string(),
            headers,
            max_batch: OnceLock::new(),
        }))
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The names of the headers every call carries.
    pub fn header_names(&self) -> Vec<String> {
        self.headers.iter().map(|(k, _)| k.clone()).collect()
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    fn request<T>(&self, message: T) -> Result<Request<T>> {
        let mut request = Request::new(message);
        for (k, v) in &self.headers {
            let key = tonic::metadata::MetadataKey::from_bytes(k.to_lowercase().as_bytes())
                .map_err(|_| anyhow!("invalid header name {k:?}"))?;
            let value = v
                .parse()
                .map_err(|_| anyhow!("invalid value for header {k:?}"))?;
            request.metadata_mut().insert(key, value);
        }
        Ok(request)
    }

    fn block<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self.runtime.block_on(f)
    }

    fn status(&self, what: &str, e: tonic::Status) -> anyhow::Error {
        anyhow!("{what} on {}: {}: {}", self.endpoint, e.code(), e.message())
    }

    /// The largest blob the server takes in a batch call.
    fn max_batch(&self) -> i64 {
        *self.max_batch.get_or_init(|| {
            let mut client = CapabilitiesClient::new(self.channel.clone());
            let request = self.request(GetCapabilitiesRequest {
                instance_name: self.instance.clone(),
            });
            let Ok(request) = request else {
                return DEFAULT_BATCH_BYTES;
            };
            match self.block(client.get_capabilities(request)) {
                Ok(r) => r
                    .into_inner()
                    .cache_capabilities
                    .map(|c| c.max_batch_total_size_bytes)
                    .filter(|n| *n > 0)
                    .unwrap_or(DEFAULT_BATCH_BYTES),
                Err(_) => DEFAULT_BATCH_BYTES,
            }
        })
    }

    pub fn find_missing(&self, digests: &[Digest]) -> Result<Vec<Digest>> {
        let mut client = ContentAddressableStorageClient::new(self.channel.clone());
        let mut missing = Vec::new();
        for chunk in digests.chunks(FIND_MISSING_BATCH) {
            let request = self.request(FindMissingBlobsRequest {
                instance_name: self.instance.clone(),
                blob_digests: chunk.to_vec(),
                digest_function: 0,
            })?;
            let response = self
                .block(client.find_missing_blobs(request))
                .map_err(|e| self.status("FindMissingBlobs", e))?;
            missing.extend(response.into_inner().missing_blob_digests);
        }
        Ok(missing)
    }

    /// Uploads blobs (digest, bytes): small ones in batches, large ones by ByteStream.
    pub fn upload(&self, blobs: Vec<(Digest, Arc<Vec<u8>>)>) -> Result<()> {
        let limit = self.max_batch();
        let mut batch: Vec<batch_update_blobs_request::Request> = Vec::new();
        let mut batch_bytes = 0i64;
        for (digest, data) in blobs {
            if digest.size_bytes >= limit {
                self.write_stream(&digest, &data)?;
                continue;
            }
            if batch_bytes + digest.size_bytes >= limit && !batch.is_empty() {
                self.flush(std::mem::take(&mut batch))?;
                batch_bytes = 0;
            }
            batch_bytes += digest.size_bytes;
            batch.push(batch_update_blobs_request::Request {
                digest: Some(digest),
                data: data.as_ref().clone(),
                compressor: 0,
            });
        }
        if !batch.is_empty() {
            self.flush(batch)?;
        }
        Ok(())
    }

    fn flush(&self, requests: Vec<batch_update_blobs_request::Request>) -> Result<()> {
        let mut client = ContentAddressableStorageClient::new(self.channel.clone());
        let request = self.request(BatchUpdateBlobsRequest {
            instance_name: self.instance.clone(),
            requests,
            digest_function: 0,
        })?;
        let response = self
            .block(client.batch_update_blobs(request))
            .map_err(|e| self.status("BatchUpdateBlobs", e))?;
        for r in response.into_inner().responses {
            if let Some(s) = r.status {
                if s.code != 0 {
                    bail!(
                        "BatchUpdateBlobs on {} failed for {}: {}",
                        self.endpoint,
                        r.digest.map(|d| d.hash).unwrap_or_default(),
                        s.message
                    );
                }
            }
        }
        Ok(())
    }

    fn resource(&self, tail: &str) -> String {
        if self.instance.is_empty() {
            tail.to_string()
        } else {
            format!("{}/{tail}", self.instance)
        }
    }

    fn write_stream(&self, digest: &Digest, data: &[u8]) -> Result<()> {
        let mut client = ByteStreamClient::new(self.channel.clone());
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let name = self.resource(&format!(
            "uploads/{:x}-{:x}/blobs/{}/{}",
            std::process::id(),
            nanos,
            digest.hash,
            digest.size_bytes
        ));
        let total = data.len();
        let messages: Vec<WriteRequest> = if total == 0 {
            vec![WriteRequest {
                resource_name: name,
                write_offset: 0,
                finish_write: true,
                data: vec![],
            }]
        } else {
            data.chunks(CHUNK)
                .enumerate()
                .map(|(i, chunk)| WriteRequest {
                    resource_name: if i == 0 { name.clone() } else { String::new() },
                    write_offset: (i * CHUNK) as i64,
                    finish_write: (i + 1) * CHUNK >= total,
                    data: chunk.to_vec(),
                })
                .collect()
        };
        let request = self.request(tokio_stream::iter(messages))?;
        self.block(client.write(request))
            .map_err(|e| self.status("ByteStream Write", e))?;
        Ok(())
    }

    /// The bytes of a blob.
    pub fn download(&self, digest: &Digest) -> Result<Vec<u8>> {
        if digest.size_bytes == 0 {
            return Ok(Vec::new());
        }
        if digest.size_bytes >= self.max_batch() {
            return self.read_stream(digest);
        }
        let mut client = ContentAddressableStorageClient::new(self.channel.clone());
        let request = self.request(BatchReadBlobsRequest {
            instance_name: self.instance.clone(),
            digests: vec![digest.clone()],
            acceptable_compressors: vec![],
            digest_function: 0,
        })?;
        let response = self
            .block(client.batch_read_blobs(request))
            .map_err(|e| self.status("BatchReadBlobs", e))?;
        let Some(r) = response.into_inner().responses.into_iter().next() else {
            bail!(
                "BatchReadBlobs on {} returned nothing for {}",
                self.endpoint,
                digest.hash
            );
        };
        if r.status.as_ref().is_some_and(|s| s.code != 0) {
            bail!("blob {} is not in {}", digest.hash, self.endpoint);
        }
        Ok(r.data)
    }

    fn read_stream(&self, digest: &Digest) -> Result<Vec<u8>> {
        let mut client = ByteStreamClient::new(self.channel.clone());
        let request = self.request(ReadRequest {
            resource_name: self.resource(&format!("blobs/{}/{}", digest.hash, digest.size_bytes)),
            read_offset: 0,
            read_limit: 0,
        })?;
        self.block(async {
            let mut stream = client.read(request).await?.into_inner();
            let mut data = Vec::with_capacity(digest.size_bytes as usize);
            while let Some(m) = stream.message().await? {
                data.extend_from_slice(&m.data);
            }
            Ok::<_, tonic::Status>(data)
        })
        .map_err(|e| self.status("ByteStream Read", e))
    }

    /// The cached result of an action, None on a miss.
    pub fn get_action_result(&self, action: &Digest) -> Result<Option<ActionResult>> {
        let mut client = ActionCacheClient::new(self.channel.clone());
        let request = self.request(GetActionResultRequest {
            instance_name: self.instance.clone(),
            action_digest: Some(action.clone()),
            inline_stdout: false,
            inline_stderr: false,
            inline_output_files: vec![],
            digest_function: 0,
        })?;
        match self.block(client.get_action_result(request)) {
            Ok(r) => Ok(Some(r.into_inner())),
            Err(e) if e.code() == tonic::Code::NotFound => Ok(None),
            Err(e) => Err(self.status("GetActionResult", e)),
        }
    }

    pub fn update_action_result(&self, action: &Digest, result: &ActionResult) -> Result<()> {
        let mut client = ActionCacheClient::new(self.channel.clone());
        let request = self.request(UpdateActionResultRequest {
            instance_name: self.instance.clone(),
            action_digest: Some(action.clone()),
            action_result: Some(result.clone()),
            results_cache_policy: None,
            digest_function: 0,
        })?;
        self.block(client.update_action_result(request))
            .map_err(|e| self.status("UpdateActionResult", e))?;
        Ok(())
    }

    pub(crate) fn channel(&self) -> Channel {
        self.channel.clone()
    }

    pub(crate) fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub(crate) fn request_for<T>(&self, message: T) -> Result<Request<T>> {
        self.request(message)
    }
}
