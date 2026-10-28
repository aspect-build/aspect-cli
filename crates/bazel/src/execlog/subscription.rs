//! The subscription state machine both execution-log iterators are built on.
//!
//! `build.execution_logs()` and `bazel.execution_log.iterator()` differ in what
//! they add — a `kinds=` filter and a deferred bind for the handle, neither for
//! the other — but underneath they hold the same thing: one subscriber of a
//! build's decoded execution log that the producer blocks for, and that
//! `build.wait()` has to be able to drop from outside. [`Subscription`] is that
//! thing, so the lifecycle exists once rather than once per iterator.

use std::sync::Arc;
use std::sync::Mutex;

use axl_proto::tools::protos::ExecLogEntry;
use fibre::TryRecvError;
use fibre::spmc::Receiver;

use crate::fanout::{Recv, recv_cancellable};

/// What a [`Subscription`] is holding.
#[derive(Debug)]
enum State {
    /// Created but not subscribed yet. Only [`Subscription::pending`] starts
    /// here; a `bind` leaves it for good.
    Pending,
    /// Holds a subscriber the producer blocks for. Reads come from `recv`.
    Live { recv: Receiver<ExecLogEntry> },
    /// On loan to an in-flight blocking read, so a [`release`](Subscription::release)
    /// can land while the read is parked.
    ///
    /// Distinct from `Pending` deliberately. Were the pre-bind state doing double
    /// duty as the on-loan marker, a [`bind`](Subscription::bind) arriving
    /// mid-read would succeed against a subscription that already has a receiver,
    /// and the completing read would then decline to put its own back — one
    /// handle claiming two builds, and a subscriber leaked into `Done`.
    ///
    /// Unreachable from AXL, which is single-threaded: a task cannot call
    /// `ctx.bazel.build` while parked inside `iter_next`. Asserted anyway — by
    /// `a_bind_is_refused_while_a_read_is_in_flight` here and
    /// `a_handle_mid_read_cannot_be_rebound` on the handle — because the
    /// guarantee belongs to these types rather than to the evaluator that
    /// happens to make it unreachable.
    Reading,
    /// Unsubscribed: drained, released by `build.wait()`, or ended by a cancel.
    Done,
}

/// One subscriber of a build's decoded execution log, releasable from outside.
///
/// `Clone`, and the state behind an `Arc`, for one reason: `Build` keeps a clone
/// of every subscription it handed out so `wait()` can [`release`](Self::release)
/// each one before joining the reader. The producer blocks for every subscriber
/// it has, so one an AXL task stopped reading — or never started — would
/// otherwise park the reader on a full ring and the join would never return.
#[derive(Clone, Debug)]
pub struct Subscription {
    state: Arc<Mutex<State>>,
}

