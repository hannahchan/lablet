//! The seam where lablet reads the OpenTelemetry environment, once, from the
//! environment a check or a build reads everything else from, parsing each
//! variable as the specification parses it, through `lablet-otel-env`. The
//! modules that build from the values take them from here and state each
//! one on the SDK, so the crates' own reading of the process environment
//! never decides.

mod context;
mod exporter;
mod sdk;

pub use context::Context;
pub use exporter::Exporter;
use lablet_config::Env;
pub(crate) use lablet_otel_env::{Choice, Named, Parse, Timeout, Variables};
pub use sdk::Sdk;

/// The variables the seam reads, from one environment.
#[derive(Debug, Clone, PartialEq)]
pub struct OtelEnv {
    /// The SDK's own: the sampler, the span limits and the batch processors.
    pub sdk: Sdk,
    /// The OTLP exporter's.
    pub exporter: Exporter,
    /// The resource's and the inbound context's.
    pub context: Context,
}

impl OtelEnv {
    /// Reads the seam's variables from `env`, warning of each value that
    /// can't be used.
    #[must_use]
    pub fn read(env: Env<'_>) -> Self {
        let variables = Variables(env);
        Self {
            sdk: Sdk::read(&variables),
            exporter: Exporter::read(&variables),
            context: Context::read(&variables),
        }
    }
}

#[cfg(test)]
mod tests;
