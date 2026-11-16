//! Subscribers of the blocking broadcast rings the decoded Bazel logs (the
//! execution log, the workspace log) are fanned out on.
//!
//! * [`ring`] makes one: the producer's sender and the [`Origin`] every
//!   consumer clones its subscriber from.
//! * [`Subscription`] is the state machine behind the
//!   `bazel.execution_log.iterator()` and `bazel.workspace_events.iterator()`
//!   handles: one subscriber, bound after the handle is created, that
//!   `build.wait()` has to be able to drop from outside.

use std::sync::Arc;
use std::sync::Mutex;

use fibre::TryRecvError;
use fibre::spmc::{Receiver, Sender, bounded};

use crate::fanout::{Recv, recv_cancellable};

/// How far ahead of its slowest subscriber a ring's producer may get before
/// it blocks: the producer may not lap any subscriber, so this is the whole of
/// the back-pressure budget, and nothing is dropped past it. It trades memory
/// for how long a slow consumer can be slow without costing the producer a
/// context switch per entry.
pub const CHANNEL_CAPACITY: usize = 1000;

/// A ring of [`CHANNEL_CAPACITY`]: the producer's sender, and its origin.
pub fn ring<T: Send + Clone>() -> (Sender<T>, Origin<T>) {
    let (sender, recv) = bounded(CHANNEL_CAPACITY);
    (sender, Origin { recv: Some(recv) })
}

/// The one subscriber a ring starts with, and the only thing a further
/// subscriber can be cloned from — so once it is gone, nothing can join.
///
/// It has to be dropped ([`seal`](Self::seal)) as soon as the consumers are
/// wired: in fibre's broadcast ring the producer may not lap its slowest
/// subscriber, and one nobody reads is a tail stuck at entry zero. Left in
/// place beside others it caps them all at [`CHANNEL_CAPACITY`], blocking the
/// producer there. Gone with no consumer in its place, the first send reports
/// `Closed` and the reader can skip decoding.
#[derive(Debug)]
pub struct Origin<T: Send + Clone> {
    recv: Option<Receiver<T>>,
}

impl<T: Send + Clone> Origin<T> {
    /// A further subscriber, starting where the origin is. `None` once sealed.
    pub fn receiver(&self) -> Option<Receiver<T>> {
        self.recv.as_ref().cloned()
    }

    /// Drop the origin once every consumer has cloned one. Idempotent; no
    /// consumer can be added afterwards.
    pub fn seal(&mut self) {
        self.recv.take();
    }
}

/// What a [`Subscription`] is holding.
#[derive(Debug)]
enum State<T: Send + Clone> {
    /// Created but not subscribed yet. Only [`Subscription::pending`] starts
    /// here; a `bind` leaves it for good.
    Pending,
    /// Holds a subscriber the producer blocks for. Reads come from `recv`.
    Live { recv: Receiver<T> },
    /// On loan to an in-flight blocking read, so a [`release`](Subscription::release)
    /// can land while the read is parked.
    ///
    /// Distinct from `Pending` deliberately. Were the pre-bind state doing double
    /// duty as the on-loan marker, a [`bind`](Subscription::bind) arriving
    /// mid-read would succeed against a subscription that already has a receiver,
    /// and the completing read would then decline to put its own back — one
    /// handle claiming two builds, and a subscriber leaked into `Done`.
    ///
    /// A single-threaded caller cannot reach it, since it cannot bind while
    /// parked in a read. Asserted anyway, by
    /// `a_bind_is_refused_while_a_read_is_in_flight`, because the guarantee
    /// belongs to this type rather than to the caller that happens to make it
    /// unreachable.
    Reading,
    /// Unsubscribed: the stream ended, or it was released or cancelled.
    Done,
}

/// One subscriber of a decoded Bazel log, releasable from outside.
///
/// `Clone`, and the state behind an `Arc`, so whoever binds it can keep a clone
/// and [`release`](Self::release) it before joining the producer. The producer
/// blocks for every subscriber it has, so one whose consumer stopped reading,
/// or never started, would otherwise park it on a full ring and the join would
/// never return.
#[derive(Debug)]
pub struct Subscription<T: Send + Clone> {
    state: Arc<Mutex<State<T>>>,
}

