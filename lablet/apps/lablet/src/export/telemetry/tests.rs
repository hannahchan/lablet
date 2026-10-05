use std::time::{Duration, Instant};

use lablet_run::telemetry::Record;
use opentelemetry::logs::{AnyValue, Severity};
use opentelemetry::trace::{
    Span as _, SpanContext, SpanId, TraceFlags, TraceId, TraceState, Tracer as _,
};
use opentelemetry::{Context, Key, Value};
use opentelemetry_sdk::trace::SpanData;

use super::*;
use crate::export::network::{OtelBuildError, OtlpSettings, Transport};
use crate::export::pipeline::QUEUE_CAPACITY;
use crate::export::testing::memory::{Export, Logged, Memory};
use crate::export::testing::{
    CONTENT, DROPPED_KEY, DURATION_MS, EXCEPTION, OTHER_RUN, RECORDS_PER_CAPTURED_RUN,
    RECORDS_PER_RUN, RUN, RUN_KEY, Records, SCHEMA_URL, SPANS_PER_RUN, VERSION, WIDE, after,
    emit_run, scope, wide_event,
};

fn exporting(memory: &Memory) -> Telemetry {
    Telemetry::builder(VERSION, scope())
        .exporting_to(memory.spans(), memory.records(), memory.wide())
        .build()
        .unwrap()
}

/// Emits one run under `run` and flushes it with its wide event.
async fn run(telemetry: &Telemetry, run: &str, records: Records) -> Result<(), FlushError> {
    let wide = emit_run(telemetry, run, records);
    telemetry.flush(wide).await
}

fn names(spans: &[SpanData]) -> Vec<&str> {
    spans.iter().map(|span| span.name.as_ref()).collect()
}

fn events_of(records: &[Logged]) -> Vec<&str> {
    records
        .iter()
        .map(|(record, _)| record.event_name().unwrap())
        .collect()
}

fn held(record: &Logged, key: &'static str) -> AnyValue {
    record
        .0
        .attributes_iter()
        .find_map(|(held, value)| (held.as_str() == key).then(|| value.clone()))
        .unwrap_or_else(|| panic!("the record holds no `{key}`"))
}

fn dropped(record: &Logged) -> AnyValue {
    held(record, DROPPED_KEY)
}

/// `n` as a wide event counts it.
fn count(n: usize) -> AnyValue {
    AnyValue::Int(i64::try_from(n).unwrap())
}

/// The queues each failure of `error` names.
fn queues(error: &FlushError) -> Vec<&str> {
    error
        .failures()
        .iter()
        .map(|failure| failure.split(':').next().unwrap())
        .collect()
}

// What a run is exported as

#[tokio::test]
async fn a_run_is_exported_as_its_spans_its_records_and_its_wide_event() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);

    run(&telemetry, RUN, Records::Captured).await.unwrap();

    assert_eq!(
        names(&memory.exported_spans()),
        [
            "chat scripted-1",
            "chat scripted-1",
            "execute_tool bash",
            "invoke_agent lablet"
        ],
        "each span is handed over when it ends"
    );
    assert_eq!(
        events_of(&memory.exported_records()),
        [CONTENT, EXCEPTION, CONTENT, CONTENT, CONTENT]
    );
    assert_eq!(events_of(&memory.exported_wide()), [WIDE]);
}

#[tokio::test]
async fn a_run_that_captures_no_content_is_exported_without_any() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);

    run(&telemetry, RUN, Records::Exception).await.unwrap();

    assert_eq!(memory.exported_spans().len(), SPANS_PER_RUN);
    assert_eq!(events_of(&memory.exported_records()), [EXCEPTION]);
    assert_eq!(events_of(&memory.exported_wide()), [WIDE]);
}

