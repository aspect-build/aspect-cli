//! `std.net` from AXL: each test runs a task body against real sockets on
//! the loopback interface. A listener's backlog holds a connection until it
//! is accepted, and the kernel buffers what is written, so one task can play
//! both ends of a small exchange.

use std::io::{Read, Write};
use std::sync::Arc;
use std::thread;

/// Starlark has no assertion builtin; the task bodies share this one.
const PRELUDE: &str = r#"
def assert_eq(got, want):
    if got != want:
        fail("want", repr(want), "got", repr(got))
"#;

/// Run `body` as a task body, with `ctx` and `net = ctx.std.net` in scope.
fn run(body: &str) -> anyhow::Result<Option<u8>> {
    let body: String = body.lines().map(|l| format!("    {l}\n")).collect();
    crate::test::eval(&format!(
        "{PRELUDE}\ndef _impl(ctx):\n    net = ctx.std.net\n{body}    return 0\n\nt = task(implementation = _impl)\n"
    ))
    .run_task(0)
}

fn ok(body: &str) {
    if let Err(e) = run(body) {
        panic!("expected the task to pass, got: {e:?}");
    }
}

fn fails(body: &str) -> String {
    format!("{:?}", run(body).expect_err("expected the task to fail"))
}

/// A Unix socket path short enough for macOS's 104-byte limit.
#[cfg(unix)]
fn sock_path(label: &str) -> String {
    format!(
        "/tmp/axl-net-{label}-{}.sock",
        uuid::Uuid::new_v4().simple()
    )
}

#[test]
fn tcp_round_trip() {
    ok(r#"
l = net.tcp.listen("127.0.0.1:0")
addr = l.local_addr()
c = net.tcp.connect(addr, timeout_ms = 2000)
s, peer = l.accept(timeout_ms = 2000)
assert_eq(peer, c.local_addr())
assert_eq(c.peer_addr(), addr)
c.set_nodelay(True)
c.write_all("ping")
assert_eq(s.read_exact(4), b"ping")
s.write_all(b"pong and more")
s.shutdown("write")
assert_eq(c.read(4), b"pong")
assert_eq(c.read_to_string(), " and more")
assert_eq(c.read(16), b"")
c.close()
c.close()
s.close()
l.close()
"#);
}

#[test]
fn a_huge_read_size_does_not_allocate_it() {
    ok(r#"
l = net.tcp.listen("127.0.0.1:0")
c = net.tcp.connect(l.local_addr(), timeout_ms = 2000)
s, _ = l.accept(timeout_ms = 2000)
c.write_all("abc")
assert_eq(s.read(4294967295), b"abc")
c.write_all("def")
c.shutdown("write")
err, data = s.try_read_exact(4294967295)
assert_eq(data, None)
assert_eq(err.kind, "unexpected_eof")
"#);
}

#[test]
fn a_refused_connection_is_an_io_error() {
    ok(r#"
l = net.tcp.listen("127.0.0.1:0")
addr = l.local_addr()
l.close()
err, s = net.tcp.try_connect(addr, timeout_ms = 2000)
assert_eq(s, None)
assert_eq(isinstance(err, std.io.Error), True)
assert_eq(isinstance(err, error), True)
assert_eq(err.kind, "connection_refused")
"#);
}

#[test]
fn an_uncaught_io_error_raises_with_its_type() {
    let msg = fails(
        r#"
l = net.tcp.listen("127.0.0.1:0")
addr = l.local_addr()
l.close()
net.tcp.connect(addr)
"#,
    );
    assert!(msg.contains("std.io.Error"), "{msg}");
}

#[test]
fn catch_takes_std_io_error_and_leaves_bugs_alone() {
    ok(r#"
def _connect_closed():
    l = net.tcp.listen("127.0.0.1:0")
    addr = l.local_addr()
    l.close()
    return net.tcp.connect(addr)

err, _ = catch(_connect_closed, types = [std.io.Error])
assert_eq(err.kind, "connection_refused")
assert_eq(repr(err).startswith("std.io.Error(message = "), True)
assert_eq("kind = \"connection_refused\"" in repr(err), True)
"#);
    let msg = fails(
        r#"
def _bug():
    fail("a bug")

catch(_bug, types = [std.io.Error])
"#,
    );
    assert!(msg.contains("a bug"), "{msg}");
}

#[test]
fn a_silent_peer_times_out() {
    ok(r#"
l = net.tcp.listen("127.0.0.1:0")
c = net.tcp.connect(l.local_addr())
c.set_read_timeout(50)
err, data = c.try_read(1)
assert_eq(err.kind, "timed_out")
err, _ = l.try_accept(timeout_ms = 1)
assert_eq(err, None)
err, _ = l.try_accept(timeout_ms = 50)
assert_eq(err.kind, "timed_out")
"#);
}

#[test]
fn a_bad_call_raises_even_from_try() {
    let msg = fails(
        r#"
l = net.tcp.listen("127.0.0.1:0")
c = net.tcp.connect(l.local_addr())
c.try_shutdown("sideways")
"#,
    );
    assert!(msg.contains("\"read\", \"write\" or \"both\""), "{msg}");

    let msg = fails(
        r#"
l = net.tcp.listen("127.0.0.1:0")
c = net.tcp.connect(l.local_addr())
c.close()
c.try_read(1)
"#,
    );
    assert!(msg.contains("the stream is closed"), "{msg}");

    let msg = fails(
        r#"
net.tcp.try_connect("127.0.0.1:1", timeout_ms = 0)
"#,
    );
    assert!(msg.contains("timeout_ms must be positive"), "{msg}");
}

#[cfg(unix)]
#[test]
fn unix_round_trip() {
    let path = sock_path("rt");
    ok(&format!(
        r#"
l = net.unix.listen("{path}")
assert_eq(l.local_addr(), "{path}")
c = net.unix.connect("{path}", timeout_ms = 2000)
s, peer = l.accept(timeout_ms = 2000)
assert_eq(peer, None)
assert_eq(c.peer_addr(), "{path}")
c.write_all("hello")
c.shutdown("write")
assert_eq(s.read_to_end(), b"hello")
err, _ = net.unix.try_listen("{path}")
assert_eq(err.kind, "addr_in_use")
l.close()
"#
    ));
    let _ = std::fs::remove_file(&path);
}

#[cfg(unix)]
#[test]
fn a_missing_unix_socket_is_not_found() {
    let path = sock_path("missing");
    ok(&format!(
        r#"
err, s = net.unix.try_connect("{path}")
assert_eq(err.kind, "not_found")
"#
    ));
}

/// A self-signed CA and a `localhost` server certificate it signed.
struct TestPki {
    ca_pem: String,
    server: Arc<rustls::ServerConfig>,
}

fn test_pki() -> TestPki {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};

    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_key).unwrap();

    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .signed_by(&key, &ca, &ca_key)
        .unwrap();

    let server = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.der().clone()],
        rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
    )
    .unwrap();
    TestPki {
        ca_pem: ca.pem(),
        server: Arc::new(server),
    }
}

/// A TLS server for one connection that echoes what it reads, uppercased,
/// once the client finishes writing. Returns its port, and a handle whose
/// result says whether the client ended the stream with `close_notify`.
fn tls_echo_server(config: Arc<rustls::ServerConfig>) -> (u16, thread::JoinHandle<Option<bool>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (sock, _) = listener.accept().ok()?;
        let conn = rustls::ServerConnection::new(config).ok()?;
        let mut tls = rustls::StreamOwned::new(conn, sock);
        let mut buf = Vec::new();
        let clean = match tls.read_to_end(&mut buf) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => false,
            Err(_) => return None,
        };
        let _ = tls.write_all(&buf.to_ascii_uppercase());
        tls.conn.send_close_notify();
        let _ = tls.flush();
        Some(clean)
    });
    (port, handle)
}

