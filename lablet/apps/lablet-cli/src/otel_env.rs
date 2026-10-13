//! The seam where lablet read the OpenTelemetry environment, kept with no
//! production caller: each variable parsed as the specification parses it,
//! through `lablet-otel-env`, from an environment a caller hands in. The
//! command line's settings come from its config alone, and the canaries in
//! `export/tests/canaries/` record what the crates read of the environment
//! that lablet doesn't. Of the three modules, [`context`]'s composition of
//! the propagators is still in use, fed by `telemetry.propagators`, and
//! [`Sdk::default`] is what the composition states on every builder, so the
//! crates' own reading of the sampler and the batch variables never decides.

pub mod context;
pub mod exporter;
pub mod sdk;

pub use context::Context;
pub use exporter::Exporter;
use lablet_otel_env::Env;
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
