//! The `tcp`, `unix` and `tls` namespaces and the streams they connect.
//!
//! The three stream types share one implementation, [`StreamInner`], and one
//! set of methods; each adds only the address methods its transport has.
//! Every fallible method is written once as an `op_*` function, which both
//! the method and its `try_` twin call. An I/O failure leaves it as an
//! [`IoError`]; anything else, like a closed stream, is a plain error.

use std::cell::RefCell;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::Shutdown;

use allocative::Allocative;
use either::Either;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::bytes::StarlarkBytes;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{
    AllocValue, Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Value, ValueLike,
    starlark_value,
};

use super::listener::{Listener, ListenerInner, TcpListener, UnixListener};
use super::{connect_tcp, timed, timeout, tls};
use crate::engine::error::{Attempt, IoError};

/// An open connection, over whichever transport made it.
#[derive(Debug)]
pub(super) enum Conn {
    Tcp(std::net::TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            Self::Unix(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Self::Unix(s) => s.write(buf),
            Self::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Self::Unix(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}

impl Conn {
    /// The TCP socket under a TCP or TLS stream.
    fn tcp_socket(&self) -> Option<&std::net::TcpStream> {
        match self {
            Self::Tcp(s) => Some(s),
            Self::Tls(s) => Some(&s.sock),
            #[cfg(unix)]
            Self::Unix(_) => None,
        }
    }

    fn set_read_timeout(&self, t: Option<std::time::Duration>) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_read_timeout(t),
            #[cfg(unix)]
            Self::Unix(s) => s.set_read_timeout(t),
            Self::Tls(s) => s.sock.set_read_timeout(t),
        }
    }

    fn set_write_timeout(&self, t: Option<std::time::Duration>) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_write_timeout(t),
            #[cfg(unix)]
            Self::Unix(s) => s.set_write_timeout(t),
            Self::Tls(s) => s.sock.set_write_timeout(t),
        }
    }

    /// Shut down `how`. A TLS stream first tells the peer it is done
    /// writing, so the peer can tell the end of the stream from a cut.
    fn shutdown(&mut self, how: Shutdown) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.shutdown(how),
            #[cfg(unix)]
            Self::Unix(s) => s.shutdown(how),
            Self::Tls(s) => {
                if how != Shutdown::Read {
                    s.conn.send_close_notify();
                    s.flush()?;
                }
                s.sock.shutdown(how)
            }
        }
    }

    /// Close, telling a TLS peer first. A failure to tell it is not
    /// reported: the connection is going away either way.
    fn close(self) {
        if let Self::Tls(mut s) = self {
            s.conn.send_close_notify();
            let _ = s.flush();
        }
    }
}

/// A stream's connection, or nothing once it is closed.
#[derive(Debug)]
pub(super) struct StreamInner(RefCell<Option<Conn>>);

impl StreamInner {
    pub(super) fn new(conn: Conn) -> Self {
        Self(RefCell::new(Some(conn)))
    }

    /// Run `f` on the open connection.
    fn with<T>(&self, f: impl FnOnce(&mut Conn) -> io::Result<T>) -> anyhow::Result<T> {
        let mut conn = self.0.borrow_mut();
        let Some(conn) = conn.as_mut() else {
            anyhow::bail!("the stream is closed");
        };
        Ok(f(conn).map_err(|e| IoError::from(timed(e)))?)
    }

    fn close(&self) {
        if let Some(conn) = self.0.borrow_mut().take() {
            conn.close();
        }
    }
}

/// A TCP connection, from `ctx.std.net.tcp.connect` or a listener's `accept`.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TcpStream(
    #[trace(static)]
    #[allocative(skip)]
    pub(super) StreamInner,
);

/// A Unix-domain socket connection, from `ctx.std.net.unix.connect` or a
/// listener's `accept`.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
pub struct UnixStream(
    #[trace(static)]
    #[allocative(skip)]
    pub(super) StreamInner,
);

/// A TLS connection over TCP, from `ctx.std.net.tls.connect`. Reads and
/// writes carry plaintext; the handshake finished before `connect` returned.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TlsStream(
    #[trace(static)]
    #[allocative(skip)]
    pub(super) StreamInner,
);

impl fmt::Display for TcpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<std.net.TcpStream>")
    }
}

impl fmt::Display for UnixStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<std.net.UnixStream>")
    }
}

impl fmt::Display for TlsStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<std.net.TlsStream>")
    }
}

