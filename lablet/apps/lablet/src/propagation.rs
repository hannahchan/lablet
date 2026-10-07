//! The context a run comes from: what the environment says each run of a
//! `Lablet` is the child of.

use opentelemetry::Context;

use crate::otel_env;

/// What every run of one `Lablet` starts from.
#[derive(Debug, Clone)]
pub(crate) struct Inbound {
    /// The context each run's root span is opened in.
    pub(crate) parent: Context,
}

/// The inbound context the seam's `context` variables give. With none, a
/// run starts from the empty context, so it's a trace of its own whatever
/// span its caller has open.
pub(crate) fn inbound(_context: &otel_env::Context) -> Inbound {
    Inbound {
        parent: Context::new(),
    }
}
