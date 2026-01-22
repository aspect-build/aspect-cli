//! A thread-safe, non-blocking broadcaster for fan-out event distribution.
//!
//! # Overview
//!
//! The [`Broadcaster`] is a multi-producer, multi-consumer (MPMC) channel variant
//! designed specifically for event streaming scenarios where:
//!
//! - A single producer generates events that must be delivered to multiple consumers
//! - Consumers (subscribers) may join at any time during the stream
//! - Consumers may process events at different rates
//! - The producer must never block, regardless of consumer behavior
//!
//! # Design Constraints
//!
//! This broadcaster was designed with the following constraints in mind:
//!
//! ## 1. Non-blocking Producer
//!
//! The [`Broadcaster::send`] method must **never block**, regardless of:
//! - How many subscribers exist
//! - How slow subscribers are consuming events
//! - Whether subscribers have died or been dropped
//!
//! This is critical for event streaming from external processes (like Bazel's
//! Build Event Protocol) where blocking the producer could cause deadlocks or
//! buffer overflows in the source process.
//!
//! ## 2. Independent Subscriber Buffers
//!
//! Each subscriber has its own independent unbounded buffer (via `mpsc::channel`).
//! This means:
//!
//! - **Slow subscribers don't block fast subscribers**: If one consumer is slow,
//!   other consumers continue receiving events normally.
//!
//! - **Subscribers can consume at their own pace**: A subscriber that processes
//!   events slowly will simply accumulate events in its buffer.
//!
//! - **Memory usage scales with lagging subscribers**: A subscriber that never
//!   reads will accumulate all events in memory. This is intentional - the
//!   alternative (bounded channels) would require blocking the producer.
//!
//! ## 3. Late Subscribers
//!
//! Subscribers that join after events have been sent will **only receive future
//! events**. Past events are not replayed. This is the expected behavior for
//! a simple broadcaster where:
//!
//! - Events represent real-time occurrences
//! - Replaying history is not the broadcaster's responsibility
//! - Higher-level code (like `BuildEventStream`) can implement buffering if needed
//!
//! If you need late subscribers to receive all events from the beginning,
//! implement buffering at the stream level, not in the broadcaster.
//!
//! ## 4. Dead Subscriber Cleanup
//!
//! When a [`Subscriber`] is dropped (either explicitly or because it went out
//! of scope), the broadcaster automatically cleans up its sender on the next
//! [`Broadcaster::send`] call. This happens because:
//!
//! 1. The subscriber's `Receiver` is dropped
//! 2. The next `send()` attempts to send to all subscribers
//! 3. Sending to a dropped receiver returns `Err`
//! 4. Failed senders are removed from the subscriber list via `retain()`
//!
//! This lazy cleanup approach is efficient - no explicit "unsubscribe" is needed,
//! and cleanup happens naturally during normal operation.
//!
//! ## 5. Explicit Close for Lifecycle Management
//!
//! The [`Broadcaster::close`] method provides explicit lifecycle control:
//!
//! - Sets a `closed` flag to prevent new subscriptions from receiving events
//! - Drops all sender handles, causing all receivers to see channel disconnect
//! - Enables clean shutdown without requiring all references to be dropped
//!
//! This is critical for avoiding deadlocks in scenarios like:
//!
//! ```text
//! // Without close(): DEADLOCK
//! for event in stream.subscribe() {  // blocks waiting for events
//!     process(event);
//! }
//! stream.wait();  // never reached - for loop never exits
//!
//! // With close(): Works correctly
//! // Producer calls close() when done, subscriber sees disconnect, loop exits
//! ```
//!
//! ## 6. Thread Safety
//!
//! The broadcaster is fully thread-safe:
//!
//! - [`Broadcaster`] can be cloned and shared across threads (via `Arc<Mutex<...>>`)
//! - [`Subscriber`] can be moved to another thread for consumption
//! - All operations are protected by a mutex
//!
//! The mutex is held only briefly during `send()`, `subscribe()`, and `close()`,
//! so contention is minimal in practice.
//!
//! # Architecture
//!
//! ```text
//!                           ┌─────────────────┐
//!                           │   Broadcaster   │
//!                           │                 │
//!                           │ ┌─────────────┐ │
//!    send(event) ──────────►│ │   Inner     │ │
//!                           │ │             │ │
//!                           │ │ subscribers │ │
//!                           │ │   Vec<Tx>   │ │
//!                           │ │             │ │
//!                           │ │  closed: T/F│ │
//!                           │ └─────────────┘ │
//!                           └────────┬────────┘
//!                                    │
//!              ┌─────────────────────┼─────────────────────┐
//!              │                     │                     │
//!              ▼                     ▼                     ▼
//!     ┌────────────────┐    ┌────────────────┐    ┌────────────────┐
//!     │  Subscriber 1  │    │  Subscriber 2  │    │  Subscriber 3  │
//!     │                │    │                │    │                │
//!     │   ┌────────┐   │    │   ┌────────┐   │    │   ┌────────┐   │
//!     │   │ Buffer │   │    │   │ Buffer │   │    │   │ Buffer │   │
//!     │   │ (Rx)   │   │    │   │ (Rx)   │   │    │   │ (Rx)   │   │
//!     │   └────────┘   │    │   └────────┘   │    │   └────────┘   │
//!     └────────────────┘    └────────────────┘    └────────────────┘
//!              │                     │                     │
//!              ▼                     ▼                     ▼
//!         Consumer 1            Consumer 2            Consumer 3
//!         (fast)                (slow)                (medium)
//! ```
//!
//! # Example Usage
//!
//! ```ignore
//! use broadcaster::{Broadcaster, Subscriber};
//!
//! // Create a broadcaster
//! let broadcaster: Broadcaster<String> = Broadcaster::new();
//!
//! // Create subscribers before sending (to receive all events)
//! let sub1 = broadcaster.subscribe();
//! let sub2 = broadcaster.subscribe();
//!
//! // Send events - this never blocks
//! broadcaster.send("event1".to_string());
//! broadcaster.send("event2".to_string());
//!
//! // Late subscriber - will only receive future events
//! let late_sub = broadcaster.subscribe();
//!
//! broadcaster.send("event3".to_string());
//!
//! // sub1 and sub2 receive: event1, event2, event3
//! // late_sub receives: event3 only (missed event1 and event2)
//!
//! // Close when done - all subscribers see disconnect
//! broadcaster.close();
//!
//! // Subscribers now return Err on recv()
//! assert!(sub1.recv().is_err());
//! ```
//!
//! # Performance Characteristics
//!
//! | Operation | Complexity | Blocking |
//! |-----------|------------|----------|
//! | `send()`  | O(n) where n = subscribers | Never |
//! | `subscribe()` | O(1) | Never |
//! | `close()` | O(n) where n = subscribers | Never |
//! | `Subscriber::recv()` | O(1) | Yes, until event or disconnect |
//! | `Subscriber::try_recv()` | O(1) | Never |
//!
//! # Memory Considerations
//!
//! - Each event is cloned once per subscriber during `send()`
//! - Subscribers that don't consume events will accumulate them in memory
//! - Dead subscribers are cleaned up lazily on the next `send()`
//! - The `close()` method immediately frees all sender resources

