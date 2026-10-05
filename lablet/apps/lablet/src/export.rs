//! The composition root's export module: the OpenTelemetry SDK and what
//! lablet adds to it. The rings that emit name the API alone, so this is
//! where lablet holds the SDK.
//!
//! The loop opens its spans through a tracer and emits its records through
//! lablet's logger, and the composition root opens the root span and fills
//! the wide event; this module renders none of them. It builds the providers
//! those come from, with every trace sampled and the span limits set as
//! lablet means them, and takes what they emit to each destination: an OTLP
//! collector over the network, by gRPC or HTTP/protobuf, and OTLP/JSON lines
//! in a file. A run may go to both, and each is flushed on its own, so the
//! file never waits on the network.
//!
//! Each destination counts what it lost, a span or a record a full queue
//! turned away or an export failed to deliver, and emits the run's wide
//! event with its own count, alone and last, so that a batch a collector
//! refuses for its size can't take the wide event along. Its diagnostics,
//! and the SDK's own export failures, go to `tracing`.

mod file;
mod network;
mod pipeline;
mod telemetry;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub(crate) use file::FileTarget;
pub(crate) use network::{
    OtelBuildError, OtlpSettings, Signal, Transport, decode_headers, validate,
};
pub(crate) use telemetry::Telemetry;
// The composition root reads a flush's failures without naming their type;
// the tests name it, and reach the module only through these re-exports.
#[cfg(test)]
pub(crate) use telemetry::FlushError;
