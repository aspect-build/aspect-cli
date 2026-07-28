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

/// The transport URI tonic can dial for a BES `endpoint`.
///
/// `Channel::from_shared` only understands `http`/`https`, while Bazel spells a
/// TLS BES backend `grpcs://` (and plaintext `grpc://`), so those are mapped to
/// their HTTP equivalents. TLS follows from the resulting scheme: tonic applies
/// the `tls_config` below only to an `https` URI, leaving `grpc://` plaintext.
///
/// Confined to this function on purpose — the caller's `endpoint` string is also
/// what user-facing logs report, and rewriting it there would tell a user their
/// `grpcs://` backend is an `https://` one they never configured.
fn transport_uri(endpoint: &str) -> String {
    for (scheme, replacement) in [("grpcs://", "https://"), ("grpc://", "http://")] {
        if let Some(rest) = endpoint.strip_prefix(scheme) {
            return format!("{replacement}{rest}");
        }
    }
    endpoint.to_string()
}

impl Client {
    pub async fn new(
        endpoint: String,
        headers: HashMap<String, String>,
    ) -> Result<Self, ClientError> {
        let channel = Channel::from_shared(transport_uri(&endpoint))?
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every form must survive `Channel::from_shared`, which is what actually
    /// rejects a scheme tonic doesn't know.
    fn dialable(endpoint: &str) -> bool {
        Channel::from_shared(transport_uri(endpoint)).is_ok()
    }

    #[test]
    fn grpc_schemes_map_to_their_http_equivalents() {
        assert_eq!(
            transport_uri("grpcs://bes.example.com"),
            "https://bes.example.com"
        );
        assert_eq!(
            transport_uri("grpcs://bes.example.com:443/x"),
            "https://bes.example.com:443/x"
        );
        assert_eq!(
            transport_uri("grpc://buildbarn-frontend.awd.internal:8980"),
            "http://buildbarn-frontend.awd.internal:8980"
        );
    }

    #[test]
    fn http_schemes_pass_through_untouched() {
        assert_eq!(
            transport_uri("https://bes.example.com"),
            "https://bes.example.com"
        );
        assert_eq!(
            transport_uri("http://localhost:8080"),
            "http://localhost:8080"
        );
    }

    /// Only the leading scheme is rewritten; a `grpcs://` appearing later in the
    /// string (a path or query) is left alone.
    #[test]
    fn only_the_leading_scheme_is_rewritten() {
        assert_eq!(
            transport_uri("https://proxy.example.com/to/grpcs://inner"),
            "https://proxy.example.com/to/grpcs://inner"
        );
    }

    #[test]
    fn every_supported_spelling_is_dialable() {
        for endpoint in [
            "grpcs://bes.example.com",
            "grpcs://bes.example.com:443",
            "grpc://buildbarn-frontend.awd.internal:8980",
            "https://bes.example.com",
            "http://localhost:8080",
        ] {
            assert!(dialable(endpoint), "{endpoint} should be dialable");
        }
    }
}