#[tokio::test]
async fn the_signals_of_a_run_are_one_sampled_trace_beneath_the_root_span() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);

    run(&telemetry, RUN, Records::Captured).await.unwrap();

    let spans = memory.exported_spans();
    let root = spans.last().unwrap();
    let trace = root.span_context.trace_id();
    assert_eq!(root.parent_span_id, SpanId::INVALID);
    assert!(root.span_context.is_sampled());
    for span in &spans[..3] {
        assert_eq!(span.span_context.trace_id(), trace);
        assert_eq!(span.parent_span_id, root.span_context.span_id());
        assert!(span.span_context.is_sampled());
    }
    let ids: Vec<_> = spans
        .iter()
        .map(|span| span.span_context.span_id())
        .collect();
    let in_the_context_of: Vec<_> = memory
        .exported_records()
        .iter()
        .chain(&memory.exported_wide())
        .map(|(record, _)| {
            let context = record.trace_context().unwrap();
            assert_eq!(context.trace_id, trace);
            ids.iter()
                .position(|id| *id == context.span_id)
                .map(|span| spans[span].name.as_ref())
        })
        .collect();
    assert_eq!(
        in_the_context_of,
        [
            Some("invoke_agent lablet"),
            Some("chat scripted-1"),
            Some("chat scripted-1"),
            Some("chat scripted-1"),
            Some("execute_tool bash"),
            Some("invoke_agent lablet"),
        ]
    );
    assert_eq!(
        memory.exported_records()[1]
            .0
            .trace_context()
            .unwrap()
            .span_id,
        ids[0],
        "the exception is in the context of the attempt that failed"
    );
}

#[tokio::test]
async fn two_runs_are_two_traces() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);

    run(&telemetry, RUN, Records::Exception).await.unwrap();
    run(&telemetry, OTHER_RUN, Records::Exception)
        .await
        .unwrap();

    let spans = memory.exported_spans();
    assert_eq!(spans.len(), 2 * SPANS_PER_RUN);
    assert_ne!(
        spans[3].span_context.trace_id(),
        spans[7].span_context.trace_id()
    );
    let of_the_runs: Vec<_> = memory
        .exported_wide()
        .iter()
        .map(|(record, _)| record.trace_context().unwrap().span_id)
        .collect();
    assert_eq!(
        of_the_runs,
        [
            spans[3].span_context.span_id(),
            spans[7].span_context.span_id()
        ]
    );
}

/// The limits are set so the environment can't lower them, and they're the
/// SDK's defaults, so a span the loop fills never meets them.
#[tokio::test]
async fn a_span_keeps_up_to_128_attributes_and_128_events() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    let tracer = telemetry.tracer();
    let attributes: Vec<_> = (0..130)
        .map(|n| KeyValue::new(format!("lablet.test.{n}"), i64::from(n)))
        .collect();

    let mut span = tracer
        .span_builder("chat scripted-1")
        .with_start_time(after(0))
        .with_attributes(attributes)
        .start_with_context(&tracer, &Context::new());
    for n in 0..130 {
        span.add_event_with_timestamp(format!("event {n}"), after(1), Vec::new());
    }
    span.end_with_timestamp(after(2));
    telemetry.shutdown().await.unwrap();

    let spans = memory.exported_spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].attributes.len(), 128);
    assert_eq!(spans[0].dropped_attributes_count, 2);
    assert_eq!(spans[0].events.len(), 128);
    assert_eq!(spans[0].events.dropped_count, 2);
}

// The wide event

#[tokio::test]
async fn the_wide_event_is_exported_alone_and_after_everything_else_of_its_run() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);

    run(&telemetry, RUN, Records::Captured).await.unwrap();

    let exports = memory.exports();
    let Some(Export::Wide(last)) = exports.last() else {
        panic!("the last export is the wide event's: {exports:?}");
    };
    assert_eq!(events_of(last), [WIDE]);
    let others = &exports[..exports.len() - 1];
    assert!(
        others
            .iter()
            .all(|export| matches!(export, Export::Spans(_) | Export::Records(_))),
        "the wide event has one export: {exports:?}"
    );
    assert!(
        others.iter().all(|export| match export {
            Export::Records(records) => !events_of(records).contains(&WIDE),
            Export::Spans(_) | Export::Wide(_) => true,
        }),
        "and it's in no other"
    );
}

