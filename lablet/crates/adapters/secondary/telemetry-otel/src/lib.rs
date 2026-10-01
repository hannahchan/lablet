//! Secondary adapter: a `RunObserver` that maps a run's events to
//! OpenTelemetry spans and log records, once, and hands them to pluggable
//! exporters.
//!
//! Every span, record and attribute it emits is one the registry under
//! `lablet/telemetry/registry/` declares, and every attribute's name is a
//! constant of `lablet-telemetry-registry`.
//!
//! - The root span `invoke_agent lablet` covers the run, and each attempt of
//!   a provider call and each tool call has a span beneath it. A span is
//!   timed by what the loop measured, on the run's clock.
//! - A provider call that failed has a log record beside its span, and
//!   content has log records of its own when the run captures any. No span
//!   holds content.
//! - The wide event `lablet.run` is the run's one row. It's exported alone
//!   and last, so that a batch of content a collector refuses for its size
//!   can't take it along.
//!
//! Two destinations: an OTLP collector over the network, by gRPC or
//! HTTP/protobuf, and OTLP/JSON lines in a file. A run may go to both, and
//! each is flushed on its own, so the file never waits on the network.

mod attributes;
mod content;
mod file;
mod network;
mod observer;
mod pipeline;
mod run;
mod signal;
#[cfg(test)]
mod testing;
mod wide;

pub use attributes::ATTRIBUTE_MAX_BYTES;
pub use file::FileTarget;
pub use network::{OtelBuildError, OtlpSettings, Signal, Transport, decode_headers, validate};
pub use observer::{FlushError, OtelObserver, OtelObserverBuilder};

#[cfg(test)]
mod tests;
