//! Build event sinks: where the Build Event Protocol stream of a `ctx.build`
//! goes. A sink is a value a script passes per build, `events = [...]`; the
//! engine turns each into one of Bazel's own transports.

use crate::store::Store;
use allocative::Allocative;
use anyhow::{Result, bail};
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::starlark_value;
use starlark::values::{NoSerialize, ProvidesStaticType, StarlarkValue};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// A Build Event Service sink.
pub struct Bes {
    /// `grpc://host:port` or `grpcs://host:port`.
    pub endpoint: String,
    /// Sent with every call; never logged.
    pub headers: Vec<(String, String)>,
    /// The BES instance (project) name.
    pub instance: String,
    /// A base URL; the invocation id is appended to print where results are.
    pub results_url: String,
    /// What `--build_metadata` does.
    pub metadata: Vec<(String, String)>,
    /// Upload referenced files to this store and rewrite URIs to `bytestream://`.
    pub artifacts: Option<Arc<Store>>,
}

impl fmt::Debug for Bes {
    /// Header values are credentials: only their names are shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bes")
            .field("endpoint", &self.endpoint)
            .field(
                "headers",
                &self.headers.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            )
            .field("instance", &self.instance)
            .field("results_url", &self.results_url)
            .field("metadata", &self.metadata)
            .field("artifacts", &self.artifacts)
            .finish()
    }
}

#[derive(Debug)]
pub enum Sink {
    Bes(Bes),
    /// A binary BEP file.
    File(PathBuf),
}

/// `ctx.events`: sink constructors.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Events;

impl fmt::Display for Events {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Events")
    }
}

starlark::starlark_simple_value!(Events);

#[starlark_value(type = "Events")]
impl<'v> StarlarkValue<'v> for Events {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("events_methods", events_methods);
        Some(RES.methods())
    }
}

/// A sink, from `ctx.events.bes` or `ctx.events.file`.
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct SinkValue(#[allocative(skip)] pub Arc<Sink>);

impl fmt::Debug for SinkValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for SinkValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.0 {
            Sink::Bes(b) => write!(f, "EventSink(bes, {:?})", b.endpoint),
            Sink::File(p) => write!(f, "EventSink(file, {:?})", p.display()),
        }
    }
}

starlark::starlark_simple_value!(SinkValue);

#[starlark_value(type = "EventSink")]
impl<'v> StarlarkValue<'v> for SinkValue {}

/// gRPC metadata: a name of letters, digits, `-`, `_` or `.` and a printable
/// ASCII value. The wire form of a name is lowercase, so callers normalize it
/// with `ascii_lowercase` first, as the store does, and `X-Aspect` works.
fn check_header(name: &str, value: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        bail!("events.bes: header name {name:?} must be letters, digits, `-`, `_` or `.`");
    }
    if name.starts_with("grpc-") {
        bail!("events.bes: header name {name:?} is reserved by gRPC");
    }
    if !value.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        bail!("events.bes: the value of header {name:?} must be printable ASCII");
    }
    Ok(())
}

fn pairs(what: &str, d: Option<UnpackDictEntries<&str, &str>>) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (k, v) in d.map(|d| d.entries).unwrap_or_default() {
        if k.is_empty() {
            bail!("events.bes: {what} has an empty name");
        }
        out.push((k.to_string(), v.to_string()));
    }
    Ok(out)
}

/// Checks `endpoint` is `grpc://host:port` or `grpcs://host:port`.
pub fn check_endpoint(endpoint: &str) -> Result<()> {
    if endpoint.trim().is_empty() {
        bail!("events.bes: the endpoint is empty; pass grpc://host:port or grpcs://host:port");
    }
    let Some(rest) = endpoint
        .strip_prefix("grpc://")
        .or_else(|| endpoint.strip_prefix("grpcs://"))
    else {
        bail!("events.bes: endpoint {endpoint:?} must start with grpc:// or grpcs://");
    };
    if rest.is_empty() || rest.starts_with('/') {
        bail!("events.bes: endpoint {endpoint:?} has no host");
    }
    Ok(())
}

/// Checks `path` can be written: its directory exists and a file can be created in it.
pub fn check_writable(path: &str) -> Result<PathBuf> {
    if path.is_empty() {
        bail!("events.file: the path is empty");
    }
    let p = PathBuf::from(path);
    let p = if p.is_absolute() {
        p
    } else {
        std::env::current_dir()?.join(p)
    };
    if p.is_dir() {
        bail!("events.file: {path:?} is a directory");
    }
    let dir = p.parent().map(PathBuf::from).unwrap_or_default();
    if !dir.is_dir() {
        bail!("events.file: the directory of {path:?} does not exist");
    }
    if let Err(e) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        bail!("events.file: {path:?} is not writable: {e}");
    }
    Ok(p)
}

