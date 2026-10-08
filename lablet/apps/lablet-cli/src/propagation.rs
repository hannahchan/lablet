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

/// The keys each propagator reads a parent from, and what's said when one
/// of them is set and the propagator can't make a parent of them.
const PARENTS: [(&[&str], &str); 3] = [
    (
        &["traceparent"],
        "`TRACEPARENT` holds a value that isn't a W3C trace parent, so it's ignored",
    ),
    (
        &["b3"],
        "`B3` holds a value that isn't a B3 parent, so it's ignored",
    ),
    (
        &["x-b3-traceid", "x-b3-spanid"],
        "`X_B3_TRACEID` and `X_B3_SPANID` hold no B3 parent, so they're ignored",
    ),
];

/// What B3's single header holds when it carries a sampling decision and
/// no parent, which is no mistake.
const B3_DECISIONS: [&str; 3] = ["0", "1", "d"];

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
/// A `TRACEPARENT`, a `B3`, or an `X_B3_TRACEID` or `X_B3_SPANID`, that
/// the propagator reading it can't make a parent of, and a `BAGGAGE` the
/// baggage propagator can't read in full, are warned about, never shown.
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
    if !parent.span().span_context().is_valid() {
        for (keys, said) in PARENTS {
            let set = keys.iter().any(|key| {
                propagator.fields().any(|field| field == *key)
                    && carrier
                        .get(key)
                        .is_some_and(|value| *key != "b3" || !B3_DECISIONS.contains(&value))
            });
            if set {
                tracing::warn!(
                    target: TARGET,
                    "{said} and each run starts a trace of its own"
                );
            }
        }
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
