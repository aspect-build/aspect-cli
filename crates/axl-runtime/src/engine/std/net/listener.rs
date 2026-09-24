//! `TcpListener` and `UnixListener`: accept connections, within a timeout.

use std::cell::RefCell;
use std::fmt;
use std::io;
use std::thread;
use std::time::{Duration, Instant};

use allocative::Allocative;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::none::{NoneOr, NoneType};
use starlark::values::{
    AllocValue, Heap, NoSerialize, ProvidesStaticType, StarlarkValue, Trace, Value, ValueLike,
    starlark_value,
};

use super::stream::{Conn, StreamInner, TcpStream, UnixStream};
use super::{remaining, timeout};
use crate::engine::error::{Attempt, IoError};

/// How often `accept` with a timeout looks for a connection.
const ACCEPT_POLL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub(super) enum Listener {
    Tcp(std::net::TcpListener),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixListener),
}

impl Listener {
    fn set_nonblocking(&self, on: bool) -> io::Result<()> {
        match self {
            Self::Tcp(l) => l.set_nonblocking(on),
            #[cfg(unix)]
            Self::Unix(l) => l.set_nonblocking(on),
        }
    }

    /// Accept one connection, as a stream and the peer's address.
    fn accept_once(&self) -> io::Result<(Conn, Option<String>)> {
        match self {
            Self::Tcp(l) => {
                let (s, addr) = l.accept()?;
                // BSD sockets inherit the listener's non-blocking mode.
                s.set_nonblocking(false)?;
                Ok((Conn::Tcp(s), Some(addr.to_string())))
            }
            #[cfg(unix)]
            Self::Unix(l) => {
                let (s, addr) = l.accept()?;
                s.set_nonblocking(false)?;
                let path = addr.as_pathname().map(|p| p.display().to_string());
                Ok((Conn::Unix(s), path))
            }
        }
    }

    /// Accept one connection, waiting at most `timeout`.
    fn accept(&self, timeout: Option<Duration>) -> io::Result<(Conn, Option<String>)> {
        let Some(timeout) = timeout else {
            return self.accept_once();
        };
        let deadline = Instant::now() + timeout;
        self.set_nonblocking(true)?;
        let result = loop {
            match self.accept_once() {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => match remaining(deadline) {
                    Ok(left) => thread::sleep(left.min(ACCEPT_POLL)),
                    Err(e) => break Err(e),
                },
                other => break other,
            }
        };
        self.set_nonblocking(false)?;
        result
    }
}

/// A listener's socket, or nothing once it is closed.
#[derive(Debug)]
pub(super) struct ListenerInner(RefCell<Option<Listener>>);

impl ListenerInner {
    pub(super) fn new(listener: Listener) -> Self {
        Self(RefCell::new(Some(listener)))
    }

    fn with<T>(&self, f: impl FnOnce(&Listener) -> io::Result<T>) -> anyhow::Result<T> {
        let listener = self.0.borrow();
        let Some(listener) = listener.as_ref() else {
            anyhow::bail!("the listener is closed");
        };
        Ok(f(listener).map_err(IoError::from)?)
    }
}

/// A listening TCP socket, from `ctx.std.net.tcp.listen`.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TcpListener(
    #[trace(static)]
    #[allocative(skip)]
    pub(super) ListenerInner,
);

/// A listening Unix-domain socket, from `ctx.std.net.unix.listen`.
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
pub struct UnixListener(
    #[trace(static)]
    #[allocative(skip)]
    pub(super) ListenerInner,
);

impl fmt::Display for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<std.net.TcpListener>")
    }
}

impl fmt::Display for UnixListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<std.net.UnixListener>")
    }
}

// Dropping a listener stops listening, so it must not be frozen.
impl<'v> AllocValue<'v> for TcpListener {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> AllocValue<'v> for UnixListener {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

#[starlark_value(type = "std.net.TcpListener")]
impl<'v> StarlarkValue<'v> for TcpListener {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("tcp_listener_methods", tcp_listener_methods);
        Some(RES.methods())
    }
}

#[starlark_value(type = "std.net.UnixListener")]
impl<'v> StarlarkValue<'v> for UnixListener {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("unix_listener_methods", unix_listener_methods);
        Some(RES.methods())
    }
}

fn inner<'v>(this: Value<'v>) -> &'v ListenerInner {
    if let Some(l) = this.downcast_ref::<TcpListener>() {
        &l.0
    } else if let Some(l) = this.downcast_ref::<UnixListener>() {
        &l.0
    } else {
        unreachable!("listener methods are only bound on listeners")
    }
}

