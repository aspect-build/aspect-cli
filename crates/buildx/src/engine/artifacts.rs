//! `ctx.events.bes(..., artifacts = store)`: a `BuildEventArtifactUploader`
//! written in Rust over a REAPI `Store`. Each file a build event references is
//! uploaded to the store and named by a `bytestream://` URI; whatever cannot be
//! uploaded (directories, symlinks, a failed upload) keeps the local `file://`
//! URI that Bazel's own `LocalFilesArtifactUploader` gives it. A failed upload
//! never fails the build: it is counted, shown in the stream's progress and
//! reported on stderr when the stream ends.

use super::*;
use crate::reapi::digest_bytes;
use crate::store::Store;
use blazon_rt::Result as RtResult;
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};

/// Worker threads uploading files for one uploader, so also the files uploading
/// at once. A fixed pool: Bazel asks for an upload per named set, thousands at a
/// time, and a thread per request exhausts the process's threads (macOS: 8192).
/// Uploads wait on the network, so this is well above the core count.
const UPLOAD_WORKERS: usize = 32;

/// What the uploads of one build did.
#[derive(Default)]
pub(super) struct Report {
    uploaded: AtomicU64,
    failed: AtomicU64,
    messages: Mutex<Vec<String>>,
    /// Failures not yet shown in a progress event.
    unseen: Mutex<Vec<String>>,
}

impl Report {
    fn fail(&self, message: String) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        let mut messages = self.messages.lock().unwrap();
        if messages.len() < 5 {
            messages.push(message.clone());
        }
        self.unseen.lock().unwrap().push(message);
    }

    /// The summary for stderr, when anything failed.
    pub(super) fn summary(&self) -> Option<String> {
        let failed = self.failed.load(Ordering::Relaxed);
        if failed == 0 {
            return None;
        }
        let uploaded = self.uploaded.load(Ordering::Relaxed);
        let first = self.messages.lock().unwrap().first().cloned()?;
        Some(format!(
            "{failed} artifact uploads failed ({uploaded} succeeded), those files keep their file:// URIs; first: {first}"
        ))
    }
}

/// The files one `upload` call from Bazel asked for. The worker that finishes
/// the last of them completes the call's future.
struct Batch {
    remaining: AtomicUsize,
    uris: Mutex<HashMap<String, String>>,
    result: Global<bazel::SettableFuture<'static, bazel::PathConverter<'static>>>,
    local: Mutex<Option<Global<bazel::PathConverter<'static>>>>,
}

/// One file to upload, and the call it belongs to.
struct Job {
    path: String,
    batch: Arc<Batch>,
}

/// What every worker shares.
struct Shared {
    store: Arc<Store>,
    prefix: String,
    report: Arc<Report>,
    /// Each path is uploaded once per build, however many events reference it.
    slots: Arc<Slots>,
}

impl Shared {
    fn run(&self, job: Job) {
        let slot = self
            .slots
            .lock()
            .unwrap()
            .entry(job.path.clone())
            .or_default()
            .clone();
        let mut mine = false;
        let uploaded = slot.get_or_init(|| {
            mine = true;
            upload_file(&self.store, &self.prefix, &job.path)
        });
        match uploaded {
            Ok(uri) => {
                if mine {
                    self.report.uploaded.fetch_add(1, Ordering::Relaxed);
                }
                job.batch
                    .uris
                    .lock()
                    .unwrap()
                    .insert(job.path.clone(), uri.clone());
            }
            Err(message) if mine => self.report.fail(message.clone()),
            Err(_) => {}
        }
        if job.batch.remaining.fetch_sub(1, Ordering::AcqRel) == 1 {
            finish_batch(&job.batch, &self.report);
        }
    }
}

/// Completes the call's future with a converter naming what was uploaded.
fn finish_batch(batch: &Batch, report: &Report) {
    let done = || -> RtResult<()> {
        let local = batch
            .local
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| blazon_rt::Error::msg("the upload was already completed"))?;
        let uris = std::mem::take(&mut *batch.uris.lock().unwrap());
        let mut env = blazon_rt::vm::env()?;
        let converter =
            bazel::PathConverter::from_impl(&mut env, Box::new(Converter { uris, local }))?;
        batch.result.get(&mut env)?.set(&converter)?;
        Ok(())
    };
    if let Err(e) = done() {
        report.fail(format!("completing an upload: {e:#}"));
    }
}