#[tokio::test]
async fn the_wide_event_is_the_record_the_function_makes_timed_and_placed_as_it_says() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);

    run(&telemetry, RUN, Records::Exception).await.unwrap();

    let wide = memory.exported_wide();
    let (record, _) = &wide[0];
    let root = memory.exported_spans().last().unwrap().span_context.clone();
    assert_eq!(record.event_name(), Some(WIDE));
    assert_eq!(record.severity_number(), Some(Severity::Info));
    assert_eq!(record.timestamp(), Some(after(DURATION_MS)));
    assert_eq!(record.observed_timestamp(), Some(after(DURATION_MS)));
    let context = record.trace_context().unwrap();
    assert_eq!(context.trace_id, root.trace_id());
    assert_eq!(context.span_id, root.span_id());
    assert_eq!(held(&wide[0], RUN_KEY), RUN.into());
    assert_eq!(dropped(&wide[0]), AnyValue::Int(0));
}

#[tokio::test]
async fn the_wide_event_counts_what_the_exporters_lost_of_its_run() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.refuse_records(true);

    let flushed = run(&telemetry, RUN, Records::Captured).await;

    let failures = flushed.unwrap_err();
    assert_eq!(failures.failures().len(), 1, "{failures}");
    assert!(
        failures.failures()[0].starts_with("log records: "),
        "{failures}"
    );
    assert!(
        failures
            .to_string()
            .starts_with("telemetry wasn't exported whole: log records: "),
        "{failures}"
    );
    assert!(memory.exported_records().is_empty());
    assert_eq!(memory.exported_spans().len(), SPANS_PER_RUN);
    assert_eq!(
        dropped(&memory.exported_wide()[0]),
        count(RECORDS_PER_CAPTURED_RUN),
        "the five records of the run, which the flush itself lost"
    );
}

#[tokio::test]
async fn the_wide_event_counts_the_spans_and_the_records_together() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.refuse_records(true);
    memory.refuse_spans(true);

    let flushed = run(&telemetry, RUN, Records::Exception).await;

    let failures = flushed.unwrap_err();
    assert_eq!(queues(&failures), ["spans", "log records"]);
    assert_eq!(
        dropped(&memory.exported_wide()[0]),
        count(SPANS_PER_RUN + RECORDS_PER_RUN)
    );
}

#[tokio::test]
async fn the_wide_event_counts_what_a_full_queue_turned_away() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    let (tracer, logger) = (telemetry.tracer(), telemetry.logger());
    memory.hold();

    let mut root = None;
    for _ in 0..QUEUE_CAPACITY + 3 {
        let mut span = tracer
            .span_builder("chat scripted-1")
            .with_start_time(after(0))
            .start_with_context(&tracer, &Context::new());
        let context = span.span_context().clone();
        span.end_with_timestamp(after(1));
        logger.emit(Record {
            name: EXCEPTION,
            severity: Severity::Warn,
            at: after(1),
            span: context.clone(),
            attributes: Vec::new(),
        });
        root.get_or_insert(context);
    }
    memory.release();
    telemetry
        .flush(wide_event(RUN, root.unwrap()))
        .await
        .unwrap();

    assert_eq!(memory.exported_spans().len(), QUEUE_CAPACITY);
    assert_eq!(memory.exported_records().len(), QUEUE_CAPACITY);
    assert_eq!(
        dropped(&memory.exported_wide()[0]),
        AnyValue::Int(3 + 3),
        "three spans and three records"
    );
}

#[tokio::test]
async fn a_runs_wide_event_counts_nothing_of_the_run_before_it() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.refuse_spans(true);
    run(&telemetry, RUN, Records::Exception).await.unwrap_err();
    memory.refuse_spans(false);

    let flushed = run(&telemetry, OTHER_RUN, Records::Exception).await;

    assert_eq!(flushed, Ok(()));
    let lost: Vec<_> = memory.exported_wide().iter().map(dropped).collect();
    assert_eq!(lost, [count(SPANS_PER_RUN), AnyValue::Int(0)]);
}

