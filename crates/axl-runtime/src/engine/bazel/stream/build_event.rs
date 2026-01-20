use axl_proto::build_event_stream::BuildEvent;
use prost::Message;
use std::io::ErrorKind;
use std::sync::mpsc::RecvError;
use std::sync::{Arc, Mutex};
use std::{env, io};
use std::{
    io::Read,
    path::PathBuf,
    thread::{self, JoinHandle},
};
use thiserror::Error;

use super::broadcaster::{Broadcaster, Subscriber};
use super::util::read_varint;

#[derive(Error, Debug)]
pub enum BuildEventStreamError {
    #[error("io error: {0}")]
    IO(#[from] std::io::Error),
    #[error("prost decode error: {0}")]
    ProstDecode(#[from] prost::DecodeError),
}

/// A subscriber that replays historical events before returning real-time events.
///
/// This ensures that late subscribers (those who call `subscribe()` after some events
/// have already been sent) still receive all events from the beginning of the stream.
pub struct ReplaySubscriber {
    /// Historical events to replay first (in order). Using IntoIter gives ownership
    /// of each event without cloning.
    history: std::vec::IntoIter<BuildEvent>,
    /// Real-time subscriber for events after the history snapshot was taken.
    realtime: Subscriber<BuildEvent>,
}

impl std::fmt::Debug for ReplaySubscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplaySubscriber")
            .field("history_remaining", &self.history.len())
            .field("realtime", &self.realtime)
            .finish()
    }
}

impl ReplaySubscriber {
    fn new(history: Vec<BuildEvent>, realtime: Subscriber<BuildEvent>) -> Self {
        Self {
            history: history.into_iter(),
            realtime,
        }
    }

    /// Blocking receive - first returns historical events, then real-time events.
    /// Takes ownership of each event without cloning.
    pub fn recv(&mut self) -> Result<BuildEvent, RecvError> {
        // First, drain the history buffer (takes ownership, no clone)
        if let Some(event) = self.history.next() {
            return Ok(event);
        }

        // Then, receive from the real-time stream
        self.realtime.recv()
    }

    /// Non-blocking receive - first returns historical events, then real-time events.
    /// Takes ownership of each event without cloning.
    pub fn try_recv(&mut self) -> Result<BuildEvent, std::sync::mpsc::TryRecvError> {
        // First, drain the history buffer (takes ownership, no clone)
        if let Some(event) = self.history.next() {
            return Ok(event);
        }

        // Then, try to receive from the real-time stream
        self.realtime.try_recv()
    }

    /// Returns true if the stream is closed and all events have been consumed.
    pub fn is_closed(&self) -> bool {
        // If we still have history to return, not closed yet
        if self.history.len() > 0 {
            return false;
        }

        // Otherwise, check the real-time stream
        self.realtime.is_closed()
    }
}

#[derive(Debug)]
pub struct BuildEventStream {
    /// Thread handle, stored in Option so we can take() it to join without consuming self.
    handle: Option<JoinHandle<Result<(), BuildEventStreamError>>>,
    broadcaster: Arc<Mutex<Option<Broadcaster<BuildEvent>>>>,
    /// History of all events received. Used for replaying to late subscribers.
    history: Arc<Mutex<Vec<BuildEvent>>>,
    /// Whether join() has been called.
    joined: bool,
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
        let history: Arc<Mutex<Vec<BuildEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let history_for_thread = history.clone();

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

                        // Store event in history for late subscribers
                        history_for_thread.lock().unwrap().push(event.clone());

                        // Fan-out to all subscribers (non-blocking)
                        broadcaster_for_thread.send(event);

                        if last_message {
                            broadcaster_for_thread.close();
                            return Ok(());
                        }
                    }
                    Err(BuildEventStreamError::IO(err)) if err.kind() == ErrorKind::BrokenPipe => {
                        broadcaster_for_thread.close();
                        return Ok(());
                    }
                    Err(err) => {
                        broadcaster_for_thread.close();
                        return Err(err);
                    }
                }
            }
        });

        Ok(Self {
            handle: Some(handle),
            broadcaster: broadcaster_holder,
            history,
            joined: false,
        })
    }

    /// Subscribe to the build event stream with history replay.
    ///
    /// Each subscriber receives all events independently and has its own buffer.
    /// Subscribers don't block each other.
    ///
    /// # Late Subscription Support
    ///
    /// Late subscribers (those who call `subscribe()` after some events have already
    /// been sent) will still receive ALL events from the beginning of the stream.
    /// This is achieved by:
    ///
    /// 1. Taking a snapshot of the history buffer at subscription time
    /// 2. Creating a real-time subscriber for future events
    /// 3. Returning a `ReplaySubscriber` that first yields history, then real-time events
    ///
    /// This ensures that `build_events()` always returns all events regardless of
    /// when it's called relative to the build progress.
    pub fn subscribe(&self) -> ReplaySubscriber {
        // Take a snapshot of the current history.
        // Note: We hold the history lock while also getting a broadcaster subscription
        // to ensure we don't miss any events between the snapshot and subscription.
        let history_snapshot = self.history.lock().unwrap().clone();

        // Get a real-time subscriber for events after this point.
        // If the broadcaster was already taken (after join), create a closed subscriber.
        let realtime = self
            .broadcaster
            .lock()
            .unwrap()
            .as_ref()
            .map(|b| b.subscribe())
            .unwrap_or_else(|| {
                // Create an immediately-disconnected subscriber
                let (tx, rx) = std::sync::mpsc::channel();
                drop(tx);
                Subscriber::new(rx)
            });

        ReplaySubscriber::new(history_snapshot, realtime)
    }

    /// Subscribe to the build event stream without history replay.
    ///
    /// This is for internal use by sinks that subscribe at stream creation time
    /// and don't need history replay. Use `subscribe()` for user-facing APIs
    /// where late subscription support is needed.
    pub fn subscribe_realtime(&self) -> Subscriber<BuildEvent> {
        self.broadcaster
            .lock()
            .unwrap()
            .as_ref()
            .expect("cannot subscribe after join")
            .subscribe()
    }

    /// Wait for the BES thread to complete.
    ///
    /// After calling this, subscribers can still be created via `subscribe()` and
    /// will receive all historical events. This is safe to call multiple times.
    pub fn join(&mut self) -> Result<(), BuildEventStreamError> {
        if self.joined {
            return Ok(());
        }
        self.joined = true;

        // Broadcaster is already closed by the thread, but take() for cleanup
        let _ = self.broadcaster.lock().unwrap().take();

        if let Some(handle) = self.handle.take() {
            handle.join().expect("join error")?;
        }
        Ok(())
    }
}
