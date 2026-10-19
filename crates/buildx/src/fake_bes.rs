//! An in-memory Build Event Service for tests and examples: a test fixture, not a
//! product. It serves `PublishBuildEvent` (`PublishLifecycleEvent` and
//! `PublishBuildToolEventStream`), acks every ordered event of a stream, and
//! records what it received.

use axl_proto::build_event_stream::BuildEvent as BepEvent;
use axl_proto::google::devtools::build::v1::{
    PublishBuildToolEventStreamRequest, PublishBuildToolEventStreamResponse,
    PublishLifecycleEventRequest, publish_build_event_server,
};
use prost::Message;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status, Streaming};

/// What the server did, for the example scripts to show.
#[derive(Default)]
pub struct Counters {
    pub lifecycle_events: AtomicU64,
    pub streams: AtomicU64,
    pub stream_events: AtomicU64,
    pub acks: AtomicU64,
    pub refused: AtomicU64,
    pub dropped: AtomicU64,
}

impl Counters {
    pub fn json(&self) -> String {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        format!(
            "{{\"lifecycle_events\":{},\"streams\":{},\"stream_events\":{},\"acks\":{},\"refused\":{},\"dropped\":{}}}",
            g(&self.lifecycle_events),
            g(&self.streams),
            g(&self.stream_events),
            g(&self.acks),
            g(&self.refused),
            g(&self.dropped),
        )
    }
}

/// How the server treats the streams it is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    /// Every call fails with `UNAVAILABLE` before anything is read.
    Refuse,
    /// A stream is ended with `UNAVAILABLE` after it acked this many events.
    DropAfter(u64),
}

#[derive(Default)]
pub struct State {
    pub counters: Counters,
    lifecycle: Mutex<Vec<PublishLifecycleEventRequest>>,
    stream: Mutex<Vec<PublishBuildToolEventStreamRequest>>,
    headers: Mutex<Vec<(String, String)>>,
    mode: Mutex<Mode>,
}

#[derive(Clone, Default)]
pub struct Server {
    pub state: Arc<State>,
}

impl Server {
    pub fn new() -> Server {
        Server::default()
    }

    pub fn set_mode(&self, mode: Mode) {
        *self.state.mode.lock().unwrap() = mode;
    }

    /// The lifecycle events received, in order.
    pub fn lifecycle(&self) -> Vec<PublishLifecycleEventRequest> {
        self.state.lifecycle.lock().unwrap().clone()
    }

    /// The ordered stream requests received, in order of arrival.
    pub fn stream(&self) -> Vec<PublishBuildToolEventStreamRequest> {
        self.state.stream.lock().unwrap().clone()
    }

    /// The Build Event Protocol events of the stream, decoded.
    pub fn bep_events(&self) -> Vec<BepEvent> {
        self.stream()
            .iter()
            .filter_map(|r| {
                let any = match r
                    .ordered_build_event
                    .as_ref()?
                    .event
                    .as_ref()?
                    .event
                    .as_ref()?
                {
                    axl_proto::google::devtools::build::v1::build_event::Event::BazelEvent(a) => a,
                    _ => return None,
                };
                BepEvent::decode(any.value.as_slice()).ok()
            })
            .collect()
    }

    /// The BEP events as delimited bytes, what `--build_event_binary_file` writes.
    pub fn bep_file(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for e in self.bep_events() {
            e.encode_length_delimited(&mut out).expect("encode");
        }
        out
    }

