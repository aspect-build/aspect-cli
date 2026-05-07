use std::collections::HashMap;

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