#[tokio::test]
async fn a_wide_event_that_was_lost_is_reported_and_is_counted_against_no_run() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.refuse_wide(true);

    let lost = run(&telemetry, RUN, Records::Exception).await;
    memory.refuse_wide(false);
    let kept = run(&telemetry, OTHER_RUN, Records::Exception).await;

    let lost = lost.unwrap_err();
    assert_eq!(lost.failures().len(), 1, "{lost}");
    assert!(lost.failures()[0].starts_with("the wide event: "), "{lost}");
    assert_eq!(kept, Ok(()));
    let wide = memory.exported_wide();
    assert_eq!(wide.len(), 1, "a wide event is made once");
    assert_eq!(dropped(&wide[0]), AnyValue::Int(0));
}

#[tokio::test]
async fn a_flush_of_a_run_that_emitted_nothing_exports_its_wide_event_alone() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    // The context of a root span no tracer of the telemetry's opened, so
    // nothing but the wide event is handed over.
    let root = SpanContext::new(
        TraceId::from_bytes([0xab; 16]),
        SpanId::from_bytes([0xcd; 8]),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let flushed = telemetry.flush(wide_event(RUN, root)).await;

    assert_eq!(flushed, Ok(()));
    assert_eq!(memory.exports().len(), 1, "{:?}", memory.exports());
    assert_eq!(events_of(&memory.exported_wide()), [WIDE]);
}

// Shutting down

#[tokio::test]
async fn a_shutdown_after_every_run_was_flushed_exports_nothing_more() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    run(&telemetry, RUN, Records::Exception).await.unwrap();
    let exports = memory.exports().len();

    let shut = telemetry.shutdown().await;

    assert_eq!(shut, Ok(()));
    assert_eq!(memory.exports().len(), exports);
}

#[tokio::test]
async fn a_run_nobody_flushed_has_its_spans_and_records_exported_when_the_telemetry_shuts_down() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    let _wide = emit_run(&telemetry, RUN, Records::Captured);

    let shut = telemetry.shutdown().await;

    assert_eq!(shut, Ok(()));
    assert_eq!(memory.exported_spans().len(), SPANS_PER_RUN);
    assert_eq!(memory.exported_records().len(), RECORDS_PER_CAPTURED_RUN);
    assert!(
        memory.exported_wide().is_empty(),
        "the wide event is the composition root's to hand over, with the flush"
    );
}

#[tokio::test]
async fn a_telemetry_that_was_shut_down_exports_nothing_more_and_says_so() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    telemetry.shutdown().await.unwrap();

    let flushed = run(&telemetry, RUN, Records::Captured).await;
    let again = telemetry.shutdown().await;

    assert!(memory.exports().is_empty(), "{:?}", memory.exports());
    let failures = flushed.unwrap_err();
    assert_eq!(
        queues(&failures),
        ["spans", "log records", "the wide event"],
        "each queue says that it has stopped: {failures}"
    );
    assert!(again.is_err());
}

#[tokio::test]
async fn a_queue_that_cannot_export_fails_the_shutdown_and_stops_all_the_same() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.refuse_spans(true);
    let _wide = emit_run(&telemetry, RUN, Records::Exception);

    let shut = telemetry.shutdown().await;

    let failures = shut.unwrap_err();
    assert_eq!(queues(&failures), ["spans"], "{failures}");
    assert!(memory.exported_spans().is_empty());
    assert_eq!(memory.exported_records().len(), RECORDS_PER_RUN);
    let after = telemetry
        .flush(wide_event(RUN, SpanContext::empty_context()))
        .await;
    assert_eq!(
        queues(&after.unwrap_err()),
        ["spans", "log records", "the wide event"],
        "every queue stopped"
    );
}

