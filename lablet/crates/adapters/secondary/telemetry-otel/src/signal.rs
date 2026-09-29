//! What the observer makes of a run's events, before any of it is handed to
//! the SDK: spans and log records as plain values, each whole when it's
//! made.
//!
//! A span is made when it ends, from the events that say when it started
//! and how long it took, so every time in a trace is one the loop measured
//! on the run's own clock. The SDK's tracer would fix a span's id and its
//! start at once, and a tool span needs its id when its call starts, for
//! the executor to propagate, and learns its start when the call ends.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::logs::{LogRecord as _, Logger as _, Severity};
use opentelemetry::trace::{
    Event, SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_sdk::logs::{SdkLogRecord, SdkLogger};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};

use crate::attributes::Attributes;

/// The instant `offset_ms` into a run that started `started_unix_ms` after
/// the Unix epoch.
///
/// The sum is of milliseconds in a `u64`, and the clocks of the platforms
/// lablet runs on count seconds in an `i64`, so every instant this can name
/// is one the clock can say.
pub(crate) fn instant(started_unix_ms: u64, offset_ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(started_unix_ms.saturating_add(offset_ms))
}

/// One thing that happened during a span, at the instant it did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Happened {
    pub(crate) name: &'static str,
    pub(crate) at: SystemTime,
    pub(crate) attributes: Attributes,
}

/// How a span ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ended {
    /// Nothing went wrong.
    Well,
    /// Something did, and this is what there is to say about it. Never
    /// content: a span holds none.
    Badly(String),
}

/// One span, from start to end.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Span {
    pub(crate) name: String,
    pub(crate) kind: SpanKind,
    pub(crate) id: SpanId,
    /// The span this one is a child of; `None` for the root.
    pub(crate) parent: Option<SpanId>,
    pub(crate) start: SystemTime,
    pub(crate) end: SystemTime,
    pub(crate) attributes: Attributes,
    pub(crate) events: Vec<Happened>,
    pub(crate) ended: Ended,
}

/// One log record, in the context of the span it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Record {
    pub(crate) name: &'static str,
    pub(crate) severity: Severity,
    pub(crate) at: SystemTime,
    pub(crate) span: SpanId,
    pub(crate) attributes: Attributes,
}

/// What one event gave rise to.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Signals {
    pub(crate) spans: Vec<Span>,
    pub(crate) records: Vec<Record>,
}

impl Signals {
    pub(crate) fn span(mut self, span: Span) -> Self {
        self.spans.push(span);
        self
    }

    pub(crate) fn record(mut self, record: Option<Record>) -> Self {
        self.records.extend(record);
        self
    }
}

/// Every span of a run is recorded and says so, which is what an executor
/// that propagates one tells the server it calls.
const SAMPLED: TraceFlags = TraceFlags::SAMPLED;

fn key_values(attributes: Attributes) -> Vec<KeyValue> {
    attributes.into_key_values()
}

impl Span {
    /// The span as the SDK's exporters take one, in the trace `trace`.
    pub(crate) fn into_data(self, trace: TraceId, scope: &InstrumentationScope) -> SpanData {
        let mut events = SpanEvents::default();
        events.events = self
            .events
            .into_iter()
            .map(|happened| {
                Event::new(
                    happened.name,
                    happened.at,
                    key_values(happened.attributes),
                    0,
                )
            })
            .collect();
        SpanData {
            span_context: SpanContext::new(trace, self.id, SAMPLED, false, TraceState::default()),
            parent_span_id: self.parent.unwrap_or(SpanId::INVALID),
            parent_span_is_remote: false,
            span_kind: self.kind,
            name: self.name.into(),
            start_time: self.start,
            end_time: self.end,
            attributes: key_values(self.attributes),
            dropped_attributes_count: 0,
            events,
            links: SpanLinks::default(),
            status: match self.ended {
                Ended::Well => Status::Unset,
                Ended::Badly(description) => Status::error(description),
            },
            instrumentation_scope: scope.clone(),
        }
    }
}

impl Record {
    /// The record as `logger` emits one, in the trace `trace`.
    ///
    /// The time it was observed is the time it happened. The SDK would
    /// otherwise read its own clock, and nothing in a run's record is timed
    /// by the observer.
    pub(crate) fn into_sdk(self, trace: TraceId, logger: &SdkLogger) -> SdkLogRecord {
        let mut record = logger.create_log_record();
        record.set_event_name(self.name);
        record.set_severity_number(self.severity);
        record.set_severity_text(self.severity.name());
        record.set_timestamp(self.at);
        record.set_observed_timestamp(self.at);
        record.set_trace_context(trace, self.span, Some(SAMPLED));
        self.attributes.onto(&mut record);
        record
    }
}

/// The `traceparent` of the span `span` of the trace `trace`, as W3C writes
/// one.
pub(crate) fn traceparent(trace: TraceId, span: SpanId) -> String {
    format!("00-{trace}-{span}-{:02x}", SAMPLED.to_u8())
}

#[cfg(test)]
mod tests;