use std::sync::mpsc::{self, RecvError, TryRecvError};
use std::sync::{Arc, Mutex};

/// A handle for receiving events from a [`Broadcaster`].
///
/// Each subscriber has its own independent buffer and receives events
/// independently of other subscribers. Subscribers can be moved to other
/// threads for concurrent processing.
///
/// # Lifecycle
///
/// A subscriber remains active until either:
/// 1. The broadcaster is dropped (all clones)
/// 2. The broadcaster's [`close()`](Broadcaster::close) method is called
/// 3. The subscriber is dropped (lazy cleanup on next send)
///
/// When the broadcaster closes or is dropped, `recv()` will return `Err(RecvError)`
/// and `try_recv()` will return `Err(TryRecvError::Disconnected)`.
///
/// # Buffering
///
/// Events are buffered in an unbounded queue. If a subscriber doesn't consume
/// events, they accumulate in memory. This is intentional to avoid blocking
/// the producer.
#[derive(Debug)]
pub struct Subscriber<T> {
    recv: mpsc::Receiver<T>,
}

impl<T> Subscriber<T> {
    /// Creates a new subscriber wrapping the given receiver.
    pub(crate) fn new(recv: mpsc::Receiver<T>) -> Self {
        Self { recv }
    }

    /// Blocking receive - waits until an event is available or the stream closes.
    ///
    /// # Returns
    ///
    /// - `Ok(event)` - An event was received
    /// - `Err(RecvError)` - The broadcaster was closed or dropped
    ///
    /// # Blocking
    ///
    /// This method blocks the current thread until an event is available.
    /// Use [`try_recv()`](Self::try_recv) for non-blocking receives.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Typical consumption pattern
    /// loop {
    ///     match subscriber.recv() {
    ///         Ok(event) => process(event),
    ///         Err(_) => break, // Stream closed
    ///     }
    /// }
    /// ```
    pub fn recv(&self) -> Result<T, RecvError> {
        self.recv.recv()
    }

