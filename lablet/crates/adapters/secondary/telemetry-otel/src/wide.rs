//! The wide event: the one record of a run that says everything worth
//! knowing about it.

use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::EVENT_LABLET_RUN_NAME;
use opentelemetry::logs::Severity;

use crate::run::Closed;
use crate::signal::Record;

/// The wide event of the run `closed`, of whose spans and other records
/// the exporters lost `dropped`.
///
/// It's in the context of the run's root span and is timed at the run's
/// end. So far it holds what makes it the run's record and the one count
/// that's the observer's own. The registry's other keys for `lablet.run`
/// are read from the context and the summary that `closed` holds.
pub(crate) fn wide_event(closed: &Closed, dropped: u64) -> Record {
    Record {
        name: EVENT_LABLET_RUN_NAME,
        severity: Severity::Info,
        at: closed.at,
        span: closed.root,
        attributes: closed
            .join
            .clone()
            .with(key::LABLET_TELEMETRY_DROPPED_RECORDS, dropped),
    }
}
