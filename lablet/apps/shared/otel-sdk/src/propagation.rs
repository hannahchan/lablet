//! The context a run comes from: what the environment says each run of a
//! `Lablet` is the child of, extracted through the propagators
//! `OTEL_PROPAGATORS` names.
//!
//! The propagators are lablet's own, and never OpenTelemetry's global one,
//! which one process holding several `Lablet`s would share, and which a
//! crate could set without a line of lablet's changing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// What every run of one `Lablet` starts from.
#[derive(Debug, Clone)]
pub struct Inbound {
    /// The context each run's root span is opened in.
    pub parent: Context,
}

/// The inbound context the seam's `context` variables give, extracted
/// once, through its propagators. With none, a run starts from the empty
/// context, so it's a trace of its own whatever span its caller has open.
/// A `TRACEPARENT` the trace context propagator doesn't accept, and a
/// `BAGGAGE` the baggage propagator can't read in full, are warned about,
/// never shown.
pub fn inbound(context: &otel_env::Context) -> Inbound {
    let carrier = Carrier(&context.carried);
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
    Inbound { parent }
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

/// The environment as a carrier of context: a key a propagator asks for
/// is read from the variable its normalised name names, and never from one
/// named as the key is written.
struct Carrier<'a>(&'a [(&'static str, String)]);

impl Extractor for Carrier<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        let name = variable(key);
        self.0
            .iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| value.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.iter().map(|(variable, _)| *variable).collect()
    }
}

/// The name of the variable that carries `key`, as the specification's
/// carriers normalise it: ASCII letters upper-cased, any other character
/// but a digit or `_` made `_`, a leading digit given a `_` before it, and
/// an empty key `_`.
fn variable(key: &str) -> String {
    let mut name: String = key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.starts_with(|character: char| character.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

#[cfg(test)]
mod tests;
