//! Host side of the Incremental Build Protocol (IBP).
//!
//! The host (aspect-cli) listens on a unix socket whose path it advertises to
//! the spawned target via the `ABAZEL_WATCH_SOCKET_FILE` environment variable.
//! A protocol-aware target (e.g. a devserver) connects, negotiates a version
//! and capabilities, then receives CYCLE messages describing filesystem
//! changes and answers each with CYCLE_STARTED plus one terminal message.
//! Transport is line-delimited JSON, bidirectional.
//!
//! Protocol reference:
//! https://github.com/aspect-extensions/watchdog/blob/main/INCREMENTAL_BUILD_PROTOCOL.md
//!
//! The API is poll-based to suit a single-threaded caller: sends are
//! non-blocking, and connection/cycle state is sampled via [`IbpServer::state`]
//! and [`IbpServer::cycle_state`]. Socket I/O runs on background threads.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

pub const SOCKET_ENV_VAR: &str = "ABAZEL_WATCH_SOCKET_FILE";

/// Versions offered to the client, in preference order.
const VERSIONS: [i64; 3] = [2, 1, 0];

#[derive(Debug, thiserror::Error)]
pub enum IbpError {
    #[error("ibp: {0}")]
    Io(#[from] std::io::Error),
    #[error("ibp: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Sources,
    Runfiles,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Sources => "sources",
            Scope::Runfiles => "runfiles",
        }
    }
}

/// Client capabilities. Absent CAPS (or v0) means runfiles-only, no otel.
#[derive(Debug, Clone)]
pub struct Caps {
    pub scopes: Vec<Scope>,
    pub otel: bool,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            scopes: vec![Scope::Runfiles],
            otel: false,
        }
    }
}

/// One entry in a CYCLE's `sources` map. `Deleted` serializes to `null`;
/// `Generated` (a changed build output, neither source nor symlink) to `{}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    Source,
    Symlink,
    Generated,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleState {
    /// Sent; no response yet.
    Pending,
    Started,
    Completed,
    Aborted,
    Failed(String),
}

/// Connection state as sampled by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerState {
    /// Listening; no client has completed the handshake.
    AwaitingClient,
    /// Handshake complete at the given version.
    Connected(i64),
    /// The client sent EXIT (description inside) or the connection dropped.
    Disconnected(String),
}

#[derive(Default)]
struct Shared {
    connected_version: Option<i64>,
    caps: Option<Caps>,
    cycles: HashMap<u64, CycleState>,
    disconnected: Option<String>,
    generation: u64,
}

pub struct IbpServer {
    path: PathBuf,
    shared: Arc<Mutex<Shared>>,
    writer: Arc<Mutex<Option<UnixStream>>>,
    next_cycle_id: AtomicU64,
}

