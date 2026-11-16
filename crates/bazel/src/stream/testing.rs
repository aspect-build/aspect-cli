//! A [`Gate`] on flags a test flips, for the streams' tests and those of the
//! readers built on them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use super::Gate;

/// [`Flags::gate`]'s [`Gate::quiet`]: short, so a test that waits it out is
/// quick.
pub(crate) const QUIET_TEST: Duration = Duration::from_millis(200);

/// [`Flags::gate`]'s [`Gate::quiet_empty`].
pub(crate) const QUIET_EMPTY_TEST: Duration = Duration::from_millis(100);

/// Give `gate` the test windows, [`QUIET_TEST`] and [`QUIET_EMPTY_TEST`].
pub(crate) fn test_windows(gate: &mut Gate) {
    gate.quiet = QUIET_TEST;
    gate.quiet_empty = QUIET_EMPTY_TEST;
}

/// Nothing arrives on `rx` for a while: what a reader still waiting looks
/// like from outside.
pub(crate) fn still_waiting<T>(rx: &Receiver<T>) -> bool {
    matches!(
        rx.recv_timeout(Duration::from_millis(400)),
        Err(RecvTimeoutError::Timeout)
    )
}

/// What a reader on another thread sent back once it ended.
pub(crate) fn finished<T>(rx: Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(10))
        .expect("the read should have ended")
}

/// The owner and the writers, as a gate sees them. They start alive and
/// holding the stream; clones share the flags.
#[derive(Clone)]
pub(crate) struct Flags {
    pub(crate) ended: Arc<AtomicBool>,
    pub(crate) held: Arc<AtomicBool>,
    pub(crate) ended_calls: Arc<AtomicUsize>,
    pub(crate) held_calls: Arc<AtomicUsize>,
}

impl Flags {
    pub(crate) fn new() -> Self {
        Self {
            ended: Arc::new(AtomicBool::new(false)),
            held: Arc::new(AtomicBool::new(true)),
            ended_calls: Arc::new(AtomicUsize::new(0)),
            held_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The owner ends.
    pub(crate) fn end(&self) {
        self.ended.store(true, Ordering::SeqCst);
    }

    /// The writers let go.
    pub(crate) fn release(&self) {
        self.held.store(false, Ordering::SeqCst);
    }

    /// A gate that reads these flags and counts its calls, with the test
    /// windows. Without `with_held` it has no `held` at all.
    pub(crate) fn gate(&self, with_held: bool) -> Gate {
        let (e, ec) = (self.ended.clone(), self.ended_calls.clone());
        let mut gate = Gate::new(move || {
            ec.fetch_add(1, Ordering::SeqCst);
            e.load(Ordering::SeqCst)
        });
        if with_held {
            let (h, hc) = (self.held.clone(), self.held_calls.clone());
            gate = gate.with_held(move |_| {
                hc.fetch_add(1, Ordering::SeqCst);
                h.load(Ordering::SeqCst)
            });
        }
        test_windows(&mut gate);
        gate
    }
}