// A stream closes when it is dropped, so it must not be frozen into a module
// that outlives the evaluation that opened it.
impl<'v> AllocValue<'v> for TcpStream {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> AllocValue<'v> for UnixStream {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> AllocValue<'v> for TlsStream {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "std.net.TcpStream")]
impl<'v> StarlarkValue<'v> for TcpStream {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("tcp_stream_methods", tcp_stream_methods);
        Some(RES.methods())
    }
}

#[starlark_value(type = "std.net.UnixStream")]
impl<'v> StarlarkValue<'v> for UnixStream {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("unix_stream_methods", unix_stream_methods);
        Some(RES.methods())
    }
}

#[starlark_value(type = "std.net.TlsStream")]
impl<'v> StarlarkValue<'v> for TlsStream {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("tls_stream_methods", tcp_stream_methods);
        Some(RES.methods())
    }
}

fn tcp_stream_methods(builder: &mut MethodsBuilder) {
    stream_methods(builder);
    inet_methods(builder);
}

fn unix_stream_methods(builder: &mut MethodsBuilder) {
    stream_methods(builder);
    unix_methods(builder);
}

fn inner<'v>(this: Value<'v>) -> &'v StreamInner {
    if let Some(s) = this.downcast_ref::<TcpStream>() {
        &s.0
    } else if let Some(s) = this.downcast_ref::<TlsStream>() {
        &s.0
    } else if let Some(s) = this.downcast_ref::<UnixStream>() {
        &s.0
    } else {
        unreachable!("stream methods are only bound on streams")
    }
}

fn data_bytes<'a>(data: &Either<&'a str, &'a StarlarkBytes>) -> &'a [u8] {
    match data {
        Either::Left(s) => s.as_bytes(),
        Either::Right(b) => b.as_bytes(),
    }
}

/// The most one `read` returns, whatever `n` asks for: its buffer is
/// allocated before any data arrives, so a script's `n` must not size it.
const READ_CHUNK: usize = 64 * 1024;

fn op_read(s: &StreamInner, n: u32) -> anyhow::Result<Vec<u8>> {
    s.with(|c| {
        let mut buf = vec![0; (n as usize).min(READ_CHUNK)];
        let got = c.read(&mut buf)?;
        buf.truncate(got);
        Ok(buf)
    })
}

fn op_read_exact(s: &StreamInner, n: u32) -> anyhow::Result<Vec<u8>> {
    s.with(|c| {
        // Grown as bytes arrive, so memory follows what the peer sends
        // rather than what the script asked for.
        let mut buf = Vec::new();
        c.take(n.into()).read_to_end(&mut buf)?;
        if buf.len() < n as usize {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("the peer closed after {} of {n} bytes", buf.len()),
            ));
        }
        Ok(buf)
    })
}

fn op_read_to_end(s: &StreamInner) -> anyhow::Result<Vec<u8>> {
    s.with(|c| {
        let mut buf = Vec::new();
        c.read_to_end(&mut buf)?;
        Ok(buf)
    })
}

fn op_read_to_string(s: &StreamInner) -> anyhow::Result<String> {
    let bytes = op_read_to_end(s)?;
    let text = String::from_utf8(bytes)
        .map_err(|e| IoError::from(io::Error::new(io::ErrorKind::InvalidData, e)))?;
    Ok(text)
}

fn op_write(s: &StreamInner, data: &[u8]) -> anyhow::Result<i64> {
    s.with(|c| c.write(data).map(|n| n as i64))
}

fn op_write_all(s: &StreamInner, data: &[u8]) -> anyhow::Result<NoneType> {
    s.with(|c| c.write_all(data).map(|()| NoneType))
}

fn op_flush(s: &StreamInner) -> anyhow::Result<NoneType> {
    s.with(|c| c.flush().map(|()| NoneType))
}

fn op_shutdown(s: &StreamInner, how: &str) -> anyhow::Result<NoneType> {
    let how = match how {
        "read" => Shutdown::Read,
        "write" => Shutdown::Write,
        "both" => Shutdown::Both,
        other => anyhow::bail!("shutdown() takes \"read\", \"write\" or \"both\", got {other:?}"),
    };
    s.with(|c| c.shutdown(how).map(|()| NoneType))
}

fn op_set_read_timeout(s: &StreamInner, ms: Option<u32>) -> anyhow::Result<NoneType> {
    let t = timeout(ms)?;
    s.with(|c| c.set_read_timeout(t).map(|()| NoneType))
}

