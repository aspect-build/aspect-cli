use std::cell::RefCell;

use allocative::Allocative;
use derive_more::Display;

use starlark::StarlarkResultExt;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::Trace;
use starlark::values::ValueLike;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::starlark_value;

pub(crate) fn start() -> anyhow::Result<IbpServer> {
    Ok(IbpServer {
        inner: RefCell::new(ibp::IbpServer::start()?),
    })
}

/// Host side of the Incremental Build Protocol: a unix-socket server the
/// spawned target connects back to (via the env from `env()`) to receive
/// change cycles instead of being restarted.
#[derive(Display, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<bazel.IbpServer>")]
pub struct IbpServer {
    #[allocative(skip)]
    inner: RefCell<ibp::IbpServer>,
}

impl std::fmt::Debug for IbpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<bazel.IbpServer>")
    }
}

impl<'v> values::AllocValue<'v> for IbpServer {
    fn alloc_value(self, heap: Heap<'v>) -> values::Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "bazel.IbpServer")]
impl<'v> values::StarlarkValue<'v> for IbpServer {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(ibp_server_methods)
    }
}

fn parse_scope(scope: &str) -> anyhow::Result<ibp::Scope> {
    match scope {
        "sources" => Ok(ibp::Scope::Sources),
        "runfiles" => Ok(ibp::Scope::Runfiles),
        other => Err(anyhow::anyhow!(
            "invalid scope {other:?}: expected \"sources\" or \"runfiles\""
        )),
    }
}

#[starlark_module]
pub(crate) fn ibp_server_methods(registry: &mut MethodsBuilder) {
    /// The environment the spawned target needs to find this server
    /// (`ABAZEL_WATCH_SOCKET_FILE`). Merge into the spawn env.
    fn env<'v>(
        this: values::Value<'v>,
        heap: values::Heap<'v>,
    ) -> anyhow::Result<values::Value<'v>> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        let entries: Vec<(values::Value<'v>, values::Value<'v>)> = server
            .inner
            .borrow()
            .env()
            .into_iter()
            .map(|(k, v)| (heap.alloc_str(&k).to_value(), heap.alloc_str(&v).to_value()))
            .collect();
        Ok(heap.alloc(values::dict::AllocDict(entries)))
    }

    /// Connection state: `"awaiting"` (no client yet), `"connected"`, or
    /// `"disconnected"` (client sent EXIT or the connection dropped).
    #[starlark(attribute)]
    fn state<'v>(this: values::Value<'v>) -> anyhow::Result<String> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        Ok(match server.inner.borrow().state() {
            ibp::ServerState::AwaitingClient => "awaiting",
            ibp::ServerState::Connected(_) => "connected",
            ibp::ServerState::Disconnected(_) => "disconnected",
        }
        .to_string())
    }

    /// Scopes the connected client asked to watch (`"sources"` /
    /// `"runfiles"`). Empty until a client is connected.
    #[starlark(attribute)]
    fn scopes<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<String>> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        Ok(server
            .inner
            .borrow()
            .caps()
            .map(|caps| caps.scopes.iter().map(|s| s.as_str().to_string()).collect())
            .unwrap_or_default())
    }

    /// Whether the negotiated protocol version supports `cycle_reset()` (v2+).
    #[starlark(attribute)]
    fn has_reset<'v>(this: values::Value<'v>) -> anyhow::Result<bool> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        Ok(server.inner.borrow().has_reset())
    }

    /// Number of completed client handshakes. A replacement client can
    /// disconnect and reconnect between two `state` samples, so pollers
    /// compare this — not the connected state — to detect a new connection.
    #[starlark(attribute)]
    fn generation<'v>(this: values::Value<'v>) -> anyhow::Result<u64> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        Ok(server.inner.borrow().generation())
    }

    /// Send a change cycle. `sources` maps each changed path to `"source"`,
    /// `"symlink"`, or `None` (deleted). Non-blocking: returns the cycle id
    /// to poll via `cycle_state(id)`, or `None` when no client is connected
    /// (or the send failed) — the caller falls back to another notify path.
    /// `fresh = True` marks the cycle as computed from scratch (delta state
    /// lost) — serialized as `is_fresh` for pre-v2 clients, which have no
    /// `cycle_reset()`.
    fn cycle<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] sources: UnpackDictEntries<String, NoneOr<String>>,
        #[starlark(require = named, default = "sources")] scope: &str,
        #[starlark(require = named, default = false)] fresh: bool,
    ) -> anyhow::Result<NoneOr<u64>> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        let sources: Vec<(String, ibp::SourceState)> = sources
            .entries
            .into_iter()
            .map(|(path, state)| {
                let state = match state {
                    NoneOr::None => Ok(ibp::SourceState::Deleted),
                    NoneOr::Other(s) => match s.as_str() {
                        "source" => Ok(ibp::SourceState::Source),
                        "symlink" => Ok(ibp::SourceState::Symlink),
                        other => Err(anyhow::anyhow!(
                            "invalid source state {other:?}: expected \"source\", \"symlink\", or None"
                        )),
                    },
                };
                state.map(|s| (path, s))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(
            match server
                .inner
                .borrow()
                .cycle(&sources, parse_scope(scope)?, fresh)
            {
                Ok(id) => NoneOr::Other(id),
                Err(e) => {
                    tracing::debug!("ibp cycle not sent: {e}");
                    NoneOr::None
                }
            },
        )
    }

    /// Send a CYCLE_RESET (v2+ only — check `has_reset`): the host lost track
    /// of deltas and the client must recompute from scratch. Returns the cycle
    /// id, or `None` when it could not be sent.
    fn cycle_reset<'v>(this: values::Value<'v>) -> anyhow::Result<NoneOr<u64>> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        Ok(match server.inner.borrow().cycle_reset() {
            Ok(id) => NoneOr::Other(id),
            Err(e) => {
                tracing::debug!("ibp cycle_reset not sent: {e}");
                NoneOr::None
            }
        })
    }

    /// The client's progress on a cycle: `None` (unknown id), `"pending"`,
    /// `"started"`, `"completed"`, `"aborted"`, or `"failed"`.
    fn cycle_state<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] id: u64,
    ) -> anyhow::Result<NoneOr<String>> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        Ok(match server.inner.borrow().cycle_state(id) {
            None => NoneOr::None,
            Some(state) => NoneOr::Other(
                match state {
                    ibp::CycleState::Pending => "pending",
                    ibp::CycleState::Started => "started",
                    ibp::CycleState::Completed => "completed",
                    ibp::CycleState::Aborted => "aborted",
                    ibp::CycleState::Failed(description) => {
                        tracing::debug!("ibp cycle {id} failed: {description}");
                        "failed"
                    }
                }
                .to_string(),
            ),
        })
    }

    /// Best-effort EXIT notification to the client (e.g. on shutdown).
    fn exit<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos, default = "")] description: &str,
    ) -> anyhow::Result<NoneType> {
        let server = this.downcast_ref_err::<IbpServer>().into_anyhow_result()?;
        server.inner.borrow().exit(description);
        Ok(NoneType)
    }
}