    /// Non-blocking receive - returns immediately with the result.
    ///
    /// # Returns
    ///
    /// - `Ok(event)` - An event was available and returned
    /// - `Err(TryRecvError::Empty)` - No events available, but stream is still open
    /// - `Err(TryRecvError::Disconnected)` - The broadcaster was closed or dropped
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Poll for events without blocking
    /// match subscriber.try_recv() {
    ///     Ok(event) => println!("Got event: {:?}", event),
    ///     Err(TryRecvError::Empty) => println!("No events yet"),
    ///     Err(TryRecvError::Disconnected) => println!("Stream closed"),
    /// }
    /// ```
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.recv.try_recv()
    }

    /// Returns true if the stream is closed and all buffered events have been consumed.
    ///
    /// **Note**: This method consumes one event from the buffer to check the state.
    /// If the stream is still open, that event is lost. Use this only when you
    /// need to check if iteration should stop.
    ///
    /// # Returns
    ///
    /// - `true` - The broadcaster is closed/dropped AND the buffer is empty
    /// - `false` - Either the stream is still open, or there are buffered events
    ///
    /// # Warning
    ///
    /// This is a destructive check - it may consume an event. Prefer checking
    /// the result of `recv()` or `try_recv()` instead.
    pub fn is_closed(&self) -> bool {
        matches!(self.recv.try_recv(), Err(TryRecvError::Disconnected))
    }
}

/// Internal state shared between the broadcaster and its clones.
///
/// Protected by a mutex to ensure thread-safe access.
#[derive(Debug)]
struct BroadcasterInner<T> {
    /// Active subscriber senders. Each sender corresponds to one subscriber's buffer.
    /// Senders are removed when:
    /// - The corresponding receiver is dropped (detected on next send)
    /// - The broadcaster is closed
    subscribers: Vec<mpsc::Sender<T>>,

    /// Whether the broadcaster has been explicitly closed.
    /// When true:
    /// - New subscribers receive immediately-disconnected channels
    /// - The subscribers vector is empty
    closed: bool,
}

/// A thread-safe broadcaster that fans out events to multiple subscribers.
///
/// # Overview
///
/// The broadcaster implements a publish-subscribe pattern where:
/// - One or more producers send events via [`send()`](Self::send)
/// - Zero or more subscribers receive events independently
/// - Each subscriber has its own buffer and processes events at its own pace
///
/// # Cloning
///
/// Cloning a broadcaster creates a new handle to the **same** underlying
/// broadcaster. All clones share the same subscriber list and state.
/// This allows the broadcaster to be shared across threads.
///
/// # Thread Safety
///
/// All methods are thread-safe and can be called concurrently from multiple threads.
///
/// # Example
///
/// ```ignore
/// let broadcaster: Broadcaster<i32> = Broadcaster::new();
///
/// // Clone for use in another thread
/// let broadcaster_clone = broadcaster.clone();
/// std::thread::spawn(move || {
///     for i in 0..100 {
///         broadcaster_clone.send(i);
///     }
///     broadcaster_clone.close();
/// });
///
/// // Subscribe and consume
/// let sub = broadcaster.subscribe();
/// for event in std::iter::from_fn(|| sub.recv().ok()) {
///     println!("Received: {}", event);
/// }
/// ```
#[derive(Debug)]
pub struct Broadcaster<T> {
    inner: Arc<Mutex<BroadcasterInner<T>>>,
}

