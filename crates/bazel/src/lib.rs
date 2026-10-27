//! Reading what bazel writes.
//!
//! Bazel hands its outputs over as files and FIFOs written by processes that
//! start late, flush after the command has finished, crash, or never start.
//! This crate turns them into streams that end for a reason.
//!
//! * [`stream`]: following a stream bazel's processes are still writing, and
//!   deciding when it is over. Knows nothing about bazel's protocols.
//! * [`formats`]: decoding what is in it. Each output bazel writes has a
//!   module here, built on a [`stream`].
//! * [`process`]: the per-OS questions about other processes that sit under
//!   [`stream::pid_pair`].
//!
//! Dependencies point one way: `formats` uses `stream`, `stream` uses
//! `process`.

pub mod formats;
pub mod process;
pub mod stream;