impl<T: Send + Clone> Clone for Subscription<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<T: Send + Clone> Subscription<T> {
    /// Not subscribed yet: a handle is created before the build it will be
    /// bound to exists.
    pub fn pending() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::Pending)),
        }
    }

    /// Subscribe. `false` when this subscription is not [`State::Pending`] —
    /// bound already, released, or on loan to a read — which is what makes "one
    /// handle binds once" hold.
    pub fn bind(&self, recv: Receiver<T>) -> bool {
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
    /// Called on every subscription before the producer is joined, because the
    /// subscriber each holds is one the producer blocks for. A read parked in
    /// [`recv_blocking`](Self::recv_blocking) does not hold the lock, so this
    /// lands even mid-drain and the read then declines to re-subscribe. It does
    /// not unpark a producer waiting on an in-flight read, though: that read owns
    /// the receiver for its whole duration, so the receiver itself drops only
    /// when the read returns.
    pub fn release(&self) {
        *self.state.lock().unwrap() = State::Done;
    }

    /// Whether a subscriber is held right now; `false` while a read has it on
    /// loan. For tests: nothing else needs to tell `Live` from `Reading`.
    pub fn is_live(&self) -> bool {
        matches!(*self.state.lock().unwrap(), State::Live { .. })
    }

    /// Whether this subscription has been unsubscribed. A `Pending` one has not.
    pub fn is_done(&self) -> bool {
        matches!(*self.state.lock().unwrap(), State::Done)
    }

    /// Block for the next entry `wants` accepts, or `None` at the end of the
    /// stream or once `cancelled` says so. Either unsubscribes, which also frees
    /// a producer blocked on this subscriber.
    ///
    /// Entries `wants` rejects are discarded rather than returned: a filtered-out
    /// entry carries nothing a caller could act on, and a log is mostly
    /// filtered-out entries.
    pub fn recv_blocking(
        &self,
        cancelled: impl Fn() -> bool,
        wants: impl Fn(&T) -> bool,
    ) -> Option<T> {
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
    pub fn try_pop(&self, wants: impl Fn(&T) -> bool) -> Option<T> {
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
    /// should need them. Public so the iterator handles' tests in `axl-runtime`
    /// can put a subscription on loan.
    pub fn take_for_read(&self) -> Option<Receiver<T>> {
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
    pub fn finish_read(&self, recv: Receiver<T>) {
        let mut state = self.state.lock().unwrap();
        if matches!(*state, State::Reading) {
            *state = State::Live { recv };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::tools::protos::ExecLogEntry;
    use fibre::TrySendError;
    use fibre::spmc::bounded;

    fn entry(id: u32) -> ExecLogEntry {
        ExecLogEntry { id, r#type: None }
    }

    fn bound(recv: Receiver<ExecLogEntry>) -> Subscription<ExecLogEntry> {
        let sub = Subscription::pending();
        assert!(sub.bind(recv));
        sub
    }

    /// A read in flight must not look like a subscription waiting to be bound.
    /// See [`State::Reading`] for what conflating the two would cost, and why
    /// this is asserted though a single-threaded caller cannot reach it.
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
        let sub = bound(recv);
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
        let sub = bound(recv);
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
            let sub = bound(recv);

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

    /// Why an [`Origin`] must be sealed once real subscribers exist.
    ///
    /// In fibre's broadcast ring each subscriber has its own tail and the producer
    /// may not lap the slowest. One that nobody reads sits at entry zero forever,
    /// so the ring reads as full from the producer's side however diligently every
    /// other subscriber drains — capping them all at the capacity. Blocking sends
    /// then park, which the consumer cannot see.
    #[test]
    fn an_unread_subscriber_clone_stops_the_producer_once_the_ring_fills() {
        // A small ring: the behaviour is in the tails, not the size.
        const CAPACITY: u32 = 4;
        let (sender, kept) = bounded::<ExecLogEntry>(CAPACITY as usize);
        let drained = kept.clone();

        for i in 0..CAPACITY {
            sender.try_send(entry(i)).expect("the ring has room");
        }
        for i in 0..CAPACITY {
            assert_eq!(drained.recv().expect("an entry").id, i);
        }

        // `drained` is caught up, but `kept` never read anything, so from the
        // producer's side the ring is still full.
        assert!(
            matches!(sender.try_send(entry(99)), Err(TrySendError::Full(_))),
            "an unread clone should hold the ring full even though every other \
             subscriber is caught up",
        );

        drop(kept);
        sender
            .try_send(entry(99))
            .expect("dropping the unread clone must free the ring");
        assert_eq!(drained.recv().expect("an entry").id, 99);
    }
}