fn op_set_write_timeout(s: &StreamInner, ms: Option<u32>) -> anyhow::Result<NoneType> {
    let t = timeout(ms)?;
    s.with(|c| c.set_write_timeout(t).map(|()| NoneType))
}

#[starlark_module]
fn stream_methods(builder: &mut MethodsBuilder) {
    /// Read at most `n` bytes, and at most 64 KiB: whatever arrives first, at
    /// least one byte unless the peer has closed its end, where it returns
    /// `b""`. Call it again for more.
    fn read<'v>(this: Value<'v>, #[starlark(require = pos)] n: u32) -> anyhow::Result<Vec<u8>> {
        op_read(inner(this), n)
    }

    /// `read`, returning an `(err, bytes)` pair instead of raising a
    /// `std.io.Error`.
    fn try_read<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] n: u32,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, Vec<u8>>> {
        Attempt::new(op_read(inner(this), n), eval)
    }

    /// Read exactly `n` bytes. A peer that closes first fails it with
    /// `kind == "unexpected_eof"`.
    fn read_exact<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] n: u32,
    ) -> anyhow::Result<Vec<u8>> {
        op_read_exact(inner(this), n)
    }

    /// `read_exact`, returning an `(err, bytes)` pair instead of raising a
    /// `std.io.Error`.
    fn try_read_exact<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] n: u32,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, Vec<u8>>> {
        Attempt::new(op_read_exact(inner(this), n), eval)
    }

    /// Read until the peer closes its end.
    fn read_to_end<'v>(this: Value<'v>) -> anyhow::Result<Vec<u8>> {
        op_read_to_end(inner(this))
    }

    /// `read_to_end`, returning an `(err, bytes)` pair instead of raising a
    /// `std.io.Error`.
    fn try_read_to_end<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, Vec<u8>>> {
        Attempt::new(op_read_to_end(inner(this)), eval)
    }

    /// Read until the peer closes its end, as text. Bytes that are not UTF-8
    /// fail it with `kind == "invalid_data"`.
    fn read_to_string<'v>(this: Value<'v>) -> anyhow::Result<String> {
        op_read_to_string(inner(this))
    }

    /// `read_to_string`, returning an `(err, str)` pair instead of raising a
    /// `std.io.Error`.
    fn try_read_to_string<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, String>> {
        Attempt::new(op_read_to_string(inner(this)), eval)
    }

    /// Write some of `data` and return how many bytes were written, which can
    /// be fewer than `len(data)`. Use `write_all` to write all of it.
    fn write<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] data: Either<&'v str, &'v StarlarkBytes>,
    ) -> anyhow::Result<i64> {
        op_write(inner(this), data_bytes(&data))
    }

    /// `write`, returning an `(err, int)` pair instead of raising a
    /// `std.io.Error`.
    fn try_write<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] data: Either<&'v str, &'v StarlarkBytes>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, i64>> {
        Attempt::new(op_write(inner(this), data_bytes(&data)), eval)
    }

    /// Write all of `data`, a string or bytes.
    fn write_all<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] data: Either<&'v str, &'v StarlarkBytes>,
    ) -> anyhow::Result<NoneType> {
        op_write_all(inner(this), data_bytes(&data))
    }

    /// `write_all`, returning an `(err, None)` pair instead of raising a
    /// `std.io.Error`.
    fn try_write_all<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] data: Either<&'v str, &'v StarlarkBytes>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneType>> {
        Attempt::new(op_write_all(inner(this), data_bytes(&data)), eval)
    }

    /// Send whatever is buffered. A TLS stream buffers the records it
    /// encrypts; a plain socket has nothing to flush.
    fn flush<'v>(this: Value<'v>) -> anyhow::Result<NoneType> {
        op_flush(inner(this))
    }

    /// `flush`, returning an `(err, None)` pair instead of raising a
    /// `std.io.Error`.
    fn try_flush<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneType>> {
        Attempt::new(op_flush(inner(this)), eval)
    }

    /// Shut down the `"read"` half, the `"write"` half, or `"both"`. After
    /// shutting down writes, the peer reads the end of the stream: the way to
    /// tell a peer that waits for it that the request is complete. A TLS
    /// stream tells its peer first, so the end is not mistaken for a cut.
    fn shutdown<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] how: &str,
    ) -> anyhow::Result<NoneType> {
        op_shutdown(inner(this), how)
    }

    /// `shutdown`, returning an `(err, None)` pair instead of raising a
    /// `std.io.Error`.
    fn try_shutdown<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] how: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneType>> {
        Attempt::new(op_shutdown(inner(this), how), eval)
    }

    /// Fail any later read that waits longer than `timeout_ms` with
    /// `kind == "timed_out"`. `None` waits as long as the OS does.
    fn set_read_timeout<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] timeout_ms: NoneOr<u32>,
    ) -> anyhow::Result<NoneType> {
        op_set_read_timeout(inner(this), timeout_ms.into_option())
    }

    /// `set_read_timeout`, returning an `(err, None)` pair instead of raising
    /// a `std.io.Error`.
    fn try_set_read_timeout<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] timeout_ms: NoneOr<u32>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneType>> {
        Attempt::new(
            op_set_read_timeout(inner(this), timeout_ms.into_option()),
            eval,
        )
    }

    /// Fail any later write that waits longer than `timeout_ms` with
    /// `kind == "timed_out"`. `None` waits as long as the OS does.
    fn set_write_timeout<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] timeout_ms: NoneOr<u32>,
    ) -> anyhow::Result<NoneType> {
        op_set_write_timeout(inner(this), timeout_ms.into_option())
    }

    /// `set_write_timeout`, returning an `(err, None)` pair instead of
    /// raising a `std.io.Error`.
    fn try_set_write_timeout<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] timeout_ms: NoneOr<u32>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneType>> {
        Attempt::new(
            op_set_write_timeout(inner(this), timeout_ms.into_option()),
            eval,
        )
    }

    /// Close the stream. A TLS stream tells its peer first. Closing twice is
    /// harmless; any other use after closing fails. A stream that is dropped
    /// closes by itself.
    fn close<'v>(this: Value<'v>) -> anyhow::Result<NoneType> {
        inner(this).close();
        Ok(NoneType)
    }
}

