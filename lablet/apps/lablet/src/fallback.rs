//! The library root's fallback to OpenTelemetry's globals, for a host that
//! hands in no tracer provider.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{InstrumentationScope, global};

#[expect(
    clippy::disallowed_methods,
    reason = "a host that hands in no tracer provider gets OpenTelemetry's global one, read once when a Lablet is built; this is its one reader"
)]
fn global_tracer_provider() -> global::GlobalTracerProvider {
    global::tracer_provider()
}

/// The tracer of lablet's scope from the global tracer provider as it is
/// when a `Lablet` is built, so a run never splits across two providers.
pub(crate) fn global_tracer(scope: InstrumentationScope) -> global::BoxedTracer {
    global_tracer_provider().tracer_with_scope(scope)
}
