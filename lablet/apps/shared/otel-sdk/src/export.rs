//! The composition root's export module: the OpenTelemetry SDK and what
//! lablet adds to it. The rings that emit name the API alone, so this is
//! where lablet holds the SDK.
//!
//! The loop opens its spans through a tracer and emits its records through
//! lablet's logger, and `lablet-run`'s runner opens the root span and fills
//! the wide event; this module renders none of them. It builds the providers
//! those come from, sampled and limited as the environment says, and the
//! resource they describe themselves with, and takes what they emit to each
//! destination through the SDK's batch processors: an OTLP collector over
//! the network, by gRPC or HTTP/protobuf, and OTLP/JSON lines in a file. A
//! run may go to both, and the file's processors are flushed first, so the
//! file never waits on the network. What a processor fails to export, the
//! SDK says on the diagnostic log, through `tracing`. Of what it drops, the
//! SDK warns at the first drop and logs the total when the processor is
//! shut down, so which run lost what can't be told.

mod file;
mod network;
mod resource;
mod telemetry;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub use file::FileTarget;
pub use network::{OtelBuildError, OtlpSettings, Signal, Transport, decode_headers, validate};
pub use resource::resource;
pub use telemetry::{FlushError, Telemetry, TelemetryBuilder};
// What `resource` makes, named here so that `lablet-prepare` can hold one
// without depending on the SDK.
pub use opentelemetry_sdk::Resource;
