use std::collections::HashSet;
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use allocative::Allocative;
use anyhow::{Context as _, anyhow};
use starlark::any::ProvidesStaticType;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{
    Heap, NoSerialize, StarlarkValue, Value, ValueLike, none::NoneType, starlark_value,
};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::oneshot;
use tokio_stream::wrappers::{TcpListenerStream, UnixListenerStream};

use super::service::{GrpcService, ServiceHandle};

/// Builder + serve gateway for a gRPC server. Mutable until `serve()` is
/// called; once consumed, further `add`/`serve` calls raise. Constructed
/// via `grpc.Server(endpoint = ...)`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct GrpcServer {
    endpoint: String,

    #[allocative(skip)]
    state: Mutex<BuilderState>,
}

#[derive(Debug)]
struct BuilderState {
    services: Vec<Arc<dyn ServiceHandle>>,
    seen: HashSet<&'static str>,
    consumed: bool,
}

impl GrpcServer {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            state: Mutex::new(BuilderState {
                services: Vec::new(),
                seen: HashSet::new(),
                consumed: false,
            }),
        }
    }
}

impl fmt::Display for GrpcServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "grpc.Server({:?})", self.endpoint)
    }
}

starlark_simple_value!(GrpcServer);

#[starlark_value(type = "grpc.Server")]
impl<'v> StarlarkValue<'v> for GrpcServer {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(grpc_server_methods)
    }
}

#[starlark_module]
fn grpc_server_methods(registry: &mut MethodsBuilder) {
    /// Attach a service. The argument is the value returned by a
    /// `<ServiceName>Service(...)` constructor from one of the
    /// `@bazel//proto/*.axl` modules. Raises if a service for the same
    /// proto path is already attached.
    fn add<'v>(this: Value<'v>, service: Value<'v>) -> anyhow::Result<NoneType> {
        let srv = this
            .downcast_ref::<GrpcServer>()
            .ok_or_else(|| anyhow!("add called on non-server value"))?;

        let svc = service.downcast_ref::<GrpcService>().ok_or_else(|| {
            anyhow!(
                "grpc.Server.add: expected a Service value, got {}",
                service.get_type()
            )
        })?;

        let mut state = srv.state.lock().unwrap();
        if state.consumed {
            return Err(anyhow!(
                "grpc.Server: cannot add services after serve() has been called"
            ));
        }
        let name = svc.inner().proto_service_name();
        if !state.seen.insert(name) {
            return Err(anyhow!(
                "grpc.Server: service '{}' already registered",
                name
            ));
        }
        state.services.push(Arc::clone(svc.inner()));
        Ok(NoneType)
    }

    /// Bind the endpoint synchronously and spawn the accept loop on the
    /// global tokio runtime. Returns a [`GrpcServerHandle`].
    ///
    /// Raises immediately on bind failure (port in use, missing parent dir
    /// for unix socket, etc.). Subsequent `add` calls on this server raise.
    fn serve<'v>(this: Value<'v>) -> anyhow::Result<GrpcServerHandle> {
        let srv = this
            .downcast_ref::<GrpcServer>()
            .ok_or_else(|| anyhow!("serve called on non-server value"))?;

        let services = {
            let mut state = srv.state.lock().unwrap();
            if state.consumed {
                return Err(anyhow!("grpc.Server.serve: already serving"));
            }
            state.consumed = true;
            std::mem::take(&mut state.services)
        };

        // The calling Starlark code always runs inside a tokio runtime
        // (the AsyncRuntime captured at task start). We use the ambient
        // handle rather than threading `Env` through every call — keeps
        // grpc independent of the wider runtime plumbing.
        let rt_handle = tokio::runtime::Handle::current();

        let (resolved, listener) = bind_endpoint(&rt_handle, &srv.endpoint)?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let (done_tx, done_rx) = oneshot::channel::<()>();

        // Install services onto a tonic Routes router.
        let mut routes = tonic::service::Routes::default();
        for h in services {
            routes = h.install(routes);
        }

        rt_handle.spawn(async move {
            let mut server = tonic::transport::Server::builder();
            let result = match listener {
                BoundListener::Tcp(l) => {
                    let stream = TcpListenerStream::new(l);
                    server
                        .add_routes(routes)
                        .serve_with_incoming_shutdown(stream, async move {
                            let _ = shutdown_rx.await;
                        })
                        .await
                }
                BoundListener::Unix(l) => {
                    let stream = UnixListenerStream::new(l);
                    server
                        .add_routes(routes)
                        .serve_with_incoming_shutdown(stream, async move {
                            let _ = shutdown_rx.await;
                        })
                        .await
                }
            };
            if let Err(e) = result {
                tracing::warn!("grpc.Server: accept loop ended with error: {}", e);
            }
            let _ = done_tx.send(());
        });

        Ok(GrpcServerHandle {
            endpoint: resolved,
            shutdown: Mutex::new(Some(shutdown_tx)),
            done: Mutex::new(Some(done_rx)),
            rt: rt_handle,
        })
    }
}