impl IbpServer {
    /// Bind the socket and start accepting (one client) in the background.
    pub fn start() -> Result<Self, IbpError> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "aspect-watch-{}-{}-socket",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let writer = Arc::new(Mutex::new(None));
        {
            let shared = Arc::clone(&shared);
            let writer = Arc::clone(&writer);
            std::thread::spawn(move || accept_loop(listener, shared, writer));
        }
        Ok(Self {
            path,
            shared,
            writer,
            next_cycle_id: AtomicU64::new(1),
        })
    }

    /// The environment the spawned target needs to find this server.
    pub fn env(&self) -> Vec<(String, String)> {
        vec![(
            SOCKET_ENV_VAR.to_string(),
            self.path.to_string_lossy().into_owned(),
        )]
    }

    pub fn state(&self) -> ServerState {
        let shared = self.shared.lock().unwrap();
        if let Some(reason) = &shared.disconnected {
            return ServerState::Disconnected(reason.clone());
        }
        match shared.connected_version {
            Some(v) => ServerState::Connected(v),
            None => ServerState::AwaitingClient,
        }
    }

    /// Negotiated client capabilities; `None` until connected.
    pub fn caps(&self) -> Option<Caps> {
        let shared = self.shared.lock().unwrap();
        if shared.connected_version.is_none() {
            return None;
        }
        Some(shared.caps.clone().unwrap_or_default())
    }

    /// Whether the negotiated version supports CYCLE_RESET (v2+).
    pub fn has_reset(&self) -> bool {
        matches!(self.state(), ServerState::Connected(v) if v >= 2)
    }

    /// Number of completed client handshakes. A replacement client can connect
    /// between two `state()` samples (disconnect and reconnect both inside one
    /// poll interval), so pollers compare this — not the connected state — to
    /// detect a new connection.
    pub fn generation(&self) -> u64 {
        self.shared.lock().unwrap().generation
    }

    /// Send a CYCLE. Poll the returned id via [`cycle_state`]; the send itself
    /// is bounded by the connection's write timeout.
    ///
    /// `fresh` marks the cycle as computed from scratch (delta state was
    /// lost). Serialized as `is_fresh` for pre-v2 clients, which have no
    /// CYCLE_RESET message; v2+ callers use [`cycle_reset`] instead.
    pub fn cycle(
        &self,
        sources: &[(String, SourceState)],
        scope: Scope,
        fresh: bool,
    ) -> Result<u64, IbpError> {
        let version = self.connected_version()?;
        let sources: serde_json::Map<String, Value> = sources
            .iter()
            .map(|(path, state)| {
                let info = match state {
                    SourceState::Deleted => Value::Null,
                    SourceState::Source => json!({"is_source": true}),
                    SourceState::Symlink => json!({"is_symlink": true}),
                    SourceState::Generated => json!({}),
                };
                (path.clone(), info)
            })
            .collect();
        let id = self.next_cycle_id.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({
            "kind": "CYCLE",
            "cycle_id": id,
            "sources": sources,
        });
        if version >= 1 {
            message["scope"] = json!(scope.as_str());
        }
        if fresh && version < 2 {
            message["is_fresh"] = json!(true);
        }
        self.track_and_send(id, &message)
    }

    /// Send a CYCLE_RESET (v2+): the host lost delta state; the client must
    /// recompute from scratch.
    pub fn cycle_reset(&self) -> Result<u64, IbpError> {
        if !self.has_reset() {
            return Err(IbpError::Protocol(
                "CYCLE_RESET requires protocol version 2".to_string(),
            ));
        }
        let id = self.next_cycle_id.fetch_add(1, Ordering::Relaxed);
        let message = json!({"kind": "CYCLE_RESET", "cycle_id": id});
        self.track_and_send(id, &message)
    }

    pub fn cycle_state(&self, id: u64) -> Option<CycleState> {
        self.shared.lock().unwrap().cycles.get(&id).cloned()
    }

    /// Best-effort EXIT notification to the client.
    pub fn exit(&self, description: &str) {
        let message = json!({"kind": "EXIT", "description": description});
        let _ = self.send(&message);
    }

    fn connected_version(&self) -> Result<i64, IbpError> {
        match self.state() {
            ServerState::Connected(v) => Ok(v),
            other => Err(IbpError::Protocol(format!(
                "no connected client: {other:?}"
            ))),
        }
    }

    fn track_and_send(&self, id: u64, message: &Value) -> Result<u64, IbpError> {
        self.shared
            .lock()
            .unwrap()
            .cycles
            .insert(id, CycleState::Pending);
        self.send(message)?;
        Ok(id)
    }

    fn send(&self, message: &Value) -> Result<(), IbpError> {
        let mut guard = self.writer.lock().unwrap();
        let stream = guard
            .as_mut()
            .ok_or_else(|| IbpError::Protocol("no connected client".to_string()))?;
        if let Err(e) = writeln!(stream, "{message}") {
            // A wedged or gone client: record the disconnect (a re-accepted
            // client resets it) and abort outstanding cycles so pollers of
            // cycle_state don't wait forever.
            let mut shared = self.shared.lock().unwrap();
            disconnect(&mut shared, &e.to_string());
            return Err(e.into());
        }
        Ok(())
    }
}

/// Mark the connection gone: state becomes Disconnected until the next client
/// completes a handshake, and every non-terminal cycle is aborted so callers
/// polling for a terminal state observe one.
fn disconnect(shared: &mut Shared, reason: &str) {
    shared.disconnected.get_or_insert(reason.to_string());
    shared.connected_version = None;
    shared.caps = None;
    for state in shared.cycles.values_mut() {
        if matches!(state, CycleState::Pending | CycleState::Started) {
            *state = CycleState::Aborted;
        }
    }
}

impl Drop for IbpServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn accept_loop(
    listener: UnixListener,
    shared: Arc<Mutex<Shared>>,
    writer: Arc<Mutex<Option<UnixStream>>>,
) {
    // Serve clients one at a time for the server's whole life: a restarted
    // (or reconnecting) target must be able to complete a fresh handshake on
    // the same advertised socket path.
    loop {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let reason = match serve_client(stream, &shared, &writer) {
            Ok(()) => "client exited".to_string(),
            Err(e) => {
                tracing::debug!("ibp client ended: {e}");
                e.to_string()
            }
        };
        *writer.lock().unwrap() = None;
        disconnect(&mut shared.lock().unwrap(), &reason);
    }
}

