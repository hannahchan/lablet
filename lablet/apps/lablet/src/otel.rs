//! [`Otel`]: the tracer, the logger and the propagator a host hands in.

use std::sync::Arc;

use lablet_run::telemetry::generated::SCHEMA_URL;
use lablet_run::telemetry::{Bridge, Logger, SCOPE};
use opentelemetry::InstrumentationScope;
use opentelemetry::global::BoxedTracer;
use opentelemetry::logs::{LoggerProvider, NoopLoggerProvider};
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::noop::{NoopTextMapPropagator, NoopTracerProvider};
use opentelemetry::trace::{Tracer, TracerProvider};

/// The OpenTelemetry a host runs lablet on: the tracer provider a run's
/// spans go to, the logger provider its records go to, and the propagator a
/// command's context is injected through. A host hands all three in, and
/// lablet reads none of OpenTelemetry's globals, so what a `Lablet` emits
/// and injects depends only on what its host handed it.
///
/// [`Otel::new`] takes one tracer and one logger of lablet's
/// instrumentation scope from the providers, and keeps neither provider. A
/// clone shares them, so one `Otel` builds every `Lablet` of a host, and
/// each runs on the same three pieces. [`Otel::noop`] is the API's no-op for
/// each, for a host that wants nothing of lablet's telemetry, and a host
/// that wants one piece silent hands in the API's no-op for that one:
/// [`NoopTracerProvider`], [`NoopLoggerProvider`] or
/// [`NoopTextMapPropagator`]. Under the no-op tracer provider a command's
/// context is the one current where the run is awaited, since the API's
/// no-op span carries its parent's context.
///
/// A host on OpenTelemetry's globals hands them in itself: the global
/// tracer provider is `opentelemetry::global::tracer_provider()`, called
/// once the host has set its global tracer provider, since the `Otel` takes
/// its tracer when it's made, and the global propagator, which the API
/// lends only inside a closure, is the propagator the host gave
/// `set_text_map_propagator`, made again, or one of the host's own that
/// asks `get_text_map_propagator` at each call. That one names the global
/// propagator's fields in its own `fields`, since a `bash` command keeps
/// what it inherits of a field its propagator doesn't name, unless it's one
/// of `TRACEPARENT`, `TRACESTATE`, `BAGGAGE`, `B3` and the `X_B3_*`
/// variables.
#[derive(Clone)]
pub struct Otel {
    /// lablet's tracer, taken from the host's tracer provider.
    pub(crate) tracer: Arc<BoxedTracer>,
    /// lablet's logger, taken from the host's logger provider.
    pub(crate) logger: Arc<dyn Logger>,
    /// The host's propagator.
    pub(crate) propagator: Arc<dyn TextMapPropagator + Send + Sync>,
}

impl core::fmt::Debug for Otel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Otel")
            .field("propagator", &self.propagator)
            .finish_non_exhaustive()
    }
}

impl Otel {
    /// The host's OpenTelemetry: the spans of every run go to
    /// `tracer_provider`, its records to `logger_provider`, and a `bash`
    /// command's environment is given the context of its tool span through
    /// `propagator`, as the command starts.
    ///
    /// None of the three can be left out:
    ///
    /// ```compile_fail,E0061
    /// use opentelemetry::logs::NoopLoggerProvider;
    /// use opentelemetry::trace::noop::NoopTracerProvider;
    ///
    /// let otel = lablet::Otel::new(NoopTracerProvider::new(), NoopLoggerProvider::new());
    /// ```
    ///
    /// And there's no default to stand in for them:
    ///
    /// ```compile_fail,E0599
    /// let otel = lablet::Otel::default();
    /// ```
    #[must_use]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "a host hands its providers in, as OpenTelemetry's own setters take one, and lablet keeps the tracer and the logger it takes from them rather than the providers"
    )]
    pub fn new<T, L, P>(tracer_provider: T, logger_provider: L, propagator: P) -> Self
    where
        T: TracerProvider,
        T::Tracer: Send + Sync + 'static,
        <T::Tracer as Tracer>::Span: Send + Sync + 'static,
        L: LoggerProvider,
        L::Logger: Send + Sync + 'static,
        P: TextMapPropagator + Send + Sync + 'static,
    {
        Self {
            tracer: Arc::new(BoxedTracer::new(Box::new(
                tracer_provider.tracer_with_scope(scope()),
            ))),
            logger: Arc::new(Bridge::new(logger_provider.logger_with_scope(scope()))),
            propagator: Arc::new(propagator),
        }
    }

    /// The API's no-op tracer provider, logger provider and propagator: a
    /// `Lablet` built on it emits nothing and injects no context.
    #[must_use]
    pub fn noop() -> Self {
        Self::new(
            NoopTracerProvider::new(),
            NoopLoggerProvider::new(),
            NoopTextMapPropagator::new(),
        )
    }
}

/// lablet's instrumentation scope: its name, its version and the schema
/// URL of the registry its signals are declared in.
fn scope() -> InstrumentationScope {
    InstrumentationScope::builder(SCOPE)
        .with_version(crate::VERSION)
        .with_schema_url(SCHEMA_URL)
        .build()
}
