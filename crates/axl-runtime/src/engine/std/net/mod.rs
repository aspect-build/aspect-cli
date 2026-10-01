//! `std.net`: blocking TCP, Unix-domain and TLS streams and listeners.
//!
//! `ctx.std.net.tcp`, `.unix` and `.tls` are namespaces with the same verbs,
//! so code that takes a transport can call `transport.connect(addr)` without
//! caring which one it got. Every call blocks; `timeout_ms = None` means the
//! operating system's behavior, as in Rust's `std::net`.
//!
//! A failed I/O operation raises `std.io.Error` (an [`IoError`] on the Rust
//! side), and each method that can raise one has a `try_` twin returning an
//! [`Attempt`] pair instead.
//!
//! [`IoError`]: crate::engine::error::IoError
//! [`Attempt`]: crate::engine::error::Attempt
//! Streams and listeners cannot be frozen: dropping one closes it, so nothing
//! outlives the evaluation that opened it.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use allocative::Allocative;
use derive_more::Display;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::starlark_value_as_type::StarlarkValueAsType;
use starlark::values::{self, NoSerialize, ProvidesStaticType, starlark_value};

mod listener;
mod stream;
mod tls;

#[cfg(test)]
mod tests;

pub use listener::{TcpListener, UnixListener};
pub use stream::{TcpStream, TlsStream, UnixStream};

/// `ctx.std.net`.
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
        static RES: MethodsStatic = MethodsStatic::new("net_methods", net_methods);
        Some(RES.methods())
    }
}

starlark_simple_value!(Net);

#[starlark_module]
fn net_methods(registry: &mut MethodsBuilder) {
    /// TCP streams and listeners.
    #[starlark(attribute)]
    fn tcp<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<stream::Tcp> {
        Ok(stream::Tcp)
    }

    /// Unix-domain streams and listeners. On other platforms every call
    /// fails with `kind == "unsupported"`.
    #[starlark(attribute)]
    fn unix<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<stream::Unix> {
        Ok(stream::Unix)
    }

    /// TLS client streams over TCP.
    #[starlark(attribute)]
    fn tls<'v>(#[allow(unused)] this: values::Value<'v>) -> anyhow::Result<stream::Tls> {
        Ok(stream::Tls)
    }
}

#[starlark_module]
pub(super) fn register_net_types(globals: &mut GlobalsBuilder) {
    const TcpStream: StarlarkValueAsType<TcpStream> = StarlarkValueAsType::new();
    const TcpListener: StarlarkValueAsType<TcpListener> = StarlarkValueAsType::new();
    const UnixStream: StarlarkValueAsType<UnixStream> = StarlarkValueAsType::new();
    const UnixListener: StarlarkValueAsType<UnixListener> = StarlarkValueAsType::new();
    const TlsStream: StarlarkValueAsType<TlsStream> = StarlarkValueAsType::new();
    const Tcp: StarlarkValueAsType<stream::Tcp> = StarlarkValueAsType::new();
    const Unix: StarlarkValueAsType<stream::Unix> = StarlarkValueAsType::new();
    const Tls: StarlarkValueAsType<stream::Tls> = StarlarkValueAsType::new();
}

/// A timeout argument as a `Duration`. Zero is refused: Rust reads a zero
/// socket timeout as an error, and "never wait" is not what anyone means.
fn timeout(ms: Option<u32>) -> anyhow::Result<Option<Duration>> {
    match ms {
        None => Ok(None),
        Some(0) => {
            anyhow::bail!("timeout_ms must be positive, or None to wait as long as the OS does")
        }
        Some(ms) => Ok(Some(Duration::from_millis(ms.into()))),
    }
}

/// The time left before `deadline`, or a `timed_out` error once it passed.
fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "timed out"))
}

/// Run `f`, giving up with `timed_out` at `deadline`. For the blocking calls
/// Rust offers no timeout for (name resolution, a Unix connect): `f` runs on
/// its own thread, which a timeout abandons to finish on its own.
fn within<T: Send + 'static>(
    deadline: Option<Instant>,
    f: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    let Some(deadline) = deadline else {
        return f();
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(remaining(deadline)?)
        .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "timed out")))
}

/// Connect to `addr` (`"host:port"`), trying each address it resolves to in
/// turn within one budget, and returning the last failure.
fn connect_tcp(addr: &str, timeout: Option<Duration>) -> io::Result<std::net::TcpStream> {
    let deadline = timeout.map(|t| Instant::now() + t);
    let owned = addr.to_owned();
    let addrs: Vec<SocketAddr> = within(deadline, move || Ok(owned.to_socket_addrs()?.collect()))?;
    let mut last = io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{addr} resolved to no addresses"),
    );
    for a in &addrs {
        let attempt = match deadline {
            None => std::net::TcpStream::connect(a),
            Some(d) => std::net::TcpStream::connect_timeout(a, remaining(d)?),
        };
        match attempt {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// On a blocking socket, a read or write that runs out its timeout reports
/// `WouldBlock` on Unix and `TimedOut` on Windows. It is one condition.
fn timed(err: io::Error) -> io::Error {
    if err.kind() == io::ErrorKind::WouldBlock {
        io::Error::new(io::ErrorKind::TimedOut, "timed out")
    } else {
        err
    }
}

/// The host part of `"host:port"`, without IPv6 brackets.
fn host_of(addr: &str) -> &str {
    let host = addr.rsplit_once(':').map_or(addr, |(h, _)| h);
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}