    /// The ids of the invocations the server saw lifecycle events for.
    pub fn invocation_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .stream()
            .iter()
            .filter_map(|r| {
                r.ordered_build_event
                    .as_ref()?
                    .stream_id
                    .as_ref()
                    .map(|s| s.invocation_id.clone())
            })
            .collect();
        ids.dedup();
        ids
    }

    /// Whether any call carried the header `name` with `value`.
    pub fn saw_header(&self, name: &str, value: &str) -> bool {
        self.state
            .headers
            .lock()
            .unwrap()
            .iter()
            .any(|(n, v)| n == name && v == value)
    }

    fn note_headers(&self, metadata: &tonic::metadata::MetadataMap) {
        let mut seen = self.state.headers.lock().unwrap();
        for entry in metadata.iter() {
            if let tonic::metadata::KeyAndValueRef::Ascii(k, v) = entry
                && let Ok(v) = v.to_str()
            {
                seen.push((k.as_str().to_string(), v.to_string()));
            }
        }
    }

    /// Serves on the connections `incoming` yields until the future is dropped.
    pub async fn serve_incoming<I, IO, IE>(self, incoming: I) -> Result<(), tonic::transport::Error>
    where
        I: tokio_stream::Stream<Item = Result<IO, IE>> + Send + 'static,
        IO: tokio::io::AsyncRead
            + tokio::io::AsyncWrite
            + tonic::transport::server::Connected
            + Unpin
            + Send
            + 'static,
        IE: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        tonic::transport::Server::builder()
            .add_service(publish_build_event_server::PublishBuildEventServer::new(
                self,
            ))
            .serve_with_incoming(incoming)
            .await
    }

    /// Like `serve_incoming`, over TLS with the PEM certificate and key given.
    pub async fn serve_incoming_tls<I, IO, IE>(
        self,
        incoming: I,
        certificate: &[u8],
        key: &[u8],
    ) -> Result<(), tonic::transport::Error>
    where
        I: tokio_stream::Stream<Item = Result<IO, IE>> + Send + 'static,
        IO: tokio::io::AsyncRead
            + tokio::io::AsyncWrite
            + tonic::transport::server::Connected
            + Unpin
            + Send
            + 'static,
        IE: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let identity = tonic::transport::Identity::from_pem(certificate, key);
        tonic::transport::Server::builder()
            .tls_config(tonic::transport::ServerTlsConfig::new().identity(identity))?
            .add_service(publish_build_event_server::PublishBuildEventServer::new(
                self,
            ))
            .serve_with_incoming(incoming)
            .await
    }
}

#[tonic::async_trait]
impl publish_build_event_server::PublishBuildEvent for Server {
    async fn publish_lifecycle_event(
        &self,
        request: Request<PublishLifecycleEventRequest>,
    ) -> Result<Response<()>, Status> {
        self.note_headers(request.metadata());
        if *self.state.mode.lock().unwrap() == Mode::Refuse {
            self.state.counters.refused.fetch_add(1, Ordering::Relaxed);
            return Err(Status::unavailable("the fake BES refuses every call"));
        }
        self.state
            .counters
            .lifecycle_events
            .fetch_add(1, Ordering::Relaxed);
        self.state
            .lifecycle
            .lock()
            .unwrap()
            .push(request.into_inner());
        Ok(Response::new(()))
    }

    type PublishBuildToolEventStreamStream =
        tokio_stream::wrappers::ReceiverStream<Result<PublishBuildToolEventStreamResponse, Status>>;

    async fn publish_build_tool_event_stream(
        &self,
        request: Request<Streaming<PublishBuildToolEventStreamRequest>>,
    ) -> Result<Response<Self::PublishBuildToolEventStreamStream>, Status> {
        self.note_headers(request.metadata());
        let mode = *self.state.mode.lock().unwrap();
        if mode == Mode::Refuse {
            self.state.counters.refused.fetch_add(1, Ordering::Relaxed);
            return Err(Status::unavailable("the fake BES refuses every call"));
        }
        self.state.counters.streams.fetch_add(1, Ordering::Relaxed);
        let mut incoming = request.into_inner();
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let state = self.state.clone();
        tokio::spawn(async move {
            let mut acked = 0u64;
            while let Ok(Some(message)) = incoming.message().await {
                if let Mode::DropAfter(n) = mode
                    && acked >= n
                {
                    state.counters.dropped.fetch_add(1, Ordering::Relaxed);
                    let _ = tx
                        .send(Err(Status::unavailable("the fake BES dropped the stream")))
                        .await;
                    return;
                }
                let Some(ordered) = message.ordered_build_event.clone() else {
                    continue;
                };
                state.counters.stream_events.fetch_add(1, Ordering::Relaxed);
                state.stream.lock().unwrap().push(message);
                let ack = PublishBuildToolEventStreamResponse {
                    stream_id: ordered.stream_id,
                    sequence_number: ordered.sequence_number,
                };
                if tx.send(Ok(ack)).await.is_err() {
                    return;
                }
                acked += 1;
                state.counters.acks.fetch_add(1, Ordering::Relaxed);
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}