fn op_set_nodelay(s: &StreamInner, on: bool) -> anyhow::Result<NoneType> {
    s.with(|c| {
        c.tcp_socket()
            .expect("inet methods are only bound on TCP and TLS streams")
            .set_nodelay(on)
            .map(|()| NoneType)
    })
}

fn op_inet_addr(s: &StreamInner, peer: bool) -> anyhow::Result<String> {
    s.with(|c| {
        let sock = c
            .tcp_socket()
            .expect("inet methods are only bound on TCP and TLS streams");
        let addr = if peer {
            sock.peer_addr()
        } else {
            sock.local_addr()
        }?;
        Ok(addr.to_string())
    })
}

#[starlark_module]
fn inet_methods(builder: &mut MethodsBuilder) {
    /// Turn Nagle's algorithm off (`True`) so small writes go out at once,
    /// or back on (`False`).
    fn set_nodelay<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] on: bool,
    ) -> anyhow::Result<NoneType> {
        op_set_nodelay(inner(this), on)
    }

    /// `set_nodelay`, returning an `(err, None)` pair instead of raising a
    /// `std.io.Error`.
    fn try_set_nodelay<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] on: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneType>> {
        Attempt::new(op_set_nodelay(inner(this), on), eval)
    }

    /// The peer's address, as `"ip:port"`.
    fn peer_addr<'v>(this: Value<'v>) -> anyhow::Result<String> {
        op_inet_addr(inner(this), true)
    }

    /// `peer_addr`, returning an `(err, str)` pair instead of raising a
    /// `std.io.Error`.
    fn try_peer_addr<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, String>> {
        Attempt::new(op_inet_addr(inner(this), true), eval)
    }

    /// This end's address, as `"ip:port"`.
    fn local_addr<'v>(this: Value<'v>) -> anyhow::Result<String> {
        op_inet_addr(inner(this), false)
    }

    /// `local_addr`, returning an `(err, str)` pair instead of raising a
    /// `std.io.Error`.
    fn try_local_addr<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, String>> {
        Attempt::new(op_inet_addr(inner(this), false), eval)
    }
}

fn op_unix_addr(s: &StreamInner, peer: bool) -> anyhow::Result<NoneOr<String>> {
    s.with(|c| match c {
        #[cfg(unix)]
        Conn::Unix(sock) => {
            let addr = if peer {
                sock.peer_addr()
            } else {
                sock.local_addr()
            }?;
            Ok(NoneOr::from_option(
                addr.as_pathname().map(|p| p.display().to_string()),
            ))
        }
        _ => unreachable!("unix methods are only bound on Unix streams"),
    })
}