pub(super) struct StoreUploader {
    local: Global<bazel::LocalFilesArtifactUploader<'static>>,
    me: Arc<Mutex<Option<Global<bazel::ReferenceCounted<'static>>>>>,
    refs: AtomicI32,
    report: Arc<Report>,
    /// The workers' queue; dropping it, with the uploader, stops them.
    queue: Sender<Job>,
}

type Slots = Mutex<HashMap<String, Arc<std::sync::OnceLock<std::result::Result<String, String>>>>>;

impl StoreUploader {
    /// The uploader as the Java object Bazel's transport holds.
    pub(super) fn create<'l>(
        env: &mut JNIEnv<'l>,
        store: Arc<Store>,
        report: Arc<Report>,
    ) -> Result<bazel::BuildEventArtifactUploader<'l>> {
        let endpoint = store.endpoint();
        let authority = endpoint
            .strip_prefix("grpcs://")
            .or_else(|| endpoint.strip_prefix("grpc://"))
            .unwrap_or(endpoint)
            .trim_end_matches('/')
            .to_string();
        let prefix = if store.instance().is_empty() {
            format!("bytestream://{authority}")
        } else {
            format!("bytestream://{authority}/{}", store.instance())
        };
        let local = bazel::LocalFilesArtifactUploader::new(env)?;
        let me = Arc::new(Mutex::new(None));
        let (queue, jobs) = std::sync::mpsc::channel::<Job>();
        let jobs: Arc<Mutex<Receiver<Job>>> = Arc::new(Mutex::new(jobs));
        let shared = Arc::new(Shared {
            store,
            prefix,
            report: report.clone(),
            slots: Arc::new(Mutex::new(HashMap::new())),
        });
        for i in 0..UPLOAD_WORKERS {
            let (jobs, shared) = (jobs.clone(), shared.clone());
            std::thread::Builder::new()
                .name(format!("buildx-bep-upload-{i}"))
                .spawn(move || {
                    loop {
                        let job = jobs.lock().unwrap().recv();
                        match job {
                            Ok(job) => shared.run(job),
                            Err(_) => return,
                        }
                    }
                })
                .map_err(|e| anyhow::anyhow!("starting the artifact upload workers: {e}"))?;
        }
        let uploader = Box::new(StoreUploader {
            local: local.to_global()?,
            me: me.clone(),
            refs: AtomicI32::new(1),
            report,
            queue,
        });
        let object = bazel::BuildEventArtifactUploader::from_impl(env, uploader)?;
        // `retain` and `touch` return the object itself, as `ReferenceCounted` says; the
        // reference is dropped when the last release comes.
        *me.lock().unwrap() = Some(object.upcast::<bazel::ReferenceCounted>().to_global()?);
        Ok(object)
    }

    fn myself<'a>(&self, env: &mut JNIEnv<'a>) -> RtResult<bazel::ReferenceCounted<'a>> {
        match &*self.me.lock().unwrap() {
            Some(me) => me.get(env),
            None => Err(blazon_rt::Error::msg("the uploader was released")),
        }
    }
}

struct Converter {
    uris: HashMap<String, String>,
    local: Global<bazel::PathConverter<'static>>,
}

impl bazel::PathConverterImpl for Converter {
    fn apply<'a>(&self, env: &mut JNIEnv<'a>, path: bazel::Path<'a>) -> RtResult<Option<String>> {
        if let Some(uri) = self.uris.get(&path.path_string()?) {
            return Ok(Some(uri.clone()));
        }
        self.local.get(env)?.apply(&path)
    }
}

/// Uploads `path`'s content unless the store has it; its `bytestream://` URI.
fn upload_file(store: &Store, prefix: &str, path: &str) -> std::result::Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let digest = digest_bytes(&bytes);
    let missing = store
        .find_missing(std::slice::from_ref(&digest))
        .map_err(|e| format!("{path}: {e:#}"))?;
    if !missing.is_empty() {
        store
            .upload(vec![(digest.clone(), Arc::new(bytes))])
            .map_err(|e| format!("{path}: {e:#}"))?;
    }
    Ok(format!(
        "{prefix}/blobs/{}/{}",
        digest.hash, digest.size_bytes
    ))
}