#[starlark_module]
fn events_methods(registry: &mut MethodsBuilder) {
    /// A Build Event Service sink: the build's event stream goes to the BES
    /// server at `endpoint` (`grpc://host:port` or `grpcs://host:port`) as one
    /// invocation. `headers` are sent with every call and never logged;
    /// `instance` names the BES instance; `metadata` is what `--build_metadata`
    /// does; `results_url` is a base URL, and after the build
    /// `buildx: results: <results_url><invocation id>` is printed.
    /// `artifacts` (a `ctx.execution.store`) uploads each referenced file and
    /// names it by `bytestream://`; without it, files are named by local
    /// `file://` URIs.
    fn bes<'v>(
        this: &Events,
        endpoint: &str,
        #[starlark(require = named)] headers: Option<UnpackDictEntries<&'v str, &'v str>>,
        #[starlark(require = named)] instance: Option<&str>,
        #[starlark(require = named)] results_url: Option<&str>,
        #[starlark(require = named)] metadata: Option<UnpackDictEntries<&'v str, &'v str>>,
        #[starlark(require = named)] artifacts: Option<&crate::values::StoreValue>,
    ) -> anyhow::Result<SinkValue> {
        let _ = this;
        check_endpoint(endpoint)?;
        Ok(SinkValue(Arc::new(Sink::Bes(Bes {
            endpoint: endpoint.to_string(),
            headers: {
                let mut headers = pairs("a header", headers)?;
                for (k, v) in &mut headers {
                    check_header(k, v)?;
                    *k = k.to_ascii_lowercase();
                }
                headers.sort();
                if let Some(w) = headers.windows(2).find(|w| w[0].0 == w[1].0) {
                    bail!(
                        "events.bes: header {:?} is given twice (header names are case-insensitive)",
                        w[0].0
                    );
                }
                headers
            },
            instance: instance.unwrap_or("").to_string(),
            results_url: results_url.unwrap_or("").to_string(),
            metadata: pairs("a metadata key", metadata)?,
            artifacts: artifacts.map(|s| s.0.clone()),
        }))))
    }

    /// A binary BEP file at `path`: delimited `BuildEvent` messages, what
    /// `--build_event_binary_file` writes.
    fn file(this: &Events, path: &str) -> anyhow::Result<SinkValue> {
        let _ = this;
        Ok(SinkValue(Arc::new(Sink::File(check_writable(path)?))))
    }
}

/// A REAPI endpoint a build used.
#[derive(Debug, Clone, PartialEq)]
pub struct Remote {
    /// As the script wrote it, `grpcs://host:port`.
    pub endpoint: String,
    pub instance: String,
    /// The names of the headers sent, never their values.
    pub headers: Vec<String>,
}

impl Remote {
    fn of(store: &Store) -> Remote {
        Remote {
            endpoint: store.endpoint().to_string(),
            instance: store.instance().to_string(),
            headers: store.header_names(),
        }
    }
}

/// The remote services the executors of a build can reach, which the command
/// line of the event stream states as `remote_cache` and `remote_executor`
/// options (what a Bazel UI reads to tell a build used a remote cache).
#[derive(Debug, Clone, Default)]
pub struct Used {
    /// Stores asked for cached results, and given the results of local runs.
    pub caches: Vec<Remote>,
    /// Servers that run actions.
    pub executors: Vec<Remote>,
}

impl Used {
    pub fn cache(&mut self, store: &Store) {
        let remote = Remote::of(store);
        if !self.caches.contains(&remote) {
            self.caches.push(remote);
        }
    }

    pub fn executor(&mut self, store: &Store) {
        let remote = Remote::of(store);
        if !self.executors.contains(&remote) {
            self.executors.push(remote);
        }
    }
}

/// What the command line of a build is, as the build event stream states it.
#[derive(Debug, Clone, Default)]
pub struct Invocation {
    /// What the build's executors reach.
    pub used: Used,
    /// The script, `path.star:main`.
    pub script: String,
    /// Settings of the configuration as `name=value`.
    pub settings: Vec<String>,
    /// The requested targets or patterns.
    pub targets: Vec<String>,
}
