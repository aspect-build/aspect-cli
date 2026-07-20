use std::collections::HashMap;
use std::time::Duration;

use axl_proto::google::devtools::build::v1::{
    PublishBuildToolEventStreamRequest, PublishBuildToolEventStreamResponse,
    PublishLifecycleEventRequest, publish_build_event_client::PublishBuildEventClient,
};
use futures::Stream;
use http::uri::InvalidUri;
use tonic::{
    Request, Response, Streaming,
    service::interceptor::InterceptedService,
    transport::{Channel, ClientTlsConfig},
};

use crate::auth::AuthInterceptor;

pub struct Client {
    inner: PublishBuildEventClient<InterceptedService<Channel, AuthInterceptor>>,
}

/// HTTP/2 PING cadence (also used as the TCP keepalive interval). Without
/// keepalives, a peer that vanishes without a FIN/RST leaves reads pending
/// forever and writes pending until the OS TCP retransmit limit (~15+
/// minutes). 30s matches the `--grpc_keepalive_time=30s` commonly configured
/// for Bazel's own gRPC connections.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(30);
/// How long to wait for a PING ack before declaring the connection dead.
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(15);
/// Bound for TCP connection establishment when the lazy channel first dials.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error(transparent)]
    InvalidEndpoint(#[from] InvalidUri),
    #[error(transparent)]
    Transport(#[from] tonic::transport::Error),
    #[error(transparent)]
    Status(#[from] tonic::Status),
}

impl Client {
    pub async fn new(
        endpoint: String,
        headers: HashMap<String, String>,
    ) -> Result<Self, ClientError> {
        let channel = Channel::from_shared(endpoint)?
            .user_agent("AXL")?
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_keepalive(Some(KEEP_ALIVE_INTERVAL))
            .http2_keep_alive_interval(KEEP_ALIVE_INTERVAL)
            .keep_alive_timeout(KEEP_ALIVE_TIMEOUT)
            .keep_alive_while_idle(true)
            .tls_config(
                ClientTlsConfig::new()
                    .with_native_roots()
                    .with_enabled_roots(),
            )?
            .connect_lazy();
        let interceptor = AuthInterceptor::new(headers);
        let inner = PublishBuildEventClient::with_interceptor(channel, interceptor);
        Ok(Self { inner })
    }

    pub async fn publish_lifecycle_event(
        &mut self,
        event: PublishLifecycleEventRequest,
    ) -> Result<Response<()>, ClientError> {
        let ev = self
            .inner
            .publish_lifecycle_event(Request::new(event))
            .await?;
        Ok(ev)
    }

    pub async fn publish_build_tool_event_stream<
        S: Stream<Item = PublishBuildToolEventStreamRequest> + Send + 'static,
    >(
        &mut self,
        events: S,
    ) -> Result<Response<Streaming<PublishBuildToolEventStreamResponse>>, ClientError> {
        let x = self
            .inner
            .publish_build_tool_event_stream(Request::new(events))
            .await?;
        Ok(x)
    }
}