impl<T: Clone> Broadcaster<T> {
    /// Creates a new broadcaster with no subscribers.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let broadcaster: Broadcaster<String> = Broadcaster::new();
    /// ```
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BroadcasterInner {
                subscribers: Vec::new(),
                closed: false,
            })),
        }
    }

    /// Creates a new subscriber that will receive future events.
    ///
    /// # Late Subscription
    ///
    /// Subscribers only receive events sent **after** they subscribe.
    /// Events sent before subscription are not replayed by the broadcaster.
    ///
    /// **Note**: If you need late subscribers to receive all events, implement
    /// buffering at a higher level (e.g., in `BuildEventStream`).
    ///
    /// # Closed Broadcaster
    ///
    /// If the broadcaster has been closed (via [`close()`](Self::close)),
    /// the returned subscriber will be immediately disconnected. Calling
    /// `recv()` on it will return `Err(RecvError)`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let broadcaster: Broadcaster<i32> = Broadcaster::new();
    ///
    /// // This subscriber receives events 1, 2, 3
    /// let early_sub = broadcaster.subscribe();
    ///
    /// broadcaster.send(1);
    /// broadcaster.send(2);
    ///
    /// // Late subscriber only receives event 3 (misses 1 and 2)
    /// let late_sub = broadcaster.subscribe();
    ///
    /// broadcaster.send(3);
    /// ```
    pub fn subscribe(&self) -> Subscriber<T> {
        let mut inner = self.inner.lock().unwrap();

        // If closed, return an immediately-disconnected channel.
        // This ensures subscribers created after close() don't block forever
        // waiting for events that will never come.
        if inner.closed {
            let (tx, rx) = mpsc::channel();
            drop(tx); // Drop sender immediately to disconnect the receiver
            return Subscriber::new(rx);
        }

        let (tx, rx) = mpsc::channel();
        inner.subscribers.push(tx);
        Subscriber::new(rx)
    }

    /// Sends an event to all current subscribers.
    ///
    /// # Non-blocking Guarantee
    ///
    /// This method **never blocks**, regardless of:
    /// - How many subscribers exist
    /// - How full subscriber buffers are
    /// - Whether subscribers are consuming events
    ///
    /// # Dead Subscriber Cleanup
    ///
    /// Subscribers whose receivers have been dropped are automatically
    /// removed during this call. This is detected by a failed send
    /// (which returns `Err` when the receiver is gone).
    ///
    /// # No Subscribers
    ///
    /// If there are no subscribers, this method does nothing and returns
    /// immediately. Events sent with no subscribers are silently discarded.
    ///
    /// # Closed Broadcaster
    ///
    /// If the broadcaster has been closed, this method does nothing.
    /// The event is silently discarded (there are no subscribers to receive it).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let broadcaster: Broadcaster<String> = Broadcaster::new();
    /// let sub = broadcaster.subscribe();
    ///
    /// // Send never blocks, even with slow consumers
    /// for i in 0..10_000 {
    ///     broadcaster.send(format!("event {}", i));
    /// }
    /// ```
    pub fn send(&self, event: T) {
        let mut inner = self.inner.lock().unwrap();

        // Use retain to both send and clean up dead subscribers in one pass.
        // - send() returns Ok if the receiver is still alive
        // - send() returns Err if the receiver has been dropped
        // We keep only the subscribers where send succeeded.
        inner
            .subscribers
            .retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Closes the broadcaster, disconnecting all subscribers.
    ///
    /// # Effects
    ///
    /// 1. Sets the `closed` flag to `true`
    /// 2. Drops all sender handles, causing all receivers to see disconnect
    /// 3. Future calls to [`subscribe()`](Self::subscribe) return immediately-disconnected channels
    /// 4. Future calls to [`send()`](Self::send) are no-ops
    ///
    /// # When to Use
    ///
    /// Call `close()` when the producer is done sending events. This ensures
    /// all subscribers see the end of the stream and can exit their receive loops.
    ///
    /// # Idempotent
    ///
    /// Calling `close()` multiple times is safe and has no additional effect
    /// after the first call.
    ///
    /// # Buffered Events
    ///
    /// Events that were already sent and buffered in subscriber channels
    /// remain available. Subscribers can still receive buffered events after
    /// `close()` is called - they'll see disconnect only after draining the buffer.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let broadcaster: Broadcaster<i32> = Broadcaster::new();
    /// let sub = broadcaster.subscribe();
    ///
    /// broadcaster.send(1);
    /// broadcaster.send(2);
    /// broadcaster.close();
    ///
    /// // Subscriber can still receive buffered events
    /// assert_eq!(sub.recv().unwrap(), 1);
    /// assert_eq!(sub.recv().unwrap(), 2);
    ///
    /// // Then sees disconnect
    /// assert!(sub.recv().is_err());
    /// ```
    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        inner.subscribers.clear(); // Drops all senders, disconnecting receivers
    }
}