#[starlark_module]
fn unix_methods(builder: &mut MethodsBuilder) {
    /// The path the peer is bound to, or `None` for an unnamed socket, as a
    /// connecting client's usually is.
    fn peer_addr<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<String>> {
        op_unix_addr(inner(this), true)
    }

    /// `peer_addr`, returning an `(err, str | None)` pair instead of raising a
    /// `std.io.Error`.
    fn try_peer_addr<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneOr<String>>> {
        Attempt::new(op_unix_addr(inner(this), true), eval)
    }

    /// The path this end is bound to, or `None` for an unnamed socket.
    fn local_addr<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<String>> {
        op_unix_addr(inner(this), false)
    }

    /// `local_addr`, returning an `(err, str | None)` pair instead of raising
    /// a `std.io.Error`.
    fn try_local_addr<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneOr<String>>> {
        Attempt::new(op_unix_addr(inner(this), false), eval)
    }
}

/// `ctx.std.net.tcp`: connect to and listen on TCP addresses.
#[derive(Debug, derive_more::Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.net.Tcp>")]
pub struct Tcp;

starlark_simple_value!(Tcp);

#[starlark_value(type = "std.net.Tcp")]
impl<'v> StarlarkValue<'v> for Tcp {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("tcp_methods", tcp_methods);
        Some(RES.methods())
    }
}

/// `ctx.std.net.unix`: connect to and listen on Unix-domain socket paths.
#[derive(Debug, derive_more::Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.net.Unix>")]
pub struct Unix;

starlark_simple_value!(Unix);

#[starlark_value(type = "std.net.Unix")]
impl<'v> StarlarkValue<'v> for Unix {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("unix_ns_methods", unix_ns_methods);
        Some(RES.methods())
    }
}

/// `ctx.std.net.tls`: connect to TLS servers.
#[derive(Debug, derive_more::Display, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.net.Tls>")]
pub struct Tls;

starlark_simple_value!(Tls);

#[starlark_value(type = "std.net.Tls")]
impl<'v> StarlarkValue<'v> for Tls {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("tls_methods", tls_methods);
        Some(RES.methods())
    }
}

fn op_tcp_connect(addr: &str, timeout_ms: Option<u32>) -> anyhow::Result<TcpStream> {
    let t = timeout(timeout_ms)?;
    let sock = connect_tcp(addr, t).map_err(IoError::from)?;
    Ok(TcpStream(StreamInner::new(Conn::Tcp(sock))))
}

fn op_tcp_listen(addr: &str) -> anyhow::Result<TcpListener> {
    let l = std::net::TcpListener::bind(addr).map_err(IoError::from)?;
    Ok(TcpListener(ListenerInner::new(Listener::Tcp(l))))
}

#[cfg(unix)]
fn op_unix_connect(path: &str, timeout_ms: Option<u32>) -> anyhow::Result<UnixStream> {
    let deadline = timeout(timeout_ms)?.map(|t| std::time::Instant::now() + t);
    let owned = path.to_owned();
    let sock = super::within(deadline, move || {
        std::os::unix::net::UnixStream::connect(owned)
    })
    .map_err(IoError::from)?;
    Ok(UnixStream(StreamInner::new(Conn::Unix(sock))))
}

#[cfg(unix)]
fn op_unix_listen(path: &str) -> anyhow::Result<UnixListener> {
    let l = std::os::unix::net::UnixListener::bind(path).map_err(IoError::from)?;
    Ok(UnixListener(ListenerInner::new(Listener::Unix(l))))
}

#[cfg(not(unix))]
fn unsupported<T>() -> anyhow::Result<T> {
    Err(IoError::from(io::Error::new(
        io::ErrorKind::Unsupported,
        "Unix-domain sockets are not supported on this platform",
    ))
    .into())
}

#[cfg(not(unix))]
fn op_unix_connect(_path: &str, _timeout_ms: Option<u32>) -> anyhow::Result<UnixStream> {
    unsupported()
}

#[cfg(not(unix))]
fn op_unix_listen(_path: &str) -> anyhow::Result<UnixListener> {
    unsupported()
}

fn op_tls_connect(
    addr: &str,
    timeout_ms: Option<u32>,
    server_name: Option<&str>,
    ca_pem: Option<&[u8]>,
) -> anyhow::Result<TlsStream> {
    let t = timeout(timeout_ms)?;
    let stream = tls::connect(addr, t, server_name, ca_pem).map_err(IoError::from)?;
    Ok(TlsStream(StreamInner::new(Conn::Tls(Box::new(stream)))))
}