fn op_tcp_accept(l: &ListenerInner, ms: Option<u32>) -> anyhow::Result<(TcpStream, String)> {
    let t = timeout(ms)?;
    let (conn, addr) = l.with(|l| l.accept(t))?;
    Ok((TcpStream(StreamInner::new(conn)), addr.unwrap_or_default()))
}

fn op_unix_accept(
    l: &ListenerInner,
    ms: Option<u32>,
) -> anyhow::Result<(UnixStream, NoneOr<String>)> {
    let t = timeout(ms)?;
    let (conn, addr) = l.with(|l| l.accept(t))?;
    Ok((
        UnixStream(StreamInner::new(conn)),
        NoneOr::from_option(addr),
    ))
}

fn op_tcp_local_addr(l: &ListenerInner) -> anyhow::Result<String> {
    l.with(|l| match l {
        Listener::Tcp(l) => Ok(l.local_addr()?.to_string()),
        #[cfg(unix)]
        Listener::Unix(_) => unreachable!("TCP listener methods are only bound on TCP listeners"),
    })
}

fn op_unix_local_addr(l: &ListenerInner) -> anyhow::Result<NoneOr<String>> {
    l.with(|l| match l {
        #[cfg(unix)]
        Listener::Unix(l) => Ok(NoneOr::from_option(
            l.local_addr()?
                .as_pathname()
                .map(|p| p.display().to_string()),
        )),
        _ => unreachable!("Unix listener methods are only bound on Unix listeners"),
    })
}

fn close(this: Value) -> anyhow::Result<NoneType> {
    inner(this).0.borrow_mut().take();
    Ok(NoneType)
}

#[starlark_module]
fn tcp_listener_methods(builder: &mut MethodsBuilder) {
    /// Wait for a connection and return `(stream, peer_addr)`. With
    /// `timeout_ms`, fail with `kind == "timed_out"` when none arrives in
    /// time; `None` waits for as long as it takes.
    fn accept<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
    ) -> anyhow::Result<(TcpStream, String)> {
        op_tcp_accept(inner(this), timeout_ms.into_option())
    }

    /// `accept`, returning an `(err, (stream, peer_addr))` pair instead of
    /// raising a `std.io.Error`.
    fn try_accept<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, (TcpStream, String)>> {
        Attempt::new(op_tcp_accept(inner(this), timeout_ms.into_option()), eval)
    }

    /// The address this listener is bound to, as `"ip:port"`: how to learn
    /// the port `listen("127.0.0.1:0")` picked.
    fn local_addr<'v>(this: Value<'v>) -> anyhow::Result<String> {
        op_tcp_local_addr(inner(this))
    }

    /// `local_addr`, returning an `(err, str)` pair instead of raising a
    /// `std.io.Error`.
    fn try_local_addr<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, String>> {
        Attempt::new(op_tcp_local_addr(inner(this)), eval)
    }

    /// Stop listening. Closing twice is harmless. A listener that is dropped
    /// closes by itself.
    fn close<'v>(this: Value<'v>) -> anyhow::Result<NoneType> {
        close(this)
    }
}

#[starlark_module]
fn unix_listener_methods(builder: &mut MethodsBuilder) {
    /// Wait for a connection and return `(stream, peer_path)`, the path
    /// being `None` for an unnamed client socket, as it usually is. With
    /// `timeout_ms`, fail with `kind == "timed_out"` when none arrives in
    /// time; `None` waits for as long as it takes.
    fn accept<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
    ) -> anyhow::Result<(UnixStream, NoneOr<String>)> {
        op_unix_accept(inner(this), timeout_ms.into_option())
    }

    /// `accept`, returning an `(err, (stream, peer_path))` pair instead of
    /// raising a `std.io.Error`.
    fn try_accept<'v>(
        this: Value<'v>,
        #[starlark(require = named, default = NoneOr::None)] timeout_ms: NoneOr<u32>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, (UnixStream, NoneOr<String>)>> {
        Attempt::new(op_unix_accept(inner(this), timeout_ms.into_option()), eval)
    }

    /// The path this listener is bound to.
    fn local_addr<'v>(this: Value<'v>) -> anyhow::Result<NoneOr<String>> {
        op_unix_local_addr(inner(this))
    }

    /// `local_addr`, returning an `(err, str | None)` pair instead of raising
    /// a `std.io.Error`.
    fn try_local_addr<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Attempt<'v, IoError, NoneOr<String>>> {
        Attempt::new(op_unix_local_addr(inner(this)), eval)
    }

    /// Stop listening. The socket file stays where it is; remove it when it
    /// is no longer needed. Closing twice is harmless.
    fn close<'v>(this: Value<'v>) -> anyhow::Result<NoneType> {
        close(this)
    }
}
