//! Reading what bazel writes.
//!
//! Bazel hands its outputs over as files and FIFOs written by processes that
//! start late, flush after the command has finished, crash, or never start.
//! This crate turns them into streams that end for a reason.
//!
//! * [`stream`]: following a stream bazel's processes are still writing, and
//!   deciding when it is over. Knows nothing about bazel's protocols.
//! * [`process`]: the per-OS questions about other processes that sit under
//!   [`stream::pid_pair`].
//! * [`delimited`]: the length-delimited framing the protocol readers share.
//! * [`bes`]: the Build Event Protocol, built on a [`stream`].
//! * [`label`] and [`runfiles`]: bazel's label spelling and runfiles trees.
//! * [`rc`]: `.bazelrc` files, and the flags they resolve to for a command.
//! * [`fanout`]: one producer's events to many subscribers.
//! * [`bes`], [`execlog`], [`workspace_event`]: one module per output, each
//!   with the feed that reads it and, where there is one, the sinks that
//!   forward it.
//!
//! Dependencies point one way: `bes` uses `delimited` and `stream`, `stream`
//! uses `process`.

pub mod aspect;
pub mod bes;
pub mod command;
pub mod delimited;
pub mod execlog;
pub mod fanout;
pub mod label;
pub mod process;
pub mod rc;
pub mod runfiles;
pub mod runner;
pub mod server;
pub mod stream;
pub mod workspace_event;

#[cfg(test)]
mod testing;
