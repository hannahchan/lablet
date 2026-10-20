//! The request headers as a carrier of OpenTelemetry context.

use opentelemetry::propagation::Injector;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::DIAGNOSTICS;

/// Writes a context into a request's headers: each key as the header it
/// names, unless the value is empty or the key or the value isn't one a
/// header may have, which is said on the diagnostic log with the key alone.
#[derive(Debug)]
pub(crate) struct HeaderInjector<'a>(pub(crate) &'a mut HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if value.is_empty() {
            return;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        } else {
            tracing::debug!(
                target: DIAGNOSTICS,
                key,
                "a context header couldn't be set, so the request goes without it"
            );
        }
    }
}
