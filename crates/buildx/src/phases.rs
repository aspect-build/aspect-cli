//! Wall-clock phases of a build, for finding where a no-op build spends its time.
//!
//! Set `BUILDX_PHASES=1` and every `ctx.build` prints a table of phases to
//! stderr when it ends. Off, a phase costs one atomic load.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BUILDX_PHASES").is_some())
}

static RECORDED: Mutex<Vec<(&'static str, Duration)>> = Mutex::new(Vec::new());

/// Records the time from its creation to its drop under `name`.
pub struct Phase(Option<(&'static str, Instant)>);

pub fn start(name: &'static str) -> Phase {
    Phase(enabled().then(|| (name, Instant::now())))
}

/// Ends the phase in `slot` and starts `name` in its place.
pub fn next(slot: &mut Phase, name: &'static str) {
    *slot = start(name);
}

impl Drop for Phase {
    fn drop(&mut self) {
        if let Some((name, at)) = self.0 {
            if let Ok(mut r) = RECORDED.lock() {
                r.push((name, at.elapsed()));
            }
        }
    }
}

/// Prints the phases recorded since the last report, in the order they began
/// ending, and clears them.
pub fn report() {
    if !enabled() {
        return;
    }
    let Ok(mut recorded) = RECORDED.lock() else {
        return;
    };
    let total: Duration = recorded.iter().map(|(_, d)| *d).sum();
    eprintln!("buildx phases (wall clock, nested phases overlap):");
    for (name, d) in recorded.drain(..) {
        eprintln!("  {name:<34} {:>8.1} ms", d.as_secs_f64() * 1000.0);
    }
    let _ = total;
}
