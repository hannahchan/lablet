use std::time::{Duration, Instant};

use lablet_conformance::otlp::Exported;
use lablet_run::telemetry::Record;
use opentelemetry::logs::{AnyValue, Severity};
use opentelemetry::trace::{
    Span as _, SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
    Tracer as _,
};
use opentelemetry::{Context, Key, KeyValue, Value};
use opentelemetry_sdk::trace::SpanData;

use super::*;
use crate::export::network::OtelBuildError;
use crate::export::resource;
use crate::export::testing::memory::{Logged, Memory};
use crate::export::testing::{
    CONTENT, DURATION_MS, EXCEPTION, OTHER_RUN, RECORDS_PER_CAPTURED_RUN, RECORDS_PER_RUN, RUN,
    RUN_KEY, Records, SCHEMA_URL, SPANS_PER_RUN, Scratch, VERSION, WIDE, after, emit_run, otlp_of,
    scope, sdk_of, wide_event,
};
use crate::otel_env;

fn exporting(memory: &Memory) -> Telemetry {
    Telemetry::builder(scope())
        .exporting_to(memory.spans(), memory.records())
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

/// The providers each failure of `error` names.
fn providers(error: &FlushError) -> Vec<&str> {
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

/// The specification's defaults, which an environment that sets no limit
/// gives, so a span the loop fills never meets them.
#[tokio::test]
async fn a_span_keeps_128_attributes_and_128_events_when_the_environment_sets_no_limit() {
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

/// The processors wait an hour between exports of their own, so what a
/// destination holds when the flush returns is what the flush exported.
#[tokio::test]
async fn the_wide_event_is_emitted_once_and_flushed_with_the_run() {
    let (first, second) = (Memory::default(), Memory::default());
    let hour = "3600000";
    let telemetry = Telemetry::builder(scope())
        .sdk(sdk_of(&[
            ("OTEL_BSP_SCHEDULE_DELAY", hour),
            ("OTEL_BLRP_SCHEDULE_DELAY", hour),
        ]))
        .exporting_to(first.spans(), first.records())
        .exporting_to(second.spans(), second.records())
        .build()
        .unwrap();

    run(&telemetry, RUN, Records::Captured).await.unwrap();

    for memory in [&first, &second] {
        let wide = memory.exported_wide();
        assert_eq!(events_of(&wide), [WIDE], "one wide event, by the flush");
        assert_eq!(held(&wide[0], RUN_KEY), RUN.into());
        assert_eq!(memory.exported_spans().len(), SPANS_PER_RUN);
        assert_eq!(memory.exported_records().len(), RECORDS_PER_CAPTURED_RUN);
    }
    telemetry.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_wide_event_is_the_record_handed_over_timed_and_placed_as_it_says() {
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
}

#[tokio::test]
async fn a_flush_of_a_run_that_emitted_nothing_exports_its_wide_event() {
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

#[tokio::test]
async fn a_flush_says_what_the_sdk_said_of_an_export_that_failed() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.refuse_records(true);

    let flushed = run(&telemetry, RUN, Records::Captured).await;

    let failures = flushed.unwrap_err();
    assert_eq!(providers(&failures), ["log records"], "{failures}");
    assert!(
        failures
            .to_string()
            .starts_with("telemetry wasn't exported whole: log records: "),
        "{failures}"
    );
    assert!(
        failures
            .to_string()
            .contains("the destination can't be written"),
        "the exporter's own error, as the SDK hands it on: {failures}"
    );
    assert!(memory.exported_records().is_empty());
    assert!(memory.exported_wide().is_empty());
    assert_eq!(memory.exported_spans().len(), SPANS_PER_RUN);
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
        providers(&failures),
        ["spans", "log records"],
        "each provider says that its processors have stopped: {failures}"
    );
    assert_eq!(providers(&again.unwrap_err()), ["spans", "log records"]);
}

/// What a processor fails to export as it stops, the SDK says on its own
/// diagnostic log; the stop is what's held here.
#[tokio::test]
async fn a_shutdown_stops_every_processor_even_one_that_cannot_export() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.refuse_spans(true);
    let _wide = emit_run(&telemetry, RUN, Records::Exception);

    let _ = telemetry.shutdown().await;

    assert!(memory.exported_spans().is_empty());
    assert_eq!(memory.exported_records().len(), RECORDS_PER_RUN);
    let after = telemetry
        .flush(wide_event(RUN, SpanContext::empty_context()))
        .await;
    assert_eq!(
        providers(&after.unwrap_err()),
        ["spans", "log records"],
        "every processor stopped"
    );
}

/// The SDK gives each processor five seconds to stop, so a destination
/// that never answers costs a shutdown that much once, since the two
/// providers are shut down side by side, and not once for each.
#[tokio::test]
async fn a_shutdown_with_a_destination_that_never_answers_returns_within_five_seconds() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);
    memory.hold();
    let _wide = emit_run(&telemetry, RUN, Records::Exception);

    let began = Instant::now();
    let shut = telemetry.shutdown().await;
    let waited = began.elapsed();
    memory.release();

    assert_eq!(providers(&shut.unwrap_err()), ["spans", "log records"]);
    assert!(
        waited < Duration::from_secs(5) + Duration::from_millis(1_500),
        "the shutdown waited {waited:?}"
    );
}