impl Subscription {
    /// Already subscribed, for `build.execution_logs()`: it is handed the
    /// stream's own receiver at construction, so it has nothing to wait for.
    pub fn live(recv: Receiver<ExecLogEntry>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::Live { recv })),
        }
    }

    /// Not subscribed yet, for an `execution_log.iterator()` handle: it is
    /// created before the build it will be bound to exists.
    pub fn pending() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::Pending)),
        }
    }

    /// Subscribe. `false` when this subscription is not [`State::Pending`] —
    /// bound already, released, or on loan to a read — which is what makes "one
    /// handle binds once" hold.
    pub fn bind(&self, recv: Receiver<ExecLogEntry>) -> bool {
        let mut state = self.state.lock().unwrap();
        if matches!(*state, State::Pending) {
            *state = State::Live { recv };
            true
        } else {
            false
        }
    }

    /// Unsubscribe, dropping any entries still buffered. Idempotent.
    ///
    /// `build.wait()` calls this on every subscription it handed out, because the
    /// subscriber each holds is one the producer blocks for. A read parked in
    /// [`recv_blocking`](Self::recv_blocking) does not hold the lock, so this
    /// lands even mid-drain and the read then declines to re-subscribe. It does
    /// not unpark a producer waiting on an in-flight read, though: that read owns
    /// the receiver for its whole duration, so the receiver itself drops only
    /// when the read returns.
    pub fn release(&self) {
        *self.state.lock().unwrap() = State::Done;
    }

    /// Whether a subscriber is held right now. `false` while a read has it on
    /// loan, so this answers "is this handle bound" and not "can more entries
    /// arrive" — the latter is [`is_drained`](Self::is_drained). For tests only:
    /// nothing in the AXL path needs to distinguish `Live` from `Reading`.
    pub fn is_live(&self) -> bool {
        matches!(*self.state.lock().unwrap(), State::Live { .. })
    }

    /// Whether this subscription has been unsubscribed. A `Pending` one has not.
    pub fn is_done(&self) -> bool {
        matches!(*self.state.lock().unwrap(), State::Done)
    }

    /// Whether nothing more can arrive: unsubscribed, or the stream closed with
    /// everything buffered already taken.
    pub fn is_drained(&self) -> bool {
        match &*self.state.lock().unwrap() {
            State::Live { recv } => recv.is_closed(),
            // `Reading` is only observable from another thread, which AXL has
            // none of; either way nothing more will arrive through this handle.
            _ => true,
        }
    }

    /// Block for the next entry `wants` accepts, or the end of the stream.
    ///
    /// Cancellation-aware, as every blocking builtin must be (see
    /// `engine::cancellation`), and here that earns its keep: a drain sits on the
    /// critical path of every bazel-driving task, right before the one other
    /// cancellation-aware wait, and the end-of-stream signal is the Bazel *daemon*
    /// closing the log file — which an interrupted invocation need not make happen
    /// promptly. A cancel ends the iteration, which also drops the subscriber and
    /// so frees a reader blocked on it; an iterator's next call raises the task's
    /// exit, since an iterator cannot raise one itself.
    ///
    /// Entries `wants` rejects are discarded rather than returned: a filtered-out
    /// entry carries nothing a caller could act on, and a log is mostly
    /// filtered-out entries.
    pub fn recv_blocking(
        &self,
        cancelled: impl Fn() -> bool,
        wants: impl Fn(&ExecLogEntry) -> bool,
    ) -> Option<ExecLogEntry> {
        let recv = self.take_for_read()?;
        loop {
            match recv_cancellable(&cancelled, &recv, None) {
                Recv::Item(entry) => {
                    if !wants(&entry) {
                        continue;
                    }
                    self.finish_read(recv);
                    return Some(entry);
                }
                Recv::Closed | Recv::Cancelled => {
                    *self.state.lock().unwrap() = State::Done;
                    return None;
                }
                // No deadline to pass with `tick = None`.
                Recv::Tick => continue,
            }
        }
    }

    /// Non-blocking pop of the next entry `wants` accepts, or `None` when nothing
    /// buffered qualifies. The producer runs up to the channel capacity ahead and
    /// then waits for this subscriber, so polling slowly costs time rather than
    /// entries.
    pub fn try_pop(&self, wants: impl Fn(&ExecLogEntry) -> bool) -> Option<ExecLogEntry> {
        let mut state = self.state.lock().unwrap();
        let State::Live { recv } = &*state else {
            return None;
        };
        let mut closed = false;
        let found = loop {
            match recv.try_recv() {
                Ok(entry) => {
                    if wants(&entry) {
                        break Some(entry);
                    }
                }
                Err(TryRecvError::Empty) => break None,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break None;
                }
            }
        };
        if closed {
            *state = State::Done;
        }
        found
    }

    /// Borrow the receiver for a blocking read, leaving [`State::Reading`] behind.
    ///
    /// Taken out from under the lock so a parked read does not hold it, which is
    /// what lets [`release`](Self::release) run while a drain is in progress.
    ///
    /// Paired with [`finish_read`](Self::finish_read) by
    /// [`recv_blocking`](Self::recv_blocking), which is the only caller that
    /// should need them. Visible to the sibling iterator modules so their tests
    /// can put a subscription on loan, which AXL itself cannot observe.
    pub fn take_for_read(&self) -> Option<Receiver<ExecLogEntry>> {
        let mut state = self.state.lock().unwrap();
        match std::mem::replace(&mut *state, State::Reading) {
            State::Live { recv } => Some(recv),
            other => {
                *state = other;
                None
            }
        }
    }

    /// Re-subscribe after a read, unless a [`release`](Self::release) landed while
    /// it was parked — in which case the release wins and the receiver drops here.
    ///
    /// See [`take_for_read`](Self::take_for_read) for why this is not private.
    pub fn finish_read(&self, recv: Receiver<ExecLogEntry>) {
        let mut state = self.state.lock().unwrap();
        if matches!(*state, State::Reading) {
            *state = State::Live { recv };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fibre::spmc::bounded;

    fn entry(id: u32) -> ExecLogEntry {
        ExecLogEntry { id, r#type: None }
    }

    /// A read in flight must not look like a subscription waiting to be bound.
    /// See [`State::Reading`] for what conflating the two would cost, and why
    /// this is asserted though AXL cannot reach it.
    #[test]
    fn a_bind_is_refused_while_a_read_is_in_flight() {
        let (_s1, r1) = bounded::<ExecLogEntry>(4);
        let (_s2, r2) = bounded::<ExecLogEntry>(4);
        let sub = Subscription::pending();
        assert!(sub.bind(r1), "the first bind should take");

        let on_loan = sub.take_for_read().expect("a live subscription lends out");
        assert!(!sub.is_live(), "a lent-out subscription is not live");
        assert!(
            !sub.bind(r2),
            "a bind must not land on a subscription that is mid-read",
        );

        sub.finish_read(on_loan);
        assert!(
            sub.is_live(),
            "the read should have put its own receiver back",
        );
    }

    /// A release that lands while a read is parked wins: the read does not
    /// re-subscribe, which is what frees a reader blocked on this subscriber.
    #[test]
    fn a_release_mid_read_wins_over_the_finishing_read() {
        let (sender, recv) = bounded::<ExecLogEntry>(1);
        let sub = Subscription::live(recv);
        let on_loan = sub.take_for_read().expect("a live subscription lends out");

        sub.release();
        sub.finish_read(on_loan);

        assert!(sub.is_done(), "the release should have stuck");
        assert!(
            sender.send(entry(1)).is_err(),
            "with the only subscriber released the send should report Closed",
        );
    }

    /// `try_pop` skips what the filter rejects, and marks the subscription done
    /// when the stream has closed behind the last entry.
    #[test]
    fn try_pop_filters_and_notices_the_close() {
        let (sender, recv) = bounded::<ExecLogEntry>(8);
        let sub = Subscription::live(recv);
        for id in 1..=4 {
            sender.send(entry(id)).unwrap();
        }
        let even = |e: &ExecLogEntry| e.id.is_multiple_of(2);
        assert_eq!(sub.try_pop(even).map(|e| e.id), Some(2));
        assert_eq!(sub.try_pop(even).map(|e| e.id), Some(4));
        assert_eq!(sub.try_pop(even).map(|e| e.id), None, "nothing buffered");
        assert!(!sub.is_done(), "an empty-but-open stream is not done");

        drop(sender);
        assert_eq!(sub.try_pop(even).map(|e| e.id), None);
        assert!(sub.is_done(), "a closed stream should unsubscribe");
    }

    /// A cancelled run ends a blocking read rather than parking in it, and
    /// unsubscribes on the way out.
    ///
    /// Timed out because the regression is a park, not a wrong answer: the
    /// channel below is never written to and never closed, so a read that missed
    /// the cancel waits forever and reports nothing.
    #[test]
    fn a_cancel_ends_a_blocking_read() {
        let outcome = crate::testing::with_timeout(std::time::Duration::from_secs(10), || {
            // Never written to and never closed: there is nothing to return and no
            // end-of-stream to end on.
            let (_sender, recv) = bounded::<ExecLogEntry>(4);
            let sub = Subscription::live(recv);

            (
                sub.recv_blocking(|| true, |_| true).is_none(),
                sub.is_done(),
            )
        });
        let Some((ended, unsubscribed)) = outcome else {
            panic!(
                "timed out: a cancelled run parked in `recv_blocking` instead of ending the read"
            )
        };
        assert!(ended, "a cancelled read should return nothing");
        assert!(unsubscribed, "a cancelled read should unsubscribe");
    }

    /// `is_drained` is what `done` reports, so it has to mean "the stream ended
    /// *and* everything it sent has been taken" — not merely that the producer
    /// is gone. A subscription with no receiver to ask answers yes.
    #[test]
    fn is_drained_needs_the_buffer_empty_as_well_as_the_stream_closed() {
        let (sender, recv) = bounded::<ExecLogEntry>(4);
        let sub = Subscription::live(recv);
        sender.send(entry(1)).unwrap();
        assert!(!sub.is_drained(), "an open stream is not drained");

        drop(sender);
        assert!(
            !sub.is_drained(),
            "an entry still buffered means the caller has not seen everything yet",
        );
        assert_eq!(sub.try_pop(|_| true).map(|e| e.id), Some(1));
        assert!(sub.is_drained(), "closed and emptied is drained");

        assert!(
            Subscription::pending().is_drained(),
            "with no subscriber to ask, nothing more can arrive",
        );
        let (_s, r) = bounded::<ExecLogEntry>(4);
        let released = Subscription::live(r);
        released.release();
        assert!(released.is_drained(), "a released subscription is drained");
    }
}