fn serve_client(
    stream: UnixStream,
    shared: &Mutex<Shared>,
    writer: &Mutex<Option<UnixStream>>,
) -> Result<(), IbpError> {
    let mut write_half = stream.try_clone()?;
    // A client that stops reading must not freeze the caller in send() once
    // the socket buffer fills; a timed-out write is treated as a disconnect.
    write_half.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream);

    writeln!(
        write_half,
        "{}",
        json!({"kind": "NEGOTIATE", "versions": VERSIONS})
    )?;
    let response = read_message(&mut reader)?;
    expect_kind(&response, "NEGOTIATE_RESPONSE")?;
    let version = response
        .get("version")
        .and_then(Value::as_i64)
        .ok_or_else(|| IbpError::Protocol("NEGOTIATE_RESPONSE without version".to_string()))?;
    if !VERSIONS.contains(&version) {
        return Err(IbpError::Protocol(format!(
            "client selected unsupported version {version}"
        )));
    }

    let mut caps = None;
    if version >= 1 {
        let message = read_message(&mut reader)?;
        expect_kind(&message, "CAPS")?;
        let parsed = parse_caps(message.get("caps"));
        let confirm = json!({"kind": "CAPS_RESPONSE", "caps": {
            "scope": parsed.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            // otel propagation is not implemented; always negotiated off.
            "otel": false,
        }});
        writeln!(write_half, "{confirm}")?;
        caps = Some(Caps {
            otel: false,
            ..parsed
        });
    }

    *writer.lock().unwrap() = Some(write_half);
    {
        let mut shared = shared.lock().unwrap();
        shared.caps = caps;
        shared.connected_version = Some(version);
        shared.disconnected = None;
        shared.generation += 1;
    }

    loop {
        let message = read_message(&mut reader)?;
        let kind = message.get("kind").and_then(Value::as_str).unwrap_or("");
        let cycle_id = message.get("cycle_id").and_then(Value::as_u64);
        let mut shared = shared.lock().unwrap();
        match (kind, cycle_id) {
            ("CYCLE_STARTED", Some(id)) => {
                shared.cycles.insert(id, CycleState::Started);
            }
            ("CYCLE_COMPLETED", Some(id)) => {
                shared.cycles.insert(id, CycleState::Completed);
            }
            ("CYCLE_ABORTED", Some(id)) | ("CYCLE_ABORT", Some(id)) => {
                shared.cycles.insert(id, CycleState::Aborted);
            }
            ("CYCLE_FAILED", Some(id)) => {
                let description = message
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                shared.cycles.insert(id, CycleState::Failed(description));
            }
            ("EXIT", _) => {
                let description = message
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("client exited");
                shared.disconnected = Some(description.to_string());
                return Ok(());
            }
            _ => tracing::debug!("ibp: ignoring message kind {kind:?}"),
        }
    }
}

fn read_message(reader: &mut BufReader<UnixStream>) -> Result<Value, IbpError> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(IbpError::Protocol("connection closed".to_string()));
        }
        if line.trim().is_empty() {
            continue;
        }
        return serde_json::from_str(&line)
            .map_err(|e| IbpError::Protocol(format!("bad message: {e}")));
    }
}

fn expect_kind(message: &Value, kind: &str) -> Result<(), IbpError> {
    let got = message.get("kind").and_then(Value::as_str).unwrap_or("");
    if got != kind {
        return Err(IbpError::Protocol(format!("expected {kind}, got {got:?}")));
    }
    Ok(())
}

fn parse_caps(caps: Option<&Value>) -> Caps {
    let mut parsed = Caps::default();
    let Some(caps) = caps.and_then(Value::as_object) else {
        return parsed;
    };
    if let Some(scopes) = caps.get("scope").and_then(Value::as_array) {
        // An explicit scope list is honored as-is (unknown values dropped) —
        // the host may only clamp a client's request, never expand it. The
        // runfiles default applies solely when the key is absent.
        parsed.scopes = scopes
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|s| match s {
                "sources" => Some(Scope::Sources),
                "runfiles" => Some(Scope::Runfiles),
                _ => None,
            })
            .collect();
    }
    parsed.otel = caps.get("otel").and_then(Value::as_bool).unwrap_or(false);
    parsed
}

#[cfg(test)]
mod tests;
