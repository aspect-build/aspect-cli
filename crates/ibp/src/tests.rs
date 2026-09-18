use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{Caps, CycleState, IbpServer, Scope, ServerState, SourceState};

struct FakeClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl FakeClient {
    fn connect(server: &IbpServer) -> Self {
        let path = server.env().remove(0).1;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match UnixStream::connect(&path) {
                Ok(stream) => {
                    let writer = stream.try_clone().unwrap();
                    return Self {
                        reader: BufReader::new(stream),
                        writer,
                    };
                }
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("connect: {e}"),
            }
        }
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "server closed the connection");
        serde_json::from_str(&line).unwrap()
    }

    fn send(&mut self, message: Value) {
        writeln!(self.writer, "{message}").unwrap();
    }

    fn handshake(server: &IbpServer, version: i64, caps: Option<Value>) -> Self {
        let mut client = Self::connect(server);
        let negotiate = client.recv();
        assert_eq!(negotiate["kind"], "NEGOTIATE");
        assert_eq!(negotiate["versions"], json!([2, 1, 0]));
        client.send(json!({"kind": "NEGOTIATE_RESPONSE", "version": version}));
        if version >= 1 {
            client.send(json!({"kind": "CAPS", "caps": caps.unwrap_or(json!({}))}));
            let response = client.recv();
            assert_eq!(response["kind"], "CAPS_RESPONSE");
        }
        wait_connected(server);
        client
    }
}

fn wait_connected(server: &IbpServer) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(server.state(), ServerState::Connected(_)) {
        assert!(Instant::now() < deadline, "handshake never completed");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_cycle(server: &IbpServer, id: u64, want: CycleState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = server.cycle_state(id);
        if state.as_ref() == Some(&want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "cycle {id} stuck at {state:?}, want {want:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn v2_full_cycle_roundtrip() {
    let server = IbpServer::start().unwrap();
    assert_eq!(server.state(), ServerState::AwaitingClient);
    let mut client = FakeClient::handshake(
        &server,
        2,
        Some(json!({"scope": ["sources"], "otel": true})),
    );

    assert_eq!(server.state(), ServerState::Connected(2));
    let caps: Caps = server.caps().unwrap();
    assert_eq!(caps.scopes, vec![Scope::Sources]);
    assert!(!caps.otel, "otel must be negotiated off");
    assert!(server.has_reset());

    let id = server
        .cycle(
            &[
                ("src/a.ts".to_string(), SourceState::Source),
                ("src/gone.ts".to_string(), SourceState::Deleted),
            ],
            Scope::Sources,
            false,
        )
        .unwrap();
    let cycle = client.recv();
    assert_eq!(cycle["kind"], "CYCLE");
    assert_eq!(cycle["cycle_id"].as_u64(), Some(id));
    assert_eq!(cycle["scope"], "sources");
    assert_eq!(cycle["sources"]["src/a.ts"], json!({"is_source": true}));
    assert_eq!(cycle["sources"]["src/gone.ts"], Value::Null);

    assert_eq!(server.cycle_state(id), Some(CycleState::Pending));
    client.send(json!({"kind": "CYCLE_STARTED", "cycle_id": id}));
    wait_cycle(&server, id, CycleState::Started);
    client.send(json!({"kind": "CYCLE_COMPLETED", "cycle_id": id}));
    wait_cycle(&server, id, CycleState::Completed);

    let reset_id = server.cycle_reset().unwrap();
    let reset = client.recv();
    assert_eq!(reset["kind"], "CYCLE_RESET");
    assert_eq!(reset["cycle_id"].as_u64(), Some(reset_id));
    client.send(json!({"kind": "CYCLE_FAILED", "cycle_id": reset_id, "description": "boom"}));
    wait_cycle(&server, reset_id, CycleState::Failed("boom".to_string()));
}

#[test]
fn v0_omits_scope_and_refuses_reset() {
    let server = IbpServer::start().unwrap();
    let mut client = FakeClient::handshake(&server, 0, None);

    assert_eq!(server.state(), ServerState::Connected(0));
    assert_eq!(server.caps().unwrap().scopes, vec![Scope::Runfiles]);
    assert!(!server.has_reset());
    assert!(server.cycle_reset().is_err());

    let id = server
        .cycle(
            &[("a".to_string(), SourceState::Symlink)],
            Scope::Runfiles,
            false,
        )
        .unwrap();
    let cycle = client.recv();
    assert_eq!(cycle["kind"], "CYCLE");
    assert!(cycle.get("scope").is_none(), "v0 must omit scope: {cycle}");
    assert_eq!(cycle["sources"]["a"], json!({"is_symlink": true}));
}

#[test]
fn client_exit_disconnects() {
    let server = IbpServer::start().unwrap();
    let mut client = FakeClient::handshake(&server, 2, None);
    client.send(json!({"kind": "EXIT", "description": "done here"}));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let ServerState::Disconnected(reason) = server.state() {
            assert_eq!(reason, "done here");
            break;
        }
        assert!(Instant::now() < deadline, "EXIT never observed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(server.cycle(&[], Scope::Sources, false).is_err());
}

#[test]
fn cycle_before_connect_errors() {
    let server = IbpServer::start().unwrap();
    assert!(server.cycle(&[], Scope::Sources, false).is_err());
}

#[test]
fn reconnect_after_exit_serves_new_client() {
    let server = IbpServer::start().unwrap();
    assert_eq!(server.generation(), 0);
    let mut first = FakeClient::handshake(&server, 2, None);
    assert_eq!(server.generation(), 1);
    let stale = server
        .cycle(
            &[("a".to_string(), SourceState::Source)],
            Scope::Sources,
            false,
        )
        .unwrap();
    let _ = first.recv();
    first.send(json!({"kind": "EXIT", "description": "restarting"}));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(server.state(), ServerState::Disconnected(_)) {
        assert!(Instant::now() < deadline, "EXIT never observed");
        std::thread::sleep(Duration::from_millis(10));
    }
    // The outstanding cycle terminated as aborted rather than hanging.
    assert_eq!(server.cycle_state(stale), Some(CycleState::Aborted));

    // A restarted target handshakes on the same socket and gets cycles.
    let mut second = FakeClient::handshake(&server, 0, None);
    assert_eq!(server.state(), ServerState::Connected(0));
    assert_eq!(server.generation(), 2);
    let id = server
        .cycle(
            &[("b".to_string(), SourceState::Source)],
            Scope::Runfiles,
            true,
        )
        .unwrap();
    let cycle = second.recv();
    assert_eq!(cycle["cycle_id"].as_u64(), Some(id));
    assert_eq!(cycle["is_fresh"], json!(true), "v0 fresh cycle: {cycle}");
}

#[test]
fn explicit_empty_scope_is_not_expanded() {
    let server = IbpServer::start().unwrap();
    let _client = FakeClient::handshake(&server, 2, Some(json!({"scope": []})));
    assert_eq!(server.caps().unwrap().scopes, Vec::<Scope>::new());
}