impl<T> Clone for Broadcaster<T> {
    /// Creates a new handle to the same broadcaster.
    ///
    /// The clone shares the same subscriber list and state as the original.
    /// This is useful for sharing the broadcaster across threads.
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Clone> Default for Broadcaster<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn test_single_subscriber_receives_events() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let subscriber = broadcaster.subscribe();

        broadcaster.send(1);
        broadcaster.send(2);
        broadcaster.send(3);

        assert_eq!(subscriber.recv().unwrap(), 1);
        assert_eq!(subscriber.recv().unwrap(), 2);
        assert_eq!(subscriber.recv().unwrap(), 3);
    }

    #[test]
    fn test_multiple_subscribers_receive_all_events() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let sub1 = broadcaster.subscribe();
        let sub2 = broadcaster.subscribe();
        let sub3 = broadcaster.subscribe();

        for i in 0..100 {
            broadcaster.send(i);
        }

        // Each subscriber should receive all 100 events
        for i in 0..100 {
            assert_eq!(sub1.recv().unwrap(), i);
            assert_eq!(sub2.recv().unwrap(), i);
            assert_eq!(sub3.recv().unwrap(), i);
        }
    }

    #[test]
    fn test_slow_subscriber_does_not_block_fast_subscriber() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let fast_sub = broadcaster.subscribe();
        let _slow_sub = broadcaster.subscribe(); // Never read from this one

        let event_count = 10_000;

        // Send many events - should not block even though slow_sub isn't reading
        let start = Instant::now();
        for i in 0..event_count {
            broadcaster.send(i);
        }
        let send_duration = start.elapsed();

        // Sending should be fast (non-blocking) - less than 1 second for 10k events
        assert!(
            send_duration < Duration::from_secs(1),
            "Sending took too long: {:?}",
            send_duration
        );

        // Fast subscriber should receive all events
        for i in 0..event_count {
            assert_eq!(fast_sub.recv().unwrap(), i);
        }
    }

    #[test]
    fn test_dropped_subscriber_is_cleaned_up() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Create and immediately drop a subscriber
        {
            let _sub = broadcaster.subscribe();
        }

        // Create another subscriber
        let sub = broadcaster.subscribe();

        // Sending should work without issues
        broadcaster.send(42);
        assert_eq!(sub.recv().unwrap(), 42);

        // Verify the dropped subscriber was cleaned up
        let inner = broadcaster.inner.lock().unwrap();
        assert_eq!(inner.subscribers.len(), 1);
    }

    #[test]
    fn test_zero_subscribers_does_not_block() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Send events with no subscribers - should not block or panic
        let start = Instant::now();
        for i in 0..1000 {
            broadcaster.send(i);
        }
        let duration = start.elapsed();

        assert!(
            duration < Duration::from_millis(100),
            "Sending with no subscribers took too long: {:?}",
            duration
        );
    }

    #[test]
    fn test_subscribe_after_events_misses_early_events() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Send some events before subscribing
        broadcaster.send(1);
        broadcaster.send(2);
        broadcaster.send(3);

        // Subscribe after events were sent
        let sub = broadcaster.subscribe();

        // Send more events after subscribing
        broadcaster.send(4);
        broadcaster.send(5);

        // Subscriber should only receive events sent after subscribing
        assert_eq!(sub.recv().unwrap(), 4);
        assert_eq!(sub.recv().unwrap(), 5);

        // No more events available
        assert!(sub.try_recv().is_err());
    }

    #[test]
    fn test_is_closed_reports_correctly() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let sub = broadcaster.subscribe();

        // Not closed yet
        assert!(!sub.is_closed());

        // Send an event and consume it
        broadcaster.send(1);
        assert_eq!(sub.recv().unwrap(), 1);

        // Still not closed (broadcaster exists)
        assert!(!sub.is_closed());

        // Drop the broadcaster
        drop(broadcaster);

        // Now it should be closed
        assert!(sub.is_closed());
    }

    #[test]
    fn test_concurrent_send_and_receive() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let sub = broadcaster.subscribe();
        let broadcaster_clone = broadcaster.clone();

        let received = Arc::new(AtomicUsize::new(0));
        let received_clone = received.clone();

        let event_count = 1000;

        // Spawn receiver thread
        let receiver_handle = thread::spawn(move || {
            for _ in 0..event_count {
                sub.recv().unwrap();
                received_clone.fetch_add(1, Ordering::SeqCst);
            }
        });

        // Send from main thread
        for i in 0..event_count {
            broadcaster_clone.send(i as i32);
        }

        receiver_handle.join().unwrap();
        assert_eq!(received.load(Ordering::SeqCst), event_count);
    }

    #[test]
    fn test_multiple_concurrent_subscribers() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let event_count = 1000;
        let subscriber_count = 5;

        let subscribers: Vec<_> = (0..subscriber_count)
            .map(|_| broadcaster.subscribe())
            .collect();

        let received_counts: Vec<_> = (0..subscriber_count)
            .map(|_| Arc::new(AtomicUsize::new(0)))
            .collect();

        // Spawn receiver threads
        let handles: Vec<_> = subscribers
            .into_iter()
            .zip(received_counts.iter().cloned())
            .map(|(sub, count)| {
                thread::spawn(move || loop {
                    match sub.recv() {
                        Ok(_) => {
                            count.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(_) => break,
                    }
                })
            })
            .collect();

        // Send events
        for i in 0..event_count {
            broadcaster.send(i as i32);
        }

        // Drop broadcaster to close channels
        drop(broadcaster);

        // Wait for all receivers
        for handle in handles {
            handle.join().unwrap();
        }

        // Each subscriber should have received all events
        for (i, count) in received_counts.iter().enumerate() {
            assert_eq!(
                count.load(Ordering::SeqCst),
                event_count,
                "Subscriber {} received wrong count",
                i
            );
        }
    }

    #[test]
    fn test_close_disconnects_all_subscribers() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let sub1 = broadcaster.subscribe();
        let sub2 = broadcaster.subscribe();
        let sub3 = broadcaster.subscribe();

        // Send some events
        broadcaster.send(1);
        broadcaster.send(2);

        // Consume events from sub1
        assert_eq!(sub1.recv().unwrap(), 1);
        assert_eq!(sub1.recv().unwrap(), 2);

        // Close the broadcaster
        broadcaster.close();

        // sub1 should be disconnected immediately (no buffered events)
        assert!(sub1.recv().is_err());

        // sub2 and sub3 still have buffered events (1 and 2), drain them
        // then they should be disconnected
        assert_eq!(sub2.recv().unwrap(), 1);
        assert_eq!(sub2.recv().unwrap(), 2);
        assert!(sub2.recv().is_err());

        assert_eq!(sub3.recv().unwrap(), 1);
        assert_eq!(sub3.recv().unwrap(), 2);
        assert!(sub3.recv().is_err());

        // is_closed should return true for all
        assert!(sub1.is_closed());
        assert!(sub2.is_closed());
        assert!(sub3.is_closed());
    }

    #[test]
    fn test_subscribe_after_close_returns_disconnected_channel() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Close the broadcaster before subscribing
        broadcaster.close();

        // Subscribe after close
        let sub = broadcaster.subscribe();

        // Subscriber should immediately be disconnected
        assert!(sub.recv().is_err());
        assert!(sub.is_closed());
    }

    #[test]
    fn test_send_after_close_is_noop() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let sub = broadcaster.subscribe();

        // Send one event before close
        broadcaster.send(1);

        // Close the broadcaster
        broadcaster.close();

        // Send after close (should be no-op, not panic)
        broadcaster.send(2);
        broadcaster.send(3);

        // Subscriber should still be disconnected (not receive 2 or 3)
        // Note: The event 1 was sent before close, but close() clears subscribers
        // so the channel was disconnected and event 1 may or may not be received
        // depending on timing. The key is that recv() should eventually return Err.
        loop {
            match sub.try_recv() {
                Ok(_) => continue, // consume any buffered events
                Err(TryRecvError::Empty) => {
                    // Channel still open but empty - this shouldn't happen after close
                    panic!("Channel should be disconnected after close");
                }
                Err(TryRecvError::Disconnected) => break, // expected
            }
        }
    }

    #[test]
    fn test_close_clears_subscribers_vector() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Subscribe multiple times
        let _sub1 = broadcaster.subscribe();
        let _sub2 = broadcaster.subscribe();
        let _sub3 = broadcaster.subscribe();

        // Verify there are 3 subscribers
        {
            let inner = broadcaster.inner.lock().unwrap();
            assert_eq!(inner.subscribers.len(), 3);
            assert!(!inner.closed);
        }

        // Close the broadcaster
        broadcaster.close();

        // Verify subscribers are cleared and closed flag is set
        {
            let inner = broadcaster.inner.lock().unwrap();
            assert_eq!(inner.subscribers.len(), 0);
            assert!(inner.closed);
        }
    }

    #[test]
    fn test_close_is_idempotent() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let sub = broadcaster.subscribe();

        // Close multiple times - should not panic
        broadcaster.close();
        broadcaster.close();
        broadcaster.close();

        // Subscriber should still be disconnected
        assert!(sub.recv().is_err());
    }

    #[test]
    fn test_clone_shares_state() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let broadcaster_clone = broadcaster.clone();

        let sub = broadcaster.subscribe();

        // Send from clone
        broadcaster_clone.send(42);

        // Receive from original's subscriber
        assert_eq!(sub.recv().unwrap(), 42);

        // Close from clone
        broadcaster_clone.close();

        // Original's subscriber sees disconnect
        assert!(sub.recv().is_err());
    }

    /// Regression test for the bug in c35c9d5b where dropping a cloned broadcaster
    /// without calling close() left subscribers hanging forever.
    ///
    /// The bug: BuildEventStream kept two Broadcaster clones:
    /// 1. `broadcaster` in the holder (for subscribe() calls)
    /// 2. `broadcaster_for_thread` in the BES thread (for sending events)
    ///
    /// When the BES thread finished, it just returned without calling close().
    /// Since both clones share the same `inner` Arc, dropping `broadcaster_for_thread`
    /// only decremented the refcount - it didn't drop the senders in `inner.subscribers`.
    /// The senders were only dropped when `join()` took the broadcaster from the holder.
    /// But by then, `wait()` was already blocked waiting for TracingEventStreamSink
    /// to finish, which was blocked on `recv()` that never saw disconnect.
    ///
    /// The fix: Always call `close()` before returning from the BES thread.
    /// `close()` explicitly clears all senders, notifying all receivers immediately.
    #[test]
    fn test_drop_without_close_does_not_disconnect_subscribers() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let broadcaster_clone = broadcaster.clone();

        let sub = broadcaster.subscribe();

        // Send an event
        broadcaster_clone.send(42);
        assert_eq!(sub.recv().unwrap(), 42);

        // Drop the clone WITHOUT calling close() - simulates BES thread returning
        // without closing the broadcaster (the bug in c35c9d5b)
        drop(broadcaster_clone);

        // Subscriber should NOT be disconnected yet! The original broadcaster
        // still holds a reference to `inner`, so senders are still alive.
        // This is a non-blocking check.
        assert!(
            !sub.is_closed(),
            "Subscriber should NOT be disconnected when only one clone is dropped"
        );

        // Now close the original broadcaster - this is what the fix does
        broadcaster.close();

        // NOW the subscriber should be disconnected
        assert!(
            sub.is_closed(),
            "Subscriber should be disconnected after close() is called"
        );
        assert!(sub.recv().is_err());
    }

    /// Test that demonstrates why explicit close() is necessary.
    /// Without close(), subscribers only disconnect when ALL broadcaster clones are dropped.
    #[test]
    fn test_subscribers_disconnect_only_when_all_clones_dropped() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let clone1 = broadcaster.clone();
        let clone2 = broadcaster.clone();

        let sub = broadcaster.subscribe();
        broadcaster.send(1);

        // Drop original - subscriber should still work (clone1 and clone2 exist)
        drop(broadcaster);
        // Verify subscriber still receives events
        assert_eq!(sub.recv().unwrap(), 1);

        // Send from clone1
        clone1.send(2);

        // Drop clone1 - subscriber should still work (clone2 exists)
        drop(clone1);
        assert_eq!(sub.recv().unwrap(), 2);

        // Send from clone2
        clone2.send(3);

        // Drop clone2 - now all clones are gone, subscriber should disconnect
        drop(clone2);

        // Can still drain buffered event
        assert_eq!(sub.recv().unwrap(), 3);

        // NOW disconnected (buffer empty and all senders dropped)
        assert!(sub.recv().is_err());
        assert!(sub.is_closed());
    }

    // =========================================================================
    // Tests for late subscription scenarios (simulating ctx.bazel.build()
    // followed by late .build_events() call)
    // =========================================================================

    /// Simulates: build starts, sends all events, then user calls build_events()
    /// Expected behavior: Late subscriber misses ALL events (broadcaster doesn't buffer)
    #[test]
    fn test_late_subscriber_after_all_events_misses_everything() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Simulate: Build starts and sends all events before user subscribes
        for i in 0..10 {
            broadcaster.send(i);
        }

        // Late subscription - after all events sent
        let late_sub = broadcaster.subscribe();

        // Close the stream (build finished)
        broadcaster.close();

        // Late subscriber should receive nothing - immediately disconnected
        assert!(
            late_sub.recv().is_err(),
            "Late subscriber should be disconnected immediately with no events"
        );
    }

    /// Simulates: build starts, sends some events, user calls build_events(), more events sent
    /// Expected behavior: Late subscriber only receives events sent after subscribing
    #[test]
    fn test_late_subscriber_mid_stream_misses_early_events() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Early events (before subscription)
        broadcaster.send(1);
        broadcaster.send(2);
        broadcaster.send(3);

        // Late subscription - mid-stream
        let late_sub = broadcaster.subscribe();

        // More events (after subscription)
        broadcaster.send(4);
        broadcaster.send(5);

        broadcaster.close();

        // Late subscriber should only receive events 4 and 5
        assert_eq!(late_sub.recv().unwrap(), 4);
        assert_eq!(late_sub.recv().unwrap(), 5);
        assert!(late_sub.recv().is_err());
    }

    /// Simulates: build starts in background thread, user delays calling build_events()
    /// Expected behavior: Events sent during the delay are lost to the late subscriber
    #[test]
    fn test_late_subscriber_with_concurrent_producer() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();
        let broadcaster_for_thread = broadcaster.clone();

        // Start producer thread that sends events immediately
        let producer = thread::spawn(move || {
            for i in 0..100 {
                broadcaster_for_thread.send(i);
            }
            broadcaster_for_thread.close();
        });

        // Simulate user delay before calling build_events()
        thread::sleep(Duration::from_millis(10));

        // Subscribe late
        let late_sub = broadcaster.subscribe();

        producer.join().unwrap();

        // Late subscriber likely misses most or all events
        // (depending on timing, might catch some tail events)
        let mut received = Vec::new();
        loop {
            match late_sub.try_recv() {
                Ok(event) => received.push(event),
                Err(TryRecvError::Empty) => {
                    // Shouldn't happen since producer closed
                    break;
                }
                Err(TryRecvError::Disconnected) => break,
            }
        }

        // We don't know exactly how many events were missed, but it should be
        // fewer than 100 (likely 0 since producer finished quickly)
        println!(
            "Late subscriber received {} of 100 events: {:?}",
            received.len(),
            received
        );

        // The key assertion: late subscribers don't get all events
        // This documents the broadcaster's behavior - buffering for late
        // subscribers should be implemented at a higher level (BuildEventStream)
        assert!(
            received.len() < 100,
            "Late subscriber should miss at least some events"
        );
    }

    /// Simulates: Early subscriber exists, late subscriber joins, both should work
    /// Expected behavior: Early subscriber gets all, late subscriber gets only later events
    #[test]
    fn test_early_and_late_subscribers_coexist() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Early subscriber - will receive all events
        let early_sub = broadcaster.subscribe();

        // Send first batch
        broadcaster.send(1);
        broadcaster.send(2);

        // Late subscriber - will miss first batch
        let late_sub = broadcaster.subscribe();

        // Send second batch
        broadcaster.send(3);
        broadcaster.send(4);

        broadcaster.close();

        // Early subscriber receives all 4 events
        assert_eq!(early_sub.recv().unwrap(), 1);
        assert_eq!(early_sub.recv().unwrap(), 2);
        assert_eq!(early_sub.recv().unwrap(), 3);
        assert_eq!(early_sub.recv().unwrap(), 4);
        assert!(early_sub.recv().is_err());

        // Late subscriber receives only events 3 and 4
        assert_eq!(late_sub.recv().unwrap(), 3);
        assert_eq!(late_sub.recv().unwrap(), 4);
        assert!(late_sub.recv().is_err());
    }

    /// Documents that subscribe() after close() returns a disconnected channel
    /// with no buffered events (even if events were sent before close)
    #[test]
    fn test_subscribe_after_close_gets_no_historical_events() {
        let broadcaster: Broadcaster<i32> = Broadcaster::new();

        // Send events
        broadcaster.send(1);
        broadcaster.send(2);
        broadcaster.send(3);

        // Close the broadcaster
        broadcaster.close();

        // Subscribe after close
        let post_close_sub = broadcaster.subscribe();

        // Should immediately be disconnected with no events
        assert!(
            post_close_sub.recv().is_err(),
            "Subscriber after close should get no events"
        );
    }
}
