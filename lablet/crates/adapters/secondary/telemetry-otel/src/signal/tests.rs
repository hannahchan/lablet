use std::time::Duration;

use lablet_telemetry_registry::SCHEMA_URL;
use opentelemetry::logs::{AnyValue, LoggerProvider as _};
use opentelemetry::{Key, Value};
use opentelemetry_sdk::logs::SdkLoggerProvider;

use super::*;
use crate::testing::STARTED_UNIX_MS;

const TRACE: TraceId = TraceId::from_bytes([0xab; 16]);
const SPAN: SpanId = SpanId::from_bytes([0xcd; 8]);
const PARENT: SpanId = SpanId::from_bytes([0xef; 8]);

fn scope() -> InstrumentationScope {
    InstrumentationScope::builder("lablet")
        .with_version("0.1.0")
        .with_schema_url(SCHEMA_URL)
        .build()
}

fn after(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn span(parent: Option<SpanId>, ended: Ended) -> Span {
    Span {
        name: "chat scripted-1".to_owned(),
        kind: SpanKind::Client,
        id: SPAN,
        parent,
        start: after(5),
        end: after(255),
        attributes: Attributes::default().with("lablet.test.turn", 2_u32),
        events: vec![Happened {
            name: "lablet.retry",
            at: after(250),
            attributes: Attributes::default().with("lablet.test.will_retry", true),
        }],
        ended,
    }
}

#[test]
fn an_instant_is_as_far_into_the_run_as_its_offset_says() {
    assert_eq!(
        instant(STARTED_UNIX_MS, 2_047),
        after(STARTED_UNIX_MS + 2_047)
    );
    assert_eq!(instant(0, 0), UNIX_EPOCH);
}

#[test]
fn the_last_instant_a_run_can_name_is_one_the_clock_can_say() {
    assert_eq!(instant(u64::MAX, 1), after(u64::MAX));
    assert_eq!(instant(1, u64::MAX), after(u64::MAX));
}

#[test]
fn a_span_is_handed_to_the_sdk_as_it_was_made() {
    let data = span(Some(PARENT), Ended::Well).into_data(TRACE, &scope());

    assert_eq!(data.span_context.trace_id(), TRACE);
    assert_eq!(data.span_context.span_id(), SPAN);
    assert!(data.span_context.is_sampled());
    assert!(!data.span_context.is_remote());
    assert_eq!(data.parent_span_id, PARENT);
    assert!(!data.parent_span_is_remote);
    assert_eq!(data.span_kind, SpanKind::Client);
    assert_eq!(data.name, "chat scripted-1");
    assert_eq!(data.start_time, after(5));
    assert_eq!(data.end_time, after(255));
    assert_eq!(
        data.attributes,
        [KeyValue::new("lablet.test.turn", Value::I64(2))]
    );
    assert_eq!(data.dropped_attributes_count, 0);
    assert_eq!(data.events.len(), 1);
    assert_eq!(data.events[0].name, "lablet.retry");
    assert_eq!(
        data.events[0].timestamp,
        after(250),
        "the event is timed as it happened, not as the span ended"
    );
    assert_eq!(
        data.events[0].attributes,
        [KeyValue::new("lablet.test.will_retry", Value::Bool(true))]
    );
    assert_eq!(data.events.dropped_count, 0);
    assert!(data.links.is_empty());
    assert_eq!(data.status, Status::Unset);
    assert_eq!(data.instrumentation_scope.name(), "lablet");
    assert_eq!(data.instrumentation_scope.version(), Some("0.1.0"));
    assert_eq!(data.instrumentation_scope.schema_url(), Some(SCHEMA_URL));
}

#[test]
fn a_root_span_has_no_parent_and_a_span_that_ended_badly_says_so() {
    let data = span(None, Ended::Badly("529 overloaded".to_owned())).into_data(TRACE, &scope());

    assert_eq!(data.parent_span_id, SpanId::INVALID);
    assert_eq!(data.status, Status::error("529 overloaded"));
}

#[test]
fn a_record_is_in_the_context_of_its_span_and_is_timed_by_the_run() {
    let provider = SdkLoggerProvider::builder().build();
    let logger = provider.logger_with_scope(scope());
    let record = Record {
        name: "gen_ai.client.operation.exception",
        severity: Severity::Warn,
        at: after(255),
        span: SPAN,
        attributes: Attributes::default().with("lablet.test.turn", 2_u32),
    };

    let made = record.into_sdk(TRACE, &logger);

    assert_eq!(made.event_name(), Some("gen_ai.client.operation.exception"));
    assert_eq!(made.severity_number(), Some(Severity::Warn));
    assert_eq!(made.severity_text(), Some("WARN"));
    assert_eq!(made.timestamp(), Some(after(255)));
    assert_eq!(
        made.observed_timestamp(),
        Some(after(255)),
        "the observer's own clock is read for nothing"
    );
    let context = made.trace_context().unwrap();
    assert_eq!(context.trace_id, TRACE);
    assert_eq!(context.span_id, SPAN);
    assert_eq!(context.trace_flags, Some(TraceFlags::SAMPLED));
    assert_eq!(made.body(), None);
    let attributes: Vec<_> = made.attributes_iter().cloned().collect();
    assert_eq!(
        attributes,
        [(Key::new("lablet.test.turn"), AnyValue::Int(2))]
    );
}

#[test]
fn a_traceparent_is_the_w3c_form_of_a_sampled_span() {
    assert_eq!(
        traceparent(TRACE, SPAN),
        "00-abababababababababababababababab-cdcdcdcdcdcdcdcd-01"
    );
    assert_eq!(
        traceparent(
            TraceId::from_bytes([0; 16]),
            SpanId::from_bytes([0, 0, 0, 0, 0, 0, 0, 7])
        ),
        "00-00000000000000000000000000000000-0000000000000007-01",
        "an id is as long as it is whatever it holds"
    );
}

#[test]
fn signals_keep_their_spans_and_their_records_in_the_order_they_were_given() {
    let record = |name| Record {
        name,
        severity: Severity::Info,
        at: after(1),
        span: SPAN,
        attributes: Attributes::default(),
    };

    let signals = Signals::default()
        .record(Some(record("first")))
        .record(None)
        .span(span(None, Ended::Well))
        .record(Some(record("second")));

    let names: Vec<_> = signals.records.iter().map(|record| record.name).collect();
    assert_eq!(names, ["first", "second"]);
    assert_eq!(signals.spans, [span(None, Ended::Well)]);
}