/// Handle returned by `srv.serve()`. Server runs until `drain_and_quit()`.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct GrpcServerHandle {
    endpoint: String,

    #[allocative(skip)]
    shutdown: Mutex<Option<oneshot::Sender<()>>>,

    #[allocative(skip)]
    done: Mutex<Option<oneshot::Receiver<()>>>,

    #[allocative(skip)]
    rt: tokio::runtime::Handle,
}

impl fmt::Display for GrpcServerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<grpc.ServerHandle endpoint={:?}>", self.endpoint)
    }
}

starlark_simple_value!(GrpcServerHandle);

#[starlark_value(type = "grpc.ServerHandle")]
impl<'v> StarlarkValue<'v> for GrpcServerHandle {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(grpc_server_handle_methods)
    }

    fn get_attr(&self, attr: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        if attr == "endpoint" {
            Some(heap.alloc(self.endpoint.clone()))
        } else {
            None
        }
    }
}

#[starlark_module]
fn grpc_server_handle_methods(registry: &mut MethodsBuilder) {
    /// Initiate graceful shutdown and block until in-flight RPCs have
    /// drained (or `timeout` elapses, if provided).
    ///
    /// `timeout` is an int (milliseconds) for now. When `@std//time.axl`
    /// exposes a Duration record we'll accept that too. Omitted = wait
    /// forever.
    ///
    /// Idempotent — calling twice is a no-op.
    fn drain_and_quit<'v>(
        this: Value<'v>,
        #[starlark(require = named)] timeout: Option<Value<'v>>,
    ) -> anyhow::Result<NoneType> {
        let h = this
            .downcast_ref::<GrpcServerHandle>()
            .ok_or_else(|| anyhow!("drain_and_quit called on non-handle value"))?;

        if let Some(tx) = h.shutdown.lock().unwrap().take() {
            let _ = tx.send(());
        }

        let done = h.done.lock().unwrap().take();
        let Some(done) = done else {
            return Ok(NoneType);
        };

        let timeout = duration_from_value(timeout)?;
        h.rt.block_on(async move {
            match timeout {
                None => {
                    let _ = done.await;
                }
                Some(d) => {
                    let _ = tokio::time::timeout(d, done).await;
                }
            }
        });

        Ok(NoneType)
    }
}

fn duration_from_value(v: Option<Value<'_>>) -> anyhow::Result<Option<Duration>> {
    let Some(v) = v else {
        return Ok(None);
    };
    if let Some(ms) = v.unpack_i32() {
        return Ok(Some(Duration::from_millis(ms.max(0) as u64)));
    }
    Err(anyhow!(
        "drain_and_quit: timeout must be an int (ms), got {}",
        v.get_type()
    ))
}

enum BoundListener {
    Tcp(TcpListener),
    Unix(UnixListener),
}

fn bind_endpoint(
    rt: &tokio::runtime::Handle,
    endpoint: &str,
) -> anyhow::Result<(String, BoundListener)> {
    if let Some(path) = endpoint.strip_prefix("unix://") {
        // Stale-socket workaround: leftover socket files from prior runs
        // are a common foot-gun. Better UX than "address already in use".
        let _ = std::fs::remove_file(path);
        let l = UnixListener::bind(path).with_context(|| format!("bind unix://{}", path))?;
        let resolved = format!("unix://{}", path);
        return Ok((resolved, BoundListener::Unix(l)));
    }
    let addr = endpoint.strip_prefix("tcp://").unwrap_or(endpoint);
    let parsed: SocketAddr = addr
        .parse()
        .with_context(|| format!("parse TCP address {:?}", addr))?;
    let l = rt
        .block_on(async { TcpListener::bind(parsed).await })
        .with_context(|| format!("bind tcp {}", parsed))?;
    let actual = l.local_addr().unwrap_or(parsed);
    let resolved = format!("tcp://{}", actual);
    Ok((resolved, BoundListener::Tcp(l)))
}
