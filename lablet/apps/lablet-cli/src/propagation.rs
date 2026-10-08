//! The context a run comes from: what the environment says the command
//! line's run is the child of, extracted through the propagators
//! `OTEL_PROPAGATORS` names, once, and made current around the run, and
//! the propagators each process the run starts is given its context
//! through.
//!
//! The propagators are lablet's own, and never OpenTelemetry's global one,
//! which one process holding several `Lablet`s would share, and which a
//! crate could set without a line of lablet's changing.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lablet_env_carrier::{EnvExtractor, variable};
use opentelemetry::Context;
use opentelemetry::propagation::{Extractor, TextMapPropagator};
use opentelemetry::trace::TraceContextExt as _;
use tracing_subscriber::layer::{Context as Subscribed, Layer, SubscriberExt as _};

use crate::otel_env;

/// The key the trace context propagator reads its parent from.
const TRACEPARENT: &str = "traceparent";

/// The key the baggage propagator reads its members from.
const BAGGAGE: &str = "baggage";

/// What the names of the baggage propagator's warnings begin with. Those
/// about a member it can't read hold the whole value.
const BAGGAGE_WARNINGS: &str = "BaggagePropagator.Extract.";

/// The target of this module's warnings, the module they were written in
/// before the kernels were split out, so a line of the diagnostic log, and
/// a `RUST_LOG` directive that names it, are as they were.
const TARGET: &str = "lablet::propagation";

/// What the command line's run starts from.
#[derive(Debug, Clone)]
pub struct Inbound {
    /// The context each run's root span is opened in.
    pub parent: Context,
    /// The composite of the propagators, which a command's context is
    /// injected through.
    pub propagator: Arc<dyn TextMapPropagator + Send + Sync>,
}

/// The inbound context the seam's `context` variables give, extracted
/// once, through its propagators. With none, a run starts from the empty
/// context, so it's a trace of its own whatever span its caller has open.
/// A `TRACEPARENT` the trace context propagator doesn't accept, and a
/// `BAGGAGE` the baggage propagator can't read in full, are warned about,
/// never shown.
pub fn inbound(context: &otel_env::Context) -> Inbound {
    let environment: BTreeMap<OsString, OsString> = context
        .carried
        .iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect();
    let carrier = EnvExtractor(&environment);
    let propagator = context.propagator();
    let unread = Unread::default();
    // Under a subscriber of its own, so that the baggage propagator's
    // warnings reach neither lablet's log nor a host's.
    let parent = tracing::subscriber::with_default(
        tracing_subscriber::registry().with(unread.clone()),
        || propagator.extract_with_context(&Context::new(), &carrier),
    );
    if propagator.fields().any(|field| field == TRACEPARENT)
        && carrier.get(TRACEPARENT).is_some()
        && !parent.span().span_context().is_valid()
    {
        tracing::warn!(
            target: TARGET,
            "`{}` holds a value that isn't a W3C trace parent, so it's ignored and each run \
             starts a trace of its own",
            variable(TRACEPARENT)
        );
    }
    if unread.0.load(Ordering::Relaxed) {
        tracing::warn!(
            target: TARGET,
            "`{}` holds a value the baggage propagator can't read in full, so what it can't read \
             is ignored",
            variable(BAGGAGE)
        );
    }
    Inbound {
        parent,
        propagator: Arc::new(propagator),
    }
}

/// Whether the baggage propagator warned while extracting.
#[derive(Clone, Default)]
struct Unread(Arc<AtomicBool>);

impl<S: tracing::Subscriber> Layer<S> for Unread {
    fn on_event(&self, event: &tracing::Event<'_>, _: Subscribed<'_, S>) {
        if event.metadata().name().starts_with(BAGGAGE_WARNINGS) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests;