/// A destination that doesn't answer holds a shutdown for no longer than
/// the timeout it's given, which is far less than the five seconds the SDK
/// waits for a flush. What it bounds is the export that never ends.
#[tokio::test]
async fn a_destination_that_does_not_answer_holds_a_shutdown_only_for_its_timeout() {
    let brief = Duration::from_millis(100);
    let memory = Memory::default();
    let telemetry = Telemetry::builder(VERSION, scope())
        .exporting_to(memory.spans(), memory.records(), memory.wide())
        .shutdown_timeout(brief)
        .build()
        .unwrap();
    memory.hold();
    let _wide = emit_run(&telemetry, RUN, Records::Exception);

    let began = Instant::now();
    let shut = telemetry.shutdown().await;
    let waited = began.elapsed();
    memory.release();

    assert_eq!(
        shut.unwrap_err().failures(),
        ["the shutdown didn't end within 100ms"],
        "the shutdown stopped waiting at its timeout, and at nothing before it"
    );
    assert!(waited >= brief, "the shutdown gave up after {waited:?}");
    // A shutdown that waited on the stuck exporter, or took the default
    // bound in place of the one it was given, costs at least the SDK's five
    // seconds, which the tolerance rules out.
    assert!(
        waited < brief + Duration::from_secs(1),
        "the shutdown waited {waited:?}, past the bound it was given"
    );
}

/// The SDK's providers stop their processors when they're dropped, each for
/// up to five seconds of the SDK's. After lablet's shutdown every queue
/// answers that at once, so the drop costs nothing and stops nothing again;
/// without lablet's shutdown, the SDK's is what exports what the queues held.
#[tokio::test]
async fn a_telemetry_dropped_after_its_shutdown_costs_nothing_and_one_dropped_without_is_stopped_by_the_sdk()
 {
    let (shut, dropped) = (Memory::default(), Memory::default());
    let telemetry = exporting(&shut);
    run(&telemetry, RUN, Records::Exception).await.unwrap();
    telemetry.shutdown().await.unwrap();
    let exports = shut.exports().len();
    let began = Instant::now();
    drop(telemetry);
    let cost = began.elapsed();
    assert!(cost < Duration::from_secs(1), "the drop cost {cost:?}");
    assert_eq!(shut.exports().len(), exports);

    let telemetry = exporting(&dropped);
    let _wide = emit_run(&telemetry, RUN, Records::Exception);
    drop(telemetry);
    assert_eq!(dropped.exported_spans().len(), SPANS_PER_RUN);
    assert_eq!(dropped.exported_records().len(), RECORDS_PER_RUN);
}

// What an export says of where it came from

#[tokio::test]
async fn every_signal_is_of_the_scope_the_composition_root_handed_over() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);

    run(&telemetry, RUN, Records::Captured).await.unwrap();

    let scopes: Vec<_> = memory
        .exported_spans()
        .into_iter()
        .map(|span| span.instrumentation_scope)
        .chain(
            memory
                .exported_records()
                .into_iter()
                .chain(memory.exported_wide())
                .map(|(_, scope)| scope),
        )
        .collect();
    assert_eq!(scopes.len(), SPANS_PER_RUN + RECORDS_PER_CAPTURED_RUN + 1);
    for scope in scopes {
        assert_eq!(scope.name(), "lablet");
        assert_eq!(scope.version(), Some(VERSION));
        assert_eq!(scope.schema_url(), Some(SCHEMA_URL));
    }
}

#[tokio::test]
async fn every_exporter_is_told_the_service_the_sdk_and_what_the_composer_added() {
    let memory = Memory::default();

    let telemetry = Telemetry::builder(VERSION, scope())
        .resource(vec![
            ("team".to_owned(), "evals".to_owned()),
            ("service.name".to_owned(), "not-lablet".to_owned()),
            ("deployment.environment.name".to_owned(), "ci".to_owned()),
        ])
        .exporting_to(memory.spans(), memory.records(), memory.wide())
        .build()
        .unwrap();

    let resources = memory.resources();
    assert_eq!(resources.len(), 3, "the three exporters of one destination");
    for resource in resources {
        let said = |key: &'static str| resource.get(&Key::new(key));
        assert_eq!(said("service.name"), Some(Value::from("lablet")));
        assert_eq!(said("service.version"), Some(Value::from(VERSION)));
        assert_eq!(said("team"), Some(Value::from("evals")));
        assert_eq!(said("deployment.environment.name"), Some(Value::from("ci")));
        assert_eq!(
            said("telemetry.sdk.name"),
            Some(Value::from("opentelemetry"))
        );
        assert_eq!(said("telemetry.sdk.language"), Some(Value::from("rust")));
        assert!(said("telemetry.sdk.version").is_some());
        assert_eq!(resource.len(), 7, "{resource:?}");
    }
    telemetry.shutdown().await.unwrap();
}