#[starlark_module]
fn tcp_methods(builder: &mut MethodsBuilder) {
    /// Connect to `addr`, `"host:port"` (an IPv6 host in brackets:
    /// `"[::1]:6379"`). Each address the host resolves to is tried in turn,
    /// all within `timeout_ms`, which bounds the name lookup too; `None`
    /// waits as long as the OS does. A failure raises `std.io.Error`, e.g.
    /// `kind == "connection_refused"` or `"timed_out"`.
    fn connect<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] addr: &str,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
    ) -> anyhow::Result<TcpStream> {
        op_tcp_connect(addr, timeout_ms.into_option())
    }

    /// `connect`, returning an `(err, stream)` pair instead of raising a
    /// `std.io.Error`.
    fn try_connect<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] addr: &str,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, TcpStream>> {
        Attempt::new(op_tcp_connect(addr, timeout_ms.into_option()), eval)
    }

    /// Listen on `addr`, `"host:port"`. Port `0` picks a free port; the
    /// listener's `local_addr()` says which.
    fn listen<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] addr: &str,
    ) -> anyhow::Result<TcpListener> {
        op_tcp_listen(addr)
    }

    /// `listen`, returning an `(err, listener)` pair instead of raising a
    /// `std.io.Error`.
    fn try_listen<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] addr: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, TcpListener>> {
        Attempt::new(op_tcp_listen(addr), eval)
    }
}

#[starlark_module]
fn unix_ns_methods(builder: &mut MethodsBuilder) {
    /// Connect to the Unix-domain socket at `path`, within `timeout_ms`;
    /// `None` waits as long as the OS does. A missing socket raises
    /// `std.io.Error` with `kind == "not_found"`.
    fn connect<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] path: &str,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
    ) -> anyhow::Result<UnixStream> {
        op_unix_connect(path, timeout_ms.into_option())
    }

    /// `connect`, returning an `(err, stream)` pair instead of raising a
    /// `std.io.Error`.
    fn try_connect<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] path: &str,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, UnixStream>> {
        Attempt::new(op_unix_connect(path, timeout_ms.into_option()), eval)
    }

    /// Listen on a new Unix-domain socket at `path`. A file already at `path`,
    /// such as one a crashed listener left behind, fails it with
    /// `kind == "addr_in_use"`: remove it first if it is stale.
    fn listen<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] path: &str,
    ) -> anyhow::Result<UnixListener> {
        op_unix_listen(path)
    }

    /// `listen`, returning an `(err, listener)` pair instead of raising a
    /// `std.io.Error`.
    fn try_listen<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] path: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, UnixListener>> {
        Attempt::new(op_unix_listen(path), eval)
    }
}

#[starlark_module]
fn tls_methods(builder: &mut MethodsBuilder) {
    /// Connect to `addr`, `"host:port"`, over TLS, and finish the handshake,
    /// all within `timeout_ms`; `None` waits as long as the OS does.
    ///
    /// The server's certificate must be valid for `server_name`, by default
    /// the host part of `addr`, and must chain to a trusted root: the
    /// operating system's certificate store, or, when `ca_pem` is given, only
    /// the certificates in it (a PEM bundle, as a string or bytes). A
    /// certificate that fails verification raises `std.io.Error` with
    /// `kind == "invalid_data"`; a `ca_pem` that holds no certificate, with
    /// `kind == "invalid_input"`.
    fn connect<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] addr: &str,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
        #[starlark(require = named, default = NoneOr::None)] server_name: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] ca_pem: NoneOr<
            Either<&'v str, &'v StarlarkBytes>,
        >,
    ) -> anyhow::Result<TlsStream> {
        let ca = ca_pem.into_option();
        op_tls_connect(
            addr,
            timeout_ms.into_option(),
            server_name.into_option(),
            ca.as_ref().map(data_bytes),
        )
    }

    /// `connect`, returning an `(err, stream)` pair instead of raising a
    /// `std.io.Error`.
    fn try_connect<'v>(
        #[allow(unused)] this: Value<'v>,
        #[starlark(require = pos)] addr: &str,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
        #[starlark(require = named, default = NoneOr::None)] server_name: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] ca_pem: NoneOr<
            Either<&'v str, &'v StarlarkBytes>,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, TlsStream>> {
        let ca = ca_pem.into_option();
        Attempt::new(
            op_tls_connect(
                addr,
                timeout_ms.into_option(),
                server_name.into_option(),
                ca.as_ref().map(data_bytes),
            ),
            eval,
        )
    }
}
