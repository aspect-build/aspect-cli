use axl_proto::build_event_stream::BuildEvent;
use prost::Message;
use std::io::ErrorKind;
use std::sync::mpsc::{self, RecvError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::{env, io};
use std::{
    io::Read,
    path::PathBuf,
    thread::{self, JoinHandle},
};
use thiserror::Error;

use super::util::read_varint;

#[derive(Error, Debug)]
pub enum BuildEventStreamError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
}

/// A subscriber to the build event stream.
/// Each subscriber has its own independent buffer and receives all events.
#[derive(Debug)]
pub struct Subscriber<T> {
    recv: mpsc::Receiver<T>,
}

impl<T> Subscriber<T> {
    /// Blocking receive - waits until an event is available or the stream closes.
    pub fn recv(&self) -> Result<T, RecvError> {
        self.recv.recv()
    }

    /// Non-blocking receive - returns immediately.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.recv.try_recv()
    }

    /// Returns true if the stream is closed and all events have been received.
    pub fn is_closed(&self) -> bool {
        // Try to peek - if disconnected and empty, we're done
        match self.recv.try_recv() {
            Err(TryRecvError::Disconnected) => true,
            _ => false,
        }
    }
}

/// Internal state shared between the producer and BuildEventStream.
#[derive(Debug)]
struct BroadcasterInner<T> {
    subscribers: Vec<mpsc::Sender<T>>,
}

/// A broadcaster that fans out events to multiple subscribers.
/// Each subscriber has its own independent channel, so slow subscribers
/// don't block other subscribers or the producer.
#[derive(Debug)]
struct Broadcaster<T> {
    inner: Arc<Mutex<BroadcasterInner<T>>>,
}

impl<T: Clone> Broadcaster<T> {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BroadcasterInner {
                subscribers: Vec::new(),
            })),
        }
    }

    /// Add a new subscriber. Returns a Subscriber that will receive all
    /// future events (events sent before subscribing are not received).
    fn subscribe(&self) -> Subscriber<T> {
        let (tx, rx) = mpsc::channel();
        self.inner.lock().unwrap().subscribers.push(tx);
        Subscriber { recv: rx }
    }

    /// Send an event to all subscribers. This never blocks - if a subscriber's
    /// channel is full or closed, that subscriber is removed.
    fn send(&self, event: T) {
        let mut inner = self.inner.lock().unwrap();
        // Retain only subscribers that successfully receive the event
        inner
            .subscribers
            .retain(|tx| tx.send(event.clone()).is_ok());
    }
}

impl<T> Clone for Broadcaster<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Debug)]
pub struct BuildEventStream {
    handle: JoinHandle<Result<(), BuildEventStreamError>>,
    broadcaster: Arc<Mutex<Option<Broadcaster<BuildEvent>>>>,
}

impl BuildEventStream {
    pub fn spawn_with_pipe(pid: u32) -> io::Result<(PathBuf, Self)> {
        let out = env::temp_dir().join(format!("build-event-out-{}.bin", uuid::Uuid::new_v4()));
        let stream = Self::spawn(out.clone(), pid)?;
        Ok((out, stream))
    }

    pub fn spawn(path: PathBuf, pid: u32) -> io::Result<Self> {
        let broadcaster = Broadcaster::new();
        let broadcaster_for_thread = broadcaster.clone();
        let broadcaster_holder = Arc::new(Mutex::new(Some(broadcaster)));

        let handle = thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024 * 5);
            buf.resize(10, 0);
            let mut out_raw =
                galvanize::Pipe::new(path.clone(), galvanize::RetryPolicy::IfOpenForPid(pid))?;

            let read_event = |buf: &mut Vec<u8>,
                              out_raw: &mut galvanize::Pipe|
             -> Result<BuildEvent, BuildEventStreamError> {
                let size = read_varint(out_raw)?;
                if size > buf.len() {
                    buf.resize(size, 0);
                }
                out_raw.read_exact(&mut buf[0..size])?;
                let event = BuildEvent::decode(&buf[0..size])?;
                Ok(event)
            };

            loop {
                match read_event(&mut buf, &mut out_raw) {
                    Ok(event) => {
                        let last_message = event.last_message;

                        // Fan-out to all subscribers (non-blocking)
                        broadcaster_for_thread.send(event);

                        if last_message {
                            return Ok(());
                        }
                    }
                    Err(BuildEventStreamError::IO(err)) if err.kind() == ErrorKind::BrokenPipe => {
                        return Ok(());
                    }
                    Err(err) => return Err(err),
                }
            }
        });

        Ok(Self {
            handle,
            broadcaster: broadcaster_holder,
        })
    }

    /// Subscribe to the build event stream. Each subscriber receives all events
    /// independently and has its own buffer. Subscribers don't block each other.
    pub fn subscribe(&self) -> Subscriber<BuildEvent> {
        self.broadcaster
            .lock()
            .unwrap()
            .as_ref()
            .expect("cannot subscribe after join")
            .subscribe()
    }

    pub fn join(self) -> Result<(), BuildEventStreamError> {
        // Take the broadcaster to drop all senders when done
        let _ = self.broadcaster.lock().unwrap().take();
        self.handle.join().expect("join error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
}
