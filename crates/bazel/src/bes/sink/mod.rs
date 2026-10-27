//! Subscribers that consume a build's BES events and forward them somewhere.

use std::sync::OnceLock;

pub mod grpc;
pub mod retry;
pub mod tracing;

static WARN: OnceLock<fn(&str)> = OnceLock::new();

/// Route the sinks' user-facing warnings. A sink never fails the build, so a
/// warning is the only way the user learns events were delayed or lost; how it
/// reaches them is the host's. The first call wins. Without one, warnings are
/// `tracing` events.
pub fn set_warn(f: fn(&str)) {
    let _ = WARN.set(f);
}

fn warn(msg: &str) {
    match WARN.get() {
        Some(f) => f(msg),
        None => ::tracing::warn!("{msg}"),
    }
}
