//! `BazelBackend` — which bazel a `ctx.bazel.*` invocation actually drives.
//!
//! Two implementations of one contract (see `docs/testing.md`, decisions 6/7):
//!
//!   - [`BazelBackend::Real`] — production. Spawns whatever `bazel_command()`
//!     resolves (honoring `BAZEL_REAL`). The path is unchanged from before
//!     this module existed.
//!   - [`BazelBackend::Fake`] — testing. Spawns a generic fake-bazel binary
//!     (`basil` today; a shipped `aspect` self-exec subcommand later) with the
//!     fake path supplied **directly on the value** — never via the
//!     process-global `BAZEL_REAL` env var, so concurrent test workers don't
//!     race over one global. A declared [`BazelExpectation`] is handed to the
//!     fake over an inherited control channel (a `socketpair`); the fake
//!     synthesizes a consistent BES stream onto the real
//!     `--build_event_binary_file` the parent already wires, so the production
//!     `BuildEventIter` read path is exercised unchanged.
//!
//! The control transport sits behind the [`ControlChannel`] trait so a Windows
//! named-pipe / loopback implementation is a drop-in later; only a Unix
//! `socketpair` impl ships in this slice.

use std::process::Command;
use std::sync::Arc;

use allocative::Allocative;
use basil_core::BazelExpectation;

/// How `ctx.bazel.*` reaches bazel. Cloneable + carried on the `Bazel`
/// Starlark value so every invocation derives its own per-spawn resources
/// (no process-global state — a hard requirement under the parallel test
/// runner).
#[derive(Clone, Debug, Allocative)]
pub enum BazelBackend {
    /// Production: spawn the real (or `BAZEL_REAL`) bazel via `bazel_command()`.
    Real,
    /// Testing: spawn `fake_bin` directly and feed it `expectation` over the
    /// control channel.
    Fake {
        /// Absolute path to the fake-bazel binary to spawn.
        fake_bin: String,
        /// The declared fixture handed to the fake for this invocation.
        #[allocative(skip)]
        expectation: Arc<BazelExpectation>,
    },
}

impl Default for BazelBackend {
    fn default() -> Self {
        BazelBackend::Real
    }
}

impl BazelBackend {
    /// Build the base `Command` for a bazel invocation under this backend.
    ///
    /// `Real` defers to the shared `bazel_command()` helper (which sets the
    /// anti-inception env var and honors `BAZEL_REAL`). `Fake` builds the
    /// `Command` straight from `fake_bin` — deliberately NOT via
    /// `bazel_command()`, so the fake path touches no global env state.
    pub fn command(&self) -> Command {
        match self {
            BazelBackend::Real => super::bazel_command(),
            BazelBackend::Fake { fake_bin, .. } => Command::new(fake_bin),
        }
    }

    pub fn is_fake(&self) -> bool {
        matches!(self, BazelBackend::Fake { .. })
    }
}

/// Per-invocation control channel between the parent (aspect) and the fake
/// bazel child. Behind a trait so a non-Unix transport (named pipe, loopback
/// socket) can be a drop-in later; only [`SocketPairChannel`] ships now.
///
/// Bidirectional by construction (decision 7) so later cancellation tests can
/// drive the fake mid-stream — this slice only writes the fixture parent→child.
pub trait ControlChannel: Send {
    /// The raw fd number the child should read the fixture from. The parent
    /// passes this to the child via `ASPECT_FAKE_BAZEL_FD` and arranges for it
    /// to survive `exec` (see [`prepare_command`]).
    fn child_fd(&self) -> i32;

    /// Write the serialized [`BazelExpectation`] frame to the parent end, then
    /// shut the write half so the child's `read_to_end` terminates.
    fn send_expectation(&mut self, exp: &BazelExpectation) -> std::io::Result<()>;
}

/// Env var naming the inherited control fd. Must match basil's `FAKE_FD_ENV`.
pub const FAKE_FD_ENV: &str = "ASPECT_FAKE_BAZEL_FD";

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::Write;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;

    use nix::libc;
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};

    /// A Unix `socketpair`-backed control channel. The parent keeps one end
    /// (as a `UnixStream` for convenient writes); the raw fd of the other is
    /// inherited by the child and learned via `ASPECT_FAKE_BAZEL_FD`.
    pub struct SocketPairChannel {
        /// Parent write/read end. `take`n once the expectation is sent so the
        /// write half closes and the child's `read_to_end` terminates.
        parent: Option<UnixStream>,
        /// Child end. Held (keeping the fd open) until the child is spawned.
        child: OwnedFd,
    }

    impl SocketPairChannel {
        pub fn new() -> std::io::Result<Self> {
            let (parent, child) = socketpair(
                AddressFamily::Unix,
                SockType::Stream,
                None,
                SockFlag::empty(),
            )
            .map_err(|e| std::io::Error::other(format!("socketpair: {e}")))?;
            Ok(Self {
                parent: Some(UnixStream::from(parent)),
                child,
            })
        }
    }

    impl ControlChannel for SocketPairChannel {
        fn child_fd(&self) -> i32 {
            self.child.as_raw_fd()
        }

        fn send_expectation(&mut self, exp: &BazelExpectation) -> std::io::Result<()> {
            let frame = exp.encode_frame();
            let mut parent = self
                .parent
                .take()
                .ok_or_else(|| std::io::Error::other("control channel already sent"))?;
            parent.write_all(&frame)?;
            parent.flush()?;
            // Dropping `parent` here closes the parent write half, so the
            // child's `read_to_end` on the control fd returns.
            drop(parent);
            Ok(())
        }
    }

    /// Arrange for `cmd` to inherit the child control fd and learn its number
    /// via `ASPECT_FAKE_BAZEL_FD`.
    ///
    /// `std::process::Command` leaves CLOEXEC as-is on fds it doesn't manage,
    /// and `socketpair(2)` fds are not CLOEXEC by default, so the fd survives
    /// `exec` — but we clear CLOEXEC explicitly in a `pre_exec` hook to be
    /// robust against the flag being set elsewhere.
    pub fn prepare_command(cmd: &mut Command, child_fd: i32) {
        cmd.env(FAKE_FD_ENV, child_fd.to_string());
        // SAFETY: `pre_exec` runs in the forked child before exec. We only call
        // async-signal-safe libc `fcntl` on a single fd; no allocation, no
        // locks, no Rust runtime state touched.
        unsafe {
            cmd.pre_exec(move || {
                let flags = libc::fcntl(child_fd, libc::F_GETFD);
                if flags < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let cleared = flags & !libc::FD_CLOEXEC;
                if libc::fcntl(child_fd, libc::F_SETFD, cleared) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
}

#[cfg(unix)]
pub use unix::{SocketPairChannel, prepare_command};

/// Open a fresh per-invocation control channel for a `Fake` spawn.
#[cfg(unix)]
pub fn open_control_channel() -> std::io::Result<Box<dyn ControlChannel>> {
    Ok(Box::new(SocketPairChannel::new()?))
}

/// TODO(windows): a named-pipe / loopback `ControlChannel` impl. The trait
/// seam exists so this is a drop-in; the `Fake` backend is Unix-only today.
#[cfg(not(unix))]
pub fn open_control_channel() -> std::io::Result<Box<dyn ControlChannel>> {
    Err(std::io::Error::other(
        "the fake bazel backend is Unix-only in this slice (TODO: Windows transport)",
    ))
}
