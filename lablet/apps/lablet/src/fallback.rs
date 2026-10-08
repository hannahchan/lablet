//! The library root's fallback to OpenTelemetry's globals, for a host that
//! hands in no tracer provider or no propagator.

use opentelemetry::propagation::text_map_propagator::FieldIter;
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{Context, InstrumentationScope, global};

#[expect(
    clippy::disallowed_methods,
    reason = "a host that hands in no tracer provider gets OpenTelemetry's global one, read once when a Lablet is built; this is its one reader"
)]
fn global_tracer_provider() -> global::GlobalTracerProvider {
    global::tracer_provider()
}

#[expect(
    clippy::disallowed_methods,
    reason = "a host that hands in no propagator gets OpenTelemetry's global one, asked at each injection; this is its one reader"
)]
fn with_global_propagator<T>(f: impl FnMut(&dyn TextMapPropagator) -> T) -> T {
    global::get_text_map_propagator(f)
}

/// The tracer of lablet's scope from the global tracer provider as it is
/// when a `Lablet` is built, so a run never splits across two providers.
pub(crate) fn global_tracer(scope: InstrumentationScope) -> global::BoxedTracer {
    global_tracer_provider().tracer_with_scope(scope)
}

/// A stand-in for the global propagator, which the API lends only inside a
/// closure: it asks the global one at each inject and extract, and its
/// fields are the global one's when it was made.
#[derive(Debug)]
pub(crate) struct GlobalPropagator {
    fields: Vec<String>,
}

impl GlobalPropagator {
    /// The global propagator, its fields as they are now.
    pub(crate) fn new() -> Self {
        Self {
            fields: with_global_propagator(|propagator| {
                propagator.fields().map(str::to_owned).collect()
            }),
        }
    }
}

impl TextMapPropagator for GlobalPropagator {
    fn inject_context(&self, cx: &Context, injector: &mut dyn Injector) {
        with_global_propagator(|propagator| propagator.inject_context(cx, injector));
    }

    fn extract_with_context(&self, cx: &Context, extractor: &dyn Extractor) -> Context {
        with_global_propagator(|propagator| propagator.extract_with_context(cx, extractor))
    }

    fn fields(&self) -> FieldIter<'_> {
        FieldIter::new(&self.fields)
    }
}
