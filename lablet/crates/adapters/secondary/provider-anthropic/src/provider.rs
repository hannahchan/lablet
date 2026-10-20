//! The `ModelProvider` over the API: one request for each attempt.

use std::sync::Arc;

use lablet_model::{Endpoint, ModelRef, ProviderErrorKind, ProviderResponse};
use lablet_run::{ModelProvider, ProviderError, ProviderRequest};
use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator;
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT,
};

use crate::error::{self, Sent};
use crate::propagate::HeaderInjector;
use crate::target::Target;
use crate::{request, response};

/// The API version every request names; no beta header goes with it.
const API_VERSION: &str = "2023-06-01";

/// The product and nothing of the person.
const USER_AGENT_VALUE: &str = concat!("lablet/", env!("CARGO_PKG_VERSION"));

/// A model provider over the Anthropic Messages API. See the crate's
/// documentation for what it sends and how a failure is read.
#[derive(Debug)]
pub struct AnthropicProvider {
    target: Target,
    propagator: Arc<dyn TextMapPropagator + Send + Sync>,
    capture_content: bool,
}

impl AnthropicProvider {
    /// A provider that sends to `target`, injects the current context into
    /// each request's headers through `propagator`, and quotes a body that
    /// isn't the API's error envelope in a failure's message only when
    /// `capture_content` is on.
    #[must_use]
    pub fn new(
        target: Target,
        propagator: Arc<dyn TextMapPropagator + Send + Sync>,
        capture_content: bool,
    ) -> Self {
        Self {
            target,
            propagator,
            capture_content,
        }
    }

    /// The headers of one request: the key, the API version, the content
    /// types, the user agent, a gateway's credentials when the base URL
    /// carried any, and the context current as the request is built, which
    /// is the attempt's chat span's.
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-key"),
            self.target.key().clone(),
        );
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(API_VERSION),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        if let Some(authorization) = self.target.authorization() {
            headers.insert(AUTHORIZATION, authorization.clone());
        }
        self.propagator
            .inject_context(&Context::current(), &mut HeaderInjector(&mut headers));
        headers
    }
}

#[async_trait::async_trait]
impl ModelProvider for AnthropicProvider {
    fn model(&self) -> &ModelRef {
        self.target.model()
    }

    fn endpoint(&self) -> Option<Endpoint> {
        Some(self.target.endpoint().clone())
    }

    /// One request, with the attempt's deadline as its timeout from the
    /// connection to the last body byte; a deadline of zero is reached
    /// before anything is sent.
    ///
    /// # Errors
    ///
    /// Returns the failure the response, the transport or the deadline is
    /// classified as, and a fatal one for a conversation the adapter can't
    /// send.
    async fn complete(
        &self,
        request: ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ProviderError> {
        let deadline = request.deadline;
        if deadline.is_zero() {
            return Err(error::deadline(deadline));
        }
        let body = request::body(&request, &self.target)?;
        let bytes = serde_json::to_vec(&body).map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::Fatal,
                "the request couldn't be written as JSON",
            )
        })?;
        let sent = Sent {
            temperature: request.temperature.is_some(),
            capture_content: self.capture_content,
        };
        let response = self
            .target
            .client()
            .post(self.target.messages_url().clone())
            .headers(self.headers())
            .body(bytes)
            .timeout(deadline)
            .send()
            .await
            .map_err(|error| error::transport(&error, deadline))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|error| error::transport(&error, deadline))?;
        if status.is_success() {
            response::parse(&body)
        } else {
            Err(error::classify(status, &headers, &body, sent))
        }
    }
}