// Where a run is exported to

#[tokio::test]
async fn every_destination_is_handed_every_span_and_every_record_and_counts_its_own_losses() {
    let (first, second) = (Memory::default(), Memory::default());
    let telemetry = Telemetry::builder(VERSION, scope())
        .exporting_to(first.spans(), first.records(), first.wide())
        .exporting_over_the_network_to(second.spans(), second.records(), second.wide())
        .build()
        .unwrap();
    second.refuse_records(true);

    let flushed = run(&telemetry, RUN, Records::Captured).await;

    let failures = flushed.unwrap_err();
    assert_eq!(failures.failures().len(), 1, "{failures}");
    assert!(
        failures.failures()[0].starts_with("otlp log records: "),
        "{failures}"
    );
    assert_eq!(first.exported_spans(), second.exported_spans());
    assert_eq!(first.exported_spans().len(), SPANS_PER_RUN);
    assert_eq!(first.exported_records().len(), RECORDS_PER_CAPTURED_RUN);
    assert!(second.exported_records().is_empty());
    assert_eq!(events_of(&first.exported_wide()), [WIDE]);
    assert_eq!(events_of(&second.exported_wide()), [WIDE]);
    assert_eq!(
        dropped(&first.exported_wide()[0]),
        AnyValue::Int(0),
        "the first lost nothing, and counts nothing of what the second did"
    );
    assert_eq!(
        dropped(&second.exported_wide()[0]),
        count(RECORDS_PER_CAPTURED_RUN)
    );
}

/// A destination that doesn't answer holds a flush for no longer than the
/// bound it's given, and holds no other destination at all: the file is
/// whole, wide event and all, while the network is stuck.
#[tokio::test]
async fn a_destination_that_does_not_answer_holds_a_flush_only_for_its_bound_and_no_other() {
    let brief = Duration::from_millis(100);
    let (file, network) = (Memory::default(), Memory::default());
    let telemetry = Telemetry::builder(VERSION, scope())
        .exporting_over_the_network_to(network.spans(), network.records(), network.wide())
        .exporting_to(file.spans(), file.records(), file.wide())
        .flush_timeout(brief)
        .build()
        .unwrap();
    network.hold();

    let began = Instant::now();
    let flushed = run(&telemetry, RUN, Records::Captured).await;
    let waited = began.elapsed();

    assert_eq!(
        flushed.unwrap_err().failures(),
        ["otlp: the flush didn't end within 100ms"],
        "the network destination alone is reported, by its bound"
    );
    assert!(waited >= brief, "the flush gave up after {waited:?}");
    // A flush that waited on a queue, in place of the bound it was given,
    // costs at least the SDK's five seconds, which the tolerance rules out.
    assert!(
        waited < brief + Duration::from_secs(1),
        "the flush waited {waited:?}, past the bound it was given"
    );
    assert_eq!(file.exported_spans().len(), SPANS_PER_RUN);
    assert_eq!(file.exported_records().len(), RECORDS_PER_CAPTURED_RUN);
    assert_eq!(events_of(&file.exported_wide()), [WIDE]);
    assert_eq!(dropped(&file.exported_wide()[0]), AnyValue::Int(0));
    assert!(network.exports().is_empty());

    // Once the destination answers, the thread the flush left makes the
    // run's wide event, and the shutdown waits for that thread before it
    // stops the queues, so the event is exported however late it's made.
    network.release();
    telemetry.shutdown().await.unwrap();
    assert_eq!(network.exported_spans().len(), SPANS_PER_RUN);
    assert_eq!(events_of(&network.exported_wide()), [WIDE]);
    assert_eq!(dropped(&network.exported_wide()[0]), AnyValue::Int(0));
}

