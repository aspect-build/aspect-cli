//! `std.net` — local network primitives exposed to AXL.
//!
//! Currently scoped to a single Unix-domain-socket helper used to talk to
//! request/response sockets such as the CircleCI runner task-token socket.
//! TCP/UDP can be added alongside without restructuring.

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;

#[derive(Debug, Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.Net>")]
pub struct Net {}

impl Net {
    pub fn new() -> Self {
        Self {}
    }
}

#[starlark_value(type = "std.Net")]
impl<'v> values::StarlarkValue<'v> for Net {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(net_methods)
    }
}

starlark_simple_value!(Net);

#[starlark_module]
pub(crate) fn net_methods(registry: &mut MethodsBuilder) {
    /// One-shot Unix-domain-socket request/response.
    ///
    /// Connects to `path`, writes `send` if provided, shuts down the write
    /// side (so peers that wait for EOF before replying see end-of-stream
    /// promptly), reads all bytes until EOF, and returns them as a string.
    ///
    /// Returns `""` on any I/O error (socket missing, connect refused,
    /// non-UTF-8 response, transient read failure) so callers can treat
    /// "no response" and "error" uniformly without try/except scaffolding.
    /// Use a non-empty return as the signal that the peer replied.
    ///
    /// Unix only — returns `""` on other platforms.
    fn try_unix_request<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = pos)] path: values::StringValue,
        #[starlark(require = named, default = NoneOr::None)] send: NoneOr<values::StringValue>,
        heap: Heap<'v>,
    ) -> anyhow::Result<values::StringValue<'v>> {
        let payload = send.into_option().map(|s| s.as_str().to_owned());
        let response = read_unix_socket(path.as_str(), payload.as_deref()).unwrap_or_default();
        Ok(heap.alloc_str(&response))
    }
}

#[cfg(unix)]
fn read_unix_socket(path: &str, send: Option<&str>) -> std::io::Result<String> {
    use std::io::Read;
    use std::io::Write;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(path)?;
    if let Some(data) = send {
        stream.write_all(data.as_bytes())?;
    }
    stream.shutdown(Shutdown::Write)?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    String::from_utf8(buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(not(unix))]
fn read_unix_socket(_path: &str, _send: Option<&str>) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Unix domain sockets are not supported on this platform",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::thread;

    /// Build a unique socket path under `/tmp`. macOS caps `sun_path` at 104
    /// bytes; `std::env::temp_dir()` returns a long `/var/folders/...` path
    /// that overflows, so pin to `/tmp` for cross-platform safety.
    fn temp_sock_path(label: &str) -> String {
        format!("/tmp/axl-net-{label}-{}.sock", uuid::Uuid::new_v4())
    }

    /// Bind a one-shot Unix listener that captures whatever the client sends
    /// and replies with `reply`. Returns the thread handle so tests can join
    /// it and inspect what the server saw.
    fn spawn_echo_server(path: &str, reply: Vec<u8>) -> thread::JoinHandle<Vec<u8>> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).expect("bind unix listener");
        thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");
            let mut received = Vec::new();
            conn.read_to_end(&mut received).expect("read client");
            conn.write_all(&reply).expect("write reply");
            received
        })
    }

    #[test]
    fn reads_response_without_sending() {
        let path = temp_sock_path("noreq");
        let handle = spawn_echo_server(&path, b"{\"token\":\"abc\"}".to_vec());
        let resp = read_unix_socket(&path, None).expect("ok");
        assert_eq!(resp, "{\"token\":\"abc\"}");
        let received = handle.join().unwrap();
        assert!(received.is_empty(), "server saw: {received:?}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sends_payload_and_reads_response() {
        let path = temp_sock_path("req");
        let handle = spawn_echo_server(&path, b"{\"token\":\"xyz\"}".to_vec());
        let resp = read_unix_socket(&path, Some("get_token\n")).expect("ok");
        assert_eq!(resp, "{\"token\":\"xyz\"}");
        let received = handle.join().unwrap();
        assert_eq!(received, b"get_token\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_socket_errors() {
        let path = temp_sock_path("missing");
        let err = read_unix_socket(&path, None).expect_err("should fail");
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ),
            "unexpected error: {err:?}",
        );
    }

    #[test]
    fn non_utf8_response_errors() {
        let path = temp_sock_path("badutf8");
        let handle = spawn_echo_server(&path, vec![0xff, 0xfe, 0xfd]);
        let err = read_unix_socket(&path, None).expect_err("should fail");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