#[test]
fn tls_round_trip_with_a_private_ca() {
    let pki = test_pki();
    let (port, server) = tls_echo_server(pki.server);
    ok(&format!(
        r#"
s = net.tls.connect("localhost:{port}", timeout_ms = 5000, ca_pem = {ca:?})
s.write_all("hello tls")
s.shutdown("write")
assert_eq(s.read_to_string(), "HELLO TLS")
s.close()
"#,
        ca = pki.ca_pem
    ));
    assert_eq!(server.join().unwrap(), Some(true), "close_notify was sent");
}

#[test]
fn tls_rejects_a_certificate_outside_ca_pem() {
    let pki = test_pki();
    let other = test_pki();
    let (port, _server) = tls_echo_server(pki.server);
    ok(&format!(
        r#"
err, s = net.tls.try_connect("localhost:{port}", timeout_ms = 5000, ca_pem = {ca:?})
assert_eq(s, None)
assert_eq(err.kind, "invalid_data")
assert_eq("certificate" in err.message, True)
"#,
        ca = other.ca_pem
    ));
}

#[test]
fn tls_rejects_a_ca_pem_without_certificates() {
    ok(r#"
err, s = net.tls.try_connect("localhost:1", ca_pem = "not a certificate")
assert_eq(err.kind, "invalid_input")
err, s = net.tls.try_connect("localhost:1", ca_pem = b"")
assert_eq(err.kind, "invalid_input")
"#);
}

/// Static typechecking of `snippet` against the AXL globals.
fn type_errors(snippet: &str) -> Vec<String> {
    use starlark::syntax::AstModule;
    use starlark::typing::AstModuleTypecheck;

    let ast = AstModule::parse(
        "<snippet>",
        snippet.to_owned(),
        &crate::eval::api::dialect(),
    )
    .expect("snippet parses");
    let globals = crate::eval::api::get_globals().build();
    let (errors, ..) = ast.typecheck(&globals, &Default::default());
    errors.iter().map(|e| e.to_string()).collect()
}

#[test]
fn try_methods_are_typed_pairs() {
    let errors = type_errors(
        r#"
def _read(s: std.net.TcpStream) -> int:
    err, data = s.try_read(4)
    return data
"#,
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("None | bytes") || e.contains("bytes | None")),
        "{errors:?}"
    );

    let errors = type_errors(
        r#"
def _kind(s: std.net.TcpStream) -> str:
    err, data = s.try_read(4)
    if err:
        return err.kind
    return ""

def _io(e: std.io.Error) -> error:
    return e
"#,
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
#[ignore = "needs the network"]
fn tls_trusts_the_os_store() {
    ok(r#"
s = net.tls.connect("example.com:443", timeout_ms = 5000)
s.write_all("GET / HTTP/1.0\r\nHost: example.com\r\n\r\n")
s.set_read_timeout(5000)
reply = s.read_to_string()
assert_eq(reply.startswith("HTTP/1."), True)
"#);
}