impl bazel::ArtifactUploaderImpl for StoreUploader {
    fn upload<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        files: bazel::Map<'a, bazel::Path<'static>, bazel::LocalFile<'static>>,
    ) -> RtResult<bazel::ListenableFuture<'a, bazel::PathConverter<'static>>> {
        let local = {
            let future = self.local.get(env)?.upload(&files)?;
            let fluent = bazel::FluentFuture::from(env, &future)?;
            fluent
                .downcast::<bazel::AbstractFuture<bazel::PathConverter>>()?
                .get()?
                .ok_or_else(|| blazon_rt::Error::msg("no local path converter"))?
                .to_global()?
        };
        let mut todo = Vec::new();
        for entry in &files.entries()? {
            let entry = entry?;
            let kind = entry.value()?.kind()?.to_string();
            if kind == "OUTPUT_DIRECTORY" || kind == "OUTPUT_SYMLINK" {
                continue;
            }
            todo.push(entry.key()?.path_string()?);
        }
        let future = bazel::SettableFuture::create::<bazel::PathConverter>(env)?;
        let batch = Arc::new(Batch {
            remaining: AtomicUsize::new(todo.len()),
            uris: Mutex::new(HashMap::new()),
            result: future.to_global()?,
            local: Mutex::new(Some(local)),
        });
        if todo.is_empty() {
            finish_batch(&batch, &self.report);
            return Ok(future.upcast());
        }
        for path in todo {
            self.queue
                .send(Job {
                    path,
                    batch: batch.clone(),
                })
                .map_err(|_| blazon_rt::Error::msg("the artifact upload workers have stopped"))?;
        }
        Ok(future.upcast())
    }

    fn may_be_slow<'a>(&self, _env: &mut JNIEnv<'a>) -> RtResult<bool> {
        Ok(true)
    }

    fn ref_cnt<'a>(&self, _env: &mut JNIEnv<'a>) -> RtResult<i32> {
        Ok(self.refs.load(Ordering::SeqCst))
    }

    fn retain<'a>(&self, env: &mut JNIEnv<'a>) -> RtResult<bazel::ReferenceCounted<'a>> {
        self.refs.fetch_add(1, Ordering::SeqCst);
        self.myself(env)
    }

    fn retain_n<'a>(&self, env: &mut JNIEnv<'a>, n: i32) -> RtResult<bazel::ReferenceCounted<'a>> {
        self.refs.fetch_add(n, Ordering::SeqCst);
        self.myself(env)
    }

    fn touch<'a>(&self, env: &mut JNIEnv<'a>) -> RtResult<bazel::ReferenceCounted<'a>> {
        self.myself(env)
    }

    fn touch_with<'a>(
        &self,
        env: &mut JNIEnv<'a>,
        _hint: bazel::Object<'a>,
    ) -> RtResult<bazel::ReferenceCounted<'a>> {
        self.myself(env)
    }

    fn release<'a>(&self, env: &mut JNIEnv<'a>) -> RtResult<bool> {
        self.release_n(env, 1)
    }

    fn release_n<'a>(&self, _env: &mut JNIEnv<'a>, n: i32) -> RtResult<bool> {
        let before = self.refs.fetch_sub(n, Ordering::SeqCst);
        if before - n <= 0 {
            *self.me.lock().unwrap() = None;
            return Ok(true);
        }
        Ok(false)
    }
}

/// What the streamer's progress events carry: the upload failures not yet shown.
pub(super) struct Progress(pub Arc<Report>);

impl bazel::OutErrImpl for Progress {
    fn out<'a>(
        &self,
        env: &mut JNIEnv<'a>,
    ) -> RtResult<bazel::Iterable<'a, blazon_rt::java::lang::String<'static>>> {
        Ok(bazel::List::of_strs(env, &[] as &[&str])?.upcast())
    }

    fn err<'a>(
        &self,
        env: &mut JNIEnv<'a>,
    ) -> RtResult<bazel::Iterable<'a, blazon_rt::java::lang::String<'static>>> {
        let unseen: Vec<String> = std::mem::take(&mut *self.0.unseen.lock().unwrap());
        let lines: Vec<String> = unseen
            .into_iter()
            .map(|m| format!("buildx: events: artifact upload failed: {m}\n"))
            .collect();
        Ok(bazel::List::of_strs(env, &lines)?.upcast())
    }
}
