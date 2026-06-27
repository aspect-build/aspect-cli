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

use std::collections::BTreeMap;
use std::io;
use std::process::Command;
use std::process::Stdio;
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
    /// The single fork primitive: the base `Command` for a bazel invocation
    /// under this backend, with `startup_flags` already applied (before any
    /// subcommand). Every verb method below goes through here, so the
    /// Real/Fake choice is made in exactly one place.
    ///
    /// `Real` defers to the shared `bazel_command()` helper (which sets the
    /// anti-inception env var and honors `BAZEL_REAL`). `Fake` builds the
    /// `Command` straight from `fake_bin` — deliberately NOT via
    /// `bazel_command()`, so the fake path touches no global env state.
    pub(crate) fn base_command(&self, startup_flags: &[String]) -> Command {
        let mut cmd = match self {
            BazelBackend::Real => super::bazel_command(),
            BazelBackend::Fake { fake_bin, .. } => Command::new(fake_bin),
        };
        cmd.args(startup_flags);
        cmd
    }

    /// Base command with no startup flags. Retained for the `build`/`test`
    /// spawn path (`build.rs`), which applies startup flags itself.
    pub fn command(&self) -> Command {
        self.base_command(&[])
    }

    /// Run `bazel [startup_flags] info [keys...]` and return the parsed
    /// `key → value` map. Empty `keys` asks bazel for every key.
    pub fn info(
        &self,
        startup_flags: &[String],
        keys: &[&str],
    ) -> io::Result<BTreeMap<String, String>> {
        let mut cmd = self.base_command(startup_flags);
        cmd.arg("info");
        cmd.args(keys);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());
        let (child, _guard) = super::live::spawn_registered(&mut cmd)
            .map_err(|e| io::Error::other(format!("failed to spawn bazel: {e}")))?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stderr = stderr.trim();
            let detail = if stderr.is_empty() {
                format!("exit code {:?}", out.status.code())
            } else {
                format!("exit code {:?}: {stderr}", out.status.code())
            };
            return Err(io::Error::other(format!("bazel info failed ({detail})")));
        }
        Ok(super::info::parse_info_map(&String::from_utf8_lossy(
            &out.stdout,
        )))
    }

    /// Query `server_pid` + `release` in one `bazel info` call.
    ///
    /// The version is `None` for a non-release build (see
    /// [`super::info::parse_release`]); the pid is required.
    pub fn server_info(
        &self,
        startup_flags: &[String],
    ) -> io::Result<(u32, Option<semver::Version>)> {
        let map = self.info(startup_flags, &["server_pid", "release"])?;
        let pid = map
            .get("server_pid")
            .and_then(|v| v.parse::<u32>().ok())
            .ok_or_else(|| io::Error::other("bazel info did not return server_pid"))?;
        let version = match map.get("release") {
            Some(value) => {
                let parsed = super::info::parse_release(value);
                if parsed.is_none() {
                    tracing::debug!(
                        release = %value,
                        "bazel reported a non-release version; \
                         version-conditional flags will assume latest"
                    );
                }
                parsed
            }
            None => None,
        };
        Ok((pid, version))
    }

    /// Determine the real bazel client PID via `--noblock_for_lock info
    /// server_pid`. When another invocation holds the lock, bazel exits 9 with
    /// `"Another command (pid=12345) is running."` on stderr; we parse it out.
    pub fn client_pid(&self, startup_flags: &[String]) -> Option<u32> {
        let mut cmd = self.base_command(startup_flags);
        cmd.arg("--noblock_for_lock").arg("info").arg("server_pid");
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());
        let (child, _guard) = super::live::spawn_registered(&mut cmd).ok()?;
        let output = child.wait_with_output().ok()?;
        // Exit code 9 means the lock is held — stderr carries the client PID.
        if output.status.code() != Some(9) {
            return None;
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let start = stderr.find("pid=")? + 4;
        let rest = &stderr[start..];
        let end = rest.find(')')?;
        rest[..end].parse::<u32>().ok()
    }

    /// Whether the bazel server lock is currently held (exit 9 from the
    /// non-blocking probe).
    pub fn is_server_busy(&self, startup_flags: &[String]) -> bool {
        let mut cmd = self.base_command(startup_flags);
        cmd.arg("--noblock_for_lock").arg("info").arg("server_pid");
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        cmd.stdin(Stdio::null());
        let Ok((child, _guard)) = super::live::spawn_registered(&mut cmd) else {
            return false;
        };
        matches!(child.wait_with_output(), Ok(o) if o.status.code() == Some(9))
    }

    /// Server PID without blocking on the lock: resolve `output_base` via
    /// `--noblock_for_lock info output_base` (computed client-side) and read
    /// `<output_base>/server/server.pid.txt`. `None` if the server isn't
    /// running or bazel is unavailable.
    pub fn server_pid_nonblocking(&self, startup_flags: &[String]) -> Option<u32> {
        let mut cmd = self.base_command(startup_flags);
        cmd.arg("--noblock_for_lock").arg("info").arg("output_base");
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        cmd.stdin(Stdio::null());
        let (child, _guard) = super::live::spawn_registered(&mut cmd).ok()?;
        let output = child.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }
        let output_base = String::from_utf8_lossy(&output.stdout);
        let pid_path = std::path::Path::new(output_base.trim()).join("server/server.pid.txt");
        let contents = std::fs::read_to_string(pid_path).ok()?;
        contents.trim().parse::<u32>().ok()
    }

    /// Server info to seed a build with: `(server_pid, release_version)`.
    ///
    /// `Real` probes the live daemon. `Fake` has no daemon to probe — the
    /// fake child we are about to fork is its own BES writer — so it returns
    /// `(0, None)`; the real per-invocation pid is supplied post-spawn by
    /// [`bes_server_pid`]. The `None` version makes the announce line read
    /// "development version", same as a non-release real bazel.
    ///
    /// [`bes_server_pid`]: Self::bes_server_pid
    pub fn build_server_info(
        &self,
        startup_flags: &[String],
    ) -> io::Result<(u32, Option<semver::Version>)> {
        match self {
            BazelBackend::Real => self.server_info(startup_flags),
            BazelBackend::Fake { .. } => Ok((0, None)),
        }
    }

    /// The pid that owns the BEP file for an invocation whose client process
    /// is `child_pid`. Real bazel's BEP file is written by the long-lived
    /// daemon (`probed_daemon_pid` from [`build_server_info`]); the fake child
    /// writes it itself, so it IS its own server and galvanize's
    /// `IfOpenForPid` liveness watches the child pid.
    ///
    /// [`build_server_info`]: Self::build_server_info
    pub fn bes_server_pid(&self, child_pid: u32, probed_daemon_pid: u32) -> u32 {
        match self {
            BazelBackend::Real => probed_daemon_pid,
            BazelBackend::Fake { .. } => child_pid,
        }
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
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;

    use nix::libc;

    /// A Unix `socketpair`-backed control channel. Built with
    /// [`UnixStream::pair`] — a connected `AF_UNIX`/`SOCK_STREAM` socketpair
    /// straight from std, so no extra `nix` features are pulled into the crate
    /// graph. The parent keeps one end (for convenient writes); the raw fd of
    /// the other is inherited by the child and learned via `ASPECT_FAKE_BAZEL_FD`.
    pub struct SocketPairChannel {
        /// Parent write/read end. `take`n once the expectation is sent so the
        /// write half closes and the child's `read_to_end` terminates.
        parent: Option<UnixStream>,
        /// Child end. Held (keeping the fd open) until the child is spawned.
        child: UnixStream,
    }

    impl SocketPairChannel {
        pub fn new() -> std::io::Result<Self> {
            let (parent, child) = UnixStream::pair()?;
            Ok(Self {
                parent: Some(parent),
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
    /// `UnixStream::pair()` sets `FD_CLOEXEC` on both ends (std sets it on every
    /// fd it creates), so without intervention the child end would be closed on
    /// `exec`. We clear `FD_CLOEXEC` on the inherited fd in a `pre_exec` hook so
    /// it survives into the fake bazel process.
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