#[tokio::test]
async fn a_flush_with_a_bound_of_nothing_still_answers() {
    let (memory, other) = (Memory::default(), Memory::default());
    let telemetry = Telemetry::builder(VERSION, scope())
        .exporting_to(memory.spans(), memory.records(), memory.wide())
        .exporting_over_the_network_to(other.spans(), other.records(), other.wide())
        .flush_timeout(Duration::ZERO)
        .build()
        .unwrap();

    let flushed = run(&telemetry, RUN, Records::Exception).await;

    // A bound of nothing gives up on whichever destination hasn't answered
    // by the time it's checked, and never on one that has.
    if let Err(failures) = flushed {
        for failure in failures.failures() {
            assert!(
                failure.ends_with("the flush didn't end within 0ns"),
                "{failures}"
            );
        }
    }
    telemetry.shutdown().await.unwrap();
    assert_eq!(memory.exported_wide().len(), 1);
    assert_eq!(other.exported_wide().len(), 1);
}

#[tokio::test]
async fn a_telemetry_with_no_destination_takes_a_run_and_a_flush_returns_at_once() {
    let telemetry = Telemetry::builder(VERSION, scope()).build().unwrap();

    let began = Instant::now();
    let flushed = run(&telemetry, RUN, Records::Captured).await;
    let shut = telemetry.shutdown().await;

    assert_eq!(flushed, Ok(()));
    assert_eq!(shut, Ok(()));
    assert!(began.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn a_span_is_taken_while_an_export_waits_for_its_destination() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.hold();

    let wides = tokio::time::timeout(Duration::from_secs(5), async {
        [RUN, OTHER_RUN, RUN].map(|run| emit_run(&telemetry, run, Records::Captured))
    })
    .await;
    memory.release();
    let wides = wides.expect("the loop's side never waits for an export");
    for wide in wides {
        telemetry.flush(wide).await.unwrap();
    }

    assert_eq!(memory.exported_spans().len(), 3 * SPANS_PER_RUN);
    assert_eq!(memory.exported_wide().len(), 3);
}

// The builder

#[test]
fn a_builder_prints_what_it_was_given_and_gives_a_flush_and_a_shutdown_five_seconds_by_default() {
    let builder = Telemetry::builder("0.1.0", scope())
        .resource(vec![("team".to_owned(), "evals".to_owned())])
        .file(FileTarget::Stderr);

    let shown = format!("{builder:?}");

    assert!(
        shown.starts_with("TelemetryBuilder { version: \"0.1.0\", scope: InstrumentationScope {"),
        "{shown}"
    );
    assert!(
        shown.ends_with(
            "resource: [(\"team\", \"evals\")], file: Some(Stderr), otlp: None, \
             flush_timeout: 5s, shutdown_timeout: 5s, .. }"
        ),
        "{shown}"
    );
}

#[test]
fn a_builder_prints_neither_the_endpoint_nor_a_header_value_it_was_given() {
    let builder = Telemetry::builder("0.1.0", scope()).otlp(OtlpSettings {
        transport: Transport::HttpProtobuf,
        endpoint: Some("http://user:hunter2hunter2@collector:4318".to_owned()),
        headers: vec![(
            "authorization".to_owned(),
            "Bearer hunter2hunter2".to_owned(),
        )],
        strip_environment_headers: true,
    });

    let shown = format!("{builder:?}");

    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(
        shown.contains(
            "otlp: Some(OtlpSettings { transport: HttpProtobuf, endpoint: Some(\"..\"), \
             headers: [\"authorization\"], strip_environment_headers: true })"
        ),
        "{shown}"
    );
}

#[tokio::test]
async fn a_header_that_is_not_one_fails_the_build_and_names_the_header() {
    let refused = Telemetry::builder("0.1.0", scope())
        .otlp(OtlpSettings {
            transport: Transport::Grpc,
            endpoint: Some("http://127.0.0.1:1".to_owned()),
            headers: vec![("no spaces allowed".to_owned(), "v".to_owned())],
            strip_environment_headers: true,
        })
        .build()
        .err()
        .unwrap();

    assert_eq!(
        refused,
        OtelBuildError::Header {
            name: "no spaces allowed".to_owned(),
            reason: "its name isn't one a header may have",
        }
    );
}