/// The SDK's providers stop their processors when they're dropped, each for
/// up to five seconds of the SDK's. After a shutdown the providers are
/// marked as shut down, so the drop costs nothing and stops nothing again;
/// without one, the SDK's shutdown on the drop is what exports what the
/// processors held.
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
async fn every_exporter_is_told_the_sdk_and_what_the_composer_added_its_service_name_among_it() {
    let memory = Memory::default();

    let telemetry = Telemetry::builder(scope())
        .resource(resource(
            VERSION,
            vec![
                ("team".to_owned(), "evals".to_owned()),
                ("service.name".to_owned(), "not-lablet".to_owned()),
                ("deployment.environment.name".to_owned(), "ci".to_owned()),
            ],
            &otel_env::Context::default(),
        ))
        .exporting_to(memory.spans(), memory.records())
        .build()
        .unwrap();
    // A processor hands its exporter the resource from its own thread, which
    // has done so by the time it has stopped.
    telemetry.shutdown().await.unwrap();

    let resources = memory.resources();
    assert_eq!(resources.len(), 2, "the two exporters of one destination");
    for resource in resources {
        let said = |key: &'static str| resource.get(&Key::new(key));
        assert_eq!(said("service.name"), Some(Value::from("not-lablet")));
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
}

// Where a run is exported to

#[tokio::test]
async fn every_destination_is_handed_every_span_and_every_record() {
    let (first, second) = (Memory::default(), Memory::default());
    let telemetry = Telemetry::builder(scope())
        .exporting_to(first.spans(), first.records())
        .exporting_to(second.spans(), second.records())
        .build()
        .unwrap();
    second.refuse_records(true);

    let flushed = run(&telemetry, RUN, Records::Captured).await;

    let failures = flushed.unwrap_err();
    assert_eq!(providers(&failures), ["log records"], "{failures}");
    assert_eq!(first.exported_spans(), second.exported_spans());
    assert_eq!(first.exported_spans().len(), SPANS_PER_RUN);
    assert_eq!(first.exported_records().len(), RECORDS_PER_CAPTURED_RUN);
    assert_eq!(events_of(&first.exported_wide()), [WIDE]);
    assert!(
        second.exported_records().is_empty() && second.exported_wide().is_empty(),
        "the second refused its records, and the first has them whole"
    );
}

/// A provider flushes its processors in the order they were added, and the
/// file's are added first, so the file is whole, wide event and all, while
/// the destination added after it is still held. The SDK gives the held
/// one's flush five seconds, and the two providers wait for theirs side by
/// side.
#[tokio::test]
async fn a_run_end_flushes_the_file_before_a_destination_that_is_held() {
    let scratch = Scratch::new("held-after-the-file");
    let path = scratch.at("runs.otlp.jsonl");
    let held = Memory::default();
    let telemetry = Telemetry::builder(scope())
        .file(FileTarget::Path(path.clone()))
        .exporting_to(held.spans(), held.records())
        .build()
        .unwrap();
    held.hold();
    let wide = emit_run(&telemetry, RUN, Records::Captured);

    let began = Instant::now();
    let flushing = tokio::spawn({
        let telemetry = telemetry.clone();
        async move { telemetry.flush(wide).await }
    });
    let whole = loop {
        let read = Exported::read(&path).ok();
        if let Some(read) = read.filter(|read| {
            read.records_of(WIDE).len() == 1
                && read.spans.len() == SPANS_PER_RUN
                && read.records.len() == RECORDS_PER_CAPTURED_RUN + 1
        }) {
            break Some(read);
        }
        if began.elapsed() > Duration::from_secs(1) {
            break None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let file_whole_after = began.elapsed();
    assert!(
        whole.is_some(),
        "the file wasn't whole within a second of the flush beginning"
    );
    assert!(
        !flushing.is_finished(),
        "the flush was still waiting on the held destination"
    );
    assert!(held.exports().is_empty());

    let flushed = flushing.await.unwrap();
    let waited = began.elapsed();
    held.release();
    assert_eq!(providers(&flushed.unwrap_err()), ["spans", "log records"]);
    assert!(
        waited < Duration::from_secs(5) + Duration::from_millis(1_500),
        "the flush waited {waited:?}, with the file whole after {file_whole_after:?}"
    );
    telemetry.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_telemetry_with_no_destination_takes_a_run_and_a_flush_returns_at_once() {
    let telemetry = Telemetry::builder(scope()).build().unwrap();

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

#[tokio::test]
async fn a_record_emitted_through_the_logger_is_exported_by_the_next_flush() {
    let memory = Memory::default();
    let telemetry = exporting(&memory);

    telemetry.logger().emit(Record {
        name: EXCEPTION,
        severity: Severity::Warn,
        at: after(1),
        span: SpanContext::empty_context(),
        attributes: Vec::new(),
    });
    telemetry.flush_leftovers().await.unwrap();

    assert_eq!(events_of(&memory.exported_records()), [EXCEPTION]);
    assert!(memory.exported_wide().is_empty());
}

// The builder

#[test]
fn a_builder_prints_what_it_was_given() {
    let builder = Telemetry::builder(scope()).file(FileTarget::Stderr);

    let shown = format!("{builder:?}");

    assert!(
        shown.starts_with("TelemetryBuilder { scope: InstrumentationScope {"),
        "{shown}"
    );
    assert!(
        shown.contains("file: Some(Stderr), otlp: None, sdk: Sdk { sampling: Sampling {"),
        "{shown}"
    );
    assert!(shown.ends_with(" .. }"), "{shown}");
}

#[test]
fn a_builder_prints_neither_the_endpoint_nor_a_header_value_it_was_given() {
    let builder = Telemetry::builder(scope()).otlp(otlp_of(
        "telemetry: { otlp: { endpoint: 'http://user:hunter2hunter2@collector:4318', headers: \
         { authorization: Bearer hunter2hunter2 } } }",
        &[],
    ));

    let shown = format!("{builder:?}");

    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!shown.contains("collector"), "{shown}");
    assert!(shown.contains("headers: [\"authorization\"]"), "{shown}");
}

#[tokio::test]
async fn a_header_that_is_not_one_fails_the_build_and_names_the_header() {
    let refused = Telemetry::builder(scope())
        .otlp(otlp_of(
            "telemetry: { otlp: { headers: { no spaces allowed: v } } }",
            &[],
        ))
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

// OTEL_SDK_DISABLED: the API's no-ops

#[tokio::test]
async fn with_the_sdk_disabled_a_span_carries_its_parents_context_and_no_id_of_its_own() {
    let memory = Memory::default();
    let scratch = Scratch::new("disabled");
    let path = scratch.at("runs.otlp.jsonl");
    let telemetry = Telemetry::builder(scope())
        .disabled()
        .file(FileTarget::Path(path.clone()))
        .exporting_to(memory.spans(), memory.records())
        .build()
        .unwrap();
    let inbound = SpanContext::new(
        TraceId::from_bytes([7; 16]),
        SpanId::from_bytes([8; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::from_key_value([("vendor", "value")]).unwrap(),
    );
    let parent = Context::new().with_remote_span_context(inbound.clone());

    let tracer = telemetry.tracer();
    let span = tracer
        .span_builder("invoke_agent lablet")
        .start_with_context(&tracer, &parent);
    let within = parent.with_span(span);
    let child = tracer.start_with_context("chat scripted-1", &within);
    let wide = emit_run(&telemetry, RUN, Records::Captured);
    let flushed = telemetry.flush(wide).await;
    let shut = telemetry.shutdown().await;

    assert_eq!(within.span().span_context(), &inbound);
    assert_eq!(child.span_context(), &inbound);
    assert_eq!((flushed, shut), (Ok(()), Ok(())));
    assert!(memory.exports().is_empty(), "nothing reached a destination");
    assert!(!path.exists(), "no file was written");
}
