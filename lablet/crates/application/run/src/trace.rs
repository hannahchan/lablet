//! The identity of a span, as the two W3C strings that carry it.
//!
//! Its own module because both the observer that opens a span and the tool
//! call that carries it need it, and a type they share belongs below them
//! both. Putting it with either one makes the other import from its sibling,
//! which is how `observer` and `tool` came to import each other.

/// A span's identity, as W3C strings, so no OpenTelemetry type reaches below
/// the adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    /// The `traceparent` header value.
    pub traceparent: String,
    /// The `tracestate` header value, when there is one.
    pub tracestate: Option<String>,
}
