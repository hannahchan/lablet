use std::time::{Instant, UNIX_EPOCH};

use lablet_model::{
    ContentBlock, ProviderErrorKind, RunContext, RunSummary, StopReason, ToolCallEnd,
    ToolCallStatus, ToolResultContent, ToolSource,
};
use lablet_run::ProviderError;
use lablet_telemetry_registry::signals::{EVENT_LABLET_RUN_KEYS, EVENT_LABLET_RUN_NAME};
use opentelemetry::logs::{AnyValue, Severity};
use opentelemetry::trace::SpanId;
use opentelemetry::{Key, Value};
use opentelemetry_sdk::trace::SpanData;
use serde_json::json;

use super::*;
use crate::network::{OtelBuildError, OtlpSettings, Transport};
use crate::pipeline::QUEUE_CAPACITY;
use crate::testing::memory::{Export, Logged, Memory};
use crate::testing::{
    CONFIG_DIGEST, DURATION_MS, RUN, STARTED_UNIX_MS, call_id, calls, capturing, completed,
    context, opening, record, run_id, said, stopped, tool_name, two_counts,
};

const OTHER_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";

fn observing(memory: &Memory) -> OtelObserver {
    OtelObserver::builder("0.1.0")
        .exporting_to(memory.spans(), memory.records(), memory.wide())
        .build()
        .unwrap()
}

fn of(run: &RunId, kind: EventKind) -> RunEvent {
    RunEvent {
        run_id: run.clone(),
        kind,
    }
}

fn started(opening: Opening) -> RunEvent {
    let Opening {
        context,
        model,
        endpoint,
        request,
        tools,
        system_prompt,
        prompt,
    } = opening;
    of(
        &context.run_id.clone(),
        EventKind::RunStarted {
            context: Box::new(context),
            model,
            endpoint,
            request,
            tools,
            system_prompt,
            prompt,
        },
    )
}

fn finished(context: RunContext, summary: RunSummary) -> RunEvent {
    of(
        &context.run_id.clone(),
        EventKind::RunFinished {
            context: Box::new(context),
            summary: Box::new(summary),
        },
    )
}

fn attempt_began(turn: u32, attempt: u32) -> RunEvent {
    of(
        &run_id(),
        EventKind::ProviderCallStarted {
            turn,
            attempt,
            request_bytes: 48_211,
        },
    )
}

fn attempt_answered(turn: u32, attempt: u32, response: Option<Vec<ContentBlock>>) -> RunEvent {
    of(
        &run_id(),
        EventKind::ProviderCallFinished {
            turn,
            attempt,
            record: Box::new(record(two_counts(), 2_047, 250)),
            response,
        },
    )
}

fn attempt_failed(turn: u32, attempt: u32) -> RunEvent {
    of(
        &run_id(),
        EventKind::ProviderCallFailed {
            turn,
            attempt,
            error: ProviderError::new(ProviderErrorKind::Retryable, "529 overloaded"),
            started_ms: 7,
            latency_ms: 40,
            retry: Some(Duration::from_secs(2)),
        },
    )
}

fn call_began(turn: u32, call: &str, captured: bool) -> RunEvent {
    of(
        &run_id(),
        EventKind::ToolCallStarted {
            turn,
            call_id: call_id(call),
            name: tool_name("bash"),
            source: Some(ToolSource::Builtin),
            input_bytes: 24,
            input: captured.then(|| json!({ "command": "cargo test" })),
        },
    )
}

fn call_ended(turn: u32, call: &str, captured: bool) -> RunEvent {
    of(
        &run_id(),
        EventKind::ToolCallFinished {
            turn,
            call_id: call_id(call),
            status: ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok),
            started_ms: 2_300,
            latency_ms: 1_000,
            output_bytes: 8,
            truncated_from_bytes: None,
            mcp: None,
            output: captured.then(|| vec![ToolResultContent::Text("2 passed".to_owned())]),
        },
    )
}

/// A run of one turn, whose call failed once, answered with a call to
/// `bash`, and then the run was cancelled. With `captured` it's a run that
/// captures content, and its events hold it.
fn a_run(captured: bool) -> Vec<RunEvent> {
    let opening = if captured { capturing() } else { opening() };
    let context = opening.context.clone();
    let response = captured.then(|| vec![said("On it."), calls("call_1", "bash")]);
    vec![
        started(opening),
        of(&run_id(), EventKind::TurnStarted { turn: 1 }),
        attempt_began(1, 1),
        attempt_failed(1, 1),
        attempt_began(1, 2),
        attempt_answered(1, 2, response),
        call_began(1, "call_1", captured),
        call_ended(1, "call_1", captured),
        finished(context.clone(), stopped(&context, StopReason::Cancelled)),
    ]
}

async fn told(observer: &OtelObserver, events: Vec<RunEvent>) {
    for event in events {
        observer.on(event).await;
    }
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
    held(record, key::LABLET_TELEMETRY_DROPPED_RECORDS)
}

// What a run is exported as

#[tokio::test]
async fn a_run_is_exported_as_its_spans_its_records_and_its_wide_event() {
    let memory = Memory::default();
    let observer = observing(&memory);

    told(&observer, a_run(true)).await;
    let flushed = observer.flush().await;

    assert_eq!(flushed, Ok(()));
    let spans = memory.exported_spans();
    assert_eq!(
        names(&spans),
        [
            "chat scripted-1",
            "chat scripted-1",
            "execute_tool bash",
            "invoke_agent lablet"
        ],
        "each span is handed over as it ends"
    );
    let details = "gen_ai.client.inference.operation.details";
    assert_eq!(
        events_of(&memory.exported_records()),
        [
            details,
            "gen_ai.client.operation.exception",
            details,
            details,
            details
        ]
    );
    assert_eq!(events_of(&memory.exported_wide()), ["lablet.run"]);
}

#[tokio::test]
async fn a_run_that_captures_no_content_is_exported_without_any() {
    let memory = Memory::default();
    let observer = observing(&memory);

    told(&observer, a_run(false)).await;
    observer.flush().await.unwrap();

    assert_eq!(memory.exported_spans().len(), 4);
    assert_eq!(
        events_of(&memory.exported_records()),
        ["gen_ai.client.operation.exception"]
    );
    assert_eq!(events_of(&memory.exported_wide()), ["lablet.run"]);
}

#[tokio::test]
async fn the_signals_of_a_run_are_one_trace_beneath_the_root_span() {
    let memory = Memory::default();
    let observer = observing(&memory);

    told(&observer, a_run(true)).await;
    observer.flush().await.unwrap();

    let spans = memory.exported_spans();
    let root = spans.last().unwrap();
    let trace = root.span_context.trace_id();
    assert_eq!(root.parent_span_id, SpanId::INVALID);
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
    assert_eq!(
        memory.exported_records()[3]
            .0
            .trace_context()
            .unwrap()
            .span_id,
        ids[1]
    );
}

#[tokio::test]
async fn two_runs_are_two_traces() {
    let memory = Memory::default();
    let observer = observing(&memory);

    told(&observer, a_run(false)).await;
    told(&observer, a_run(false)).await;
    observer.flush().await.unwrap();

    let spans = memory.exported_spans();
    assert_eq!(spans.len(), 8);
    assert_ne!(
        spans[3].span_context.trace_id(),
        spans[7].span_context.trace_id()
    );
    let wide = memory.exported_wide();
    let of_the_runs: Vec<_> = wide
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

// The wide event

#[tokio::test]
async fn the_wide_event_is_exported_alone_and_after_everything_else_of_its_run() {
    let memory = Memory::default();
    let observer = observing(&memory);

    told(&observer, a_run(true)).await;
    observer.flush().await.unwrap();

    let exports = memory.exports();
    let Some(Export::Wide(last)) = exports.last() else {
        panic!("the last export is the wide event's: {exports:?}");
    };
    assert_eq!(events_of(last), ["lablet.run"]);
    let others = &exports[..exports.len() - 1];
    assert!(
        others
            .iter()
            .all(|export| matches!(export, Export::Spans(_) | Export::Records(_))),
        "the wide event has one export: {exports:?}"
    );
    assert!(
        others.iter().all(|export| match export {
            Export::Records(records) => !events_of(records).contains(&EVENT_LABLET_RUN_NAME),
            Export::Spans(_) | Export::Wide(_) => true,
        }),
        "and it's in no other"
    );
}

#[tokio::test]
async fn the_wide_event_is_the_run_s_record_timed_at_the_run_s_end() {
    let memory = Memory::default();
    let observer = observing(&memory);

    told(&observer, a_run(false)).await;
    observer.flush().await.unwrap();

    let wide = memory.exported_wide();
    let (record, _) = &wide[0];
    let end = UNIX_EPOCH + Duration::from_millis(STARTED_UNIX_MS + DURATION_MS);
    assert_eq!(record.event_name(), Some(EVENT_LABLET_RUN_NAME));
    assert_eq!(record.severity_number(), Some(Severity::Info));
    assert_eq!(record.timestamp(), Some(end));
    assert_eq!(record.observed_timestamp(), Some(end));
    assert_eq!(held(&wide[0], key::GEN_AI_CONVERSATION_ID), RUN.into());
    assert_eq!(held(&wide[0], key::SESSION_ID), RUN.into());
    assert_eq!(
        held(&wide[0], key::LABLET_CONFIG_DIGEST),
        CONFIG_DIGEST.into()
    );
    assert_eq!(dropped(&wide[0]), AnyValue::Int(0));
    let keys: Vec<_> = record
        .attributes_iter()
        .map(|(key, _)| key.as_str())
        .collect();
    assert!(
        keys.iter().all(|key| EVENT_LABLET_RUN_KEYS.contains(key)),
        "{keys:?}"
    );
}

#[tokio::test]
async fn the_wide_event_counts_what_the_exporters_lost_of_its_run() {
    let memory = Memory::default();
    let observer = observing(&memory);
    memory.refuse_records(true);

    told(&observer, a_run(true)).await;
    let flushed = observer.flush().await;

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
    assert_eq!(memory.exported_spans().len(), 4);
    let wide = memory.exported_wide();
    assert_eq!(
        dropped(&wide[0]),
        AnyValue::Int(5),
        "the five records of the run, which the flush itself lost"
    );
}

#[tokio::test]
async fn the_wide_event_counts_the_spans_and_the_records_together() {
    let memory = Memory::default();
    let observer = observing(&memory);
    memory.refuse_records(true);
    memory.refuse_spans(true);

    told(&observer, a_run(false)).await;
    let flushed = observer.flush().await;

    let failures = flushed.unwrap_err();
    let queues: Vec<_> = failures
        .failures()
        .iter()
        .map(|failure| failure.split(':').next().unwrap())
        .collect();
    assert_eq!(queues, ["spans", "log records"]);
    assert_eq!(dropped(&memory.exported_wide()[0]), AnyValue::Int(4 + 1));
}

#[tokio::test]
async fn the_wide_event_counts_what_a_full_queue_turned_away() {
    let memory = Memory::default();
    let observer = observing(&memory);
    memory.hold();

    observer.on(started(opening())).await;
    let attempts = u32::try_from(QUEUE_CAPACITY).unwrap() + 3;
    for attempt in 1..=attempts {
        told(
            &observer,
            vec![attempt_began(1, attempt), attempt_failed(1, attempt)],
        )
        .await;
    }
    observer
        .on(finished(
            context(),
            stopped(&context(), StopReason::Timeout),
        ))
        .await;
    memory.release();
    observer.flush().await.unwrap();

    assert_eq!(memory.exported_spans().len(), QUEUE_CAPACITY);
    assert_eq!(memory.exported_records().len(), QUEUE_CAPACITY);
    assert_eq!(
        dropped(&memory.exported_wide()[0]),
        AnyValue::Int(3 + 1 + 3),
        "three chat spans and the root span, and three exception records"
    );
}

#[tokio::test]
async fn a_run_s_wide_event_counts_nothing_of_the_run_before_it() {
    let memory = Memory::default();
    let observer = observing(&memory);
    memory.refuse_spans(true);
    told(&observer, a_run(false)).await;
    observer.flush().await.unwrap_err();
    memory.refuse_spans(false);

    told(&observer, a_run(false)).await;
    let flushed = observer.flush().await;

    assert_eq!(flushed, Ok(()));
    let lost: Vec<_> = memory.exported_wide().iter().map(dropped).collect();
    assert_eq!(lost, [AnyValue::Int(4), AnyValue::Int(0)]);
}

#[tokio::test]
async fn a_wide_event_that_was_lost_is_reported_and_is_counted_against_no_run() {
    let memory = Memory::default();
    let observer = observing(&memory);
    memory.refuse_wide(true);
    told(&observer, a_run(false)).await;

    let lost = observer.flush().await;
    memory.refuse_wide(false);
    told(&observer, a_run(false)).await;
    let kept = observer.flush().await;

    let lost = lost.unwrap_err();
    assert_eq!(lost.failures().len(), 1, "{lost}");
    assert!(lost.failures()[0].starts_with("the wide event: "), "{lost}");
    assert_eq!(kept, Ok(()));
    let wide = memory.exported_wide();
    assert_eq!(wide.len(), 1, "a wide event is made once");
    assert_eq!(dropped(&wide[0]), AnyValue::Int(0));
}

#[tokio::test]
async fn a_flush_with_nothing_to_export_exports_nothing() {
    let memory = Memory::default();
    let observer = observing(&memory);

    let before = observer.flush().await;
    told(&observer, a_run(false)).await;
    observer.flush().await.unwrap();
    let exports = memory.exports().len();
    let again = observer.flush().await;

    assert_eq!(before, Ok(()));
    assert_eq!(again, Ok(()));
    assert_eq!(memory.exports().len(), exports, "a wide event is made once");
}

#[tokio::test]
async fn a_run_in_progress_is_flushed_without_a_wide_event() {
    let memory = Memory::default();
    let observer = observing(&memory);
    let mut events = a_run(false);
    let last = events.pop().unwrap();

    told(&observer, events).await;
    observer.flush().await.unwrap();
    let during = (memory.exported_spans().len(), memory.exported_wide().len());
    observer.on(last).await;
    observer.flush().await.unwrap();

    assert_eq!(during, (3, 0));
    assert_eq!(names(&memory.exported_spans())[3], "invoke_agent lablet");
    assert_eq!(memory.exported_wide().len(), 1);
}

// Shutting down

#[tokio::test]
async fn a_run_nobody_flushed_is_exported_when_the_observer_shuts_down() {
    let memory = Memory::default();
    let observer = observing(&memory);
    told(&observer, a_run(true)).await;

    let shut = observer.shutdown().await;

    assert_eq!(shut, Ok(()));
    assert_eq!(memory.exported_spans().len(), 4);
    assert_eq!(memory.exported_records().len(), 5);
    assert_eq!(events_of(&memory.exported_wide()), ["lablet.run"]);
    assert!(matches!(memory.exports().last(), Some(Export::Wide(_))));
}

#[tokio::test]
async fn an_observer_that_was_shut_down_exports_nothing_more_and_says_so() {
    let memory = Memory::default();
    let observer = observing(&memory);
    observer.shutdown().await.unwrap();

    told(&observer, a_run(true)).await;
    let flushed = observer.flush().await;
    let again = observer.shutdown().await;

    assert!(memory.exports().is_empty());
    let failures = flushed.unwrap_err();
    assert_eq!(
        failures.failures().len(),
        3,
        "each queue says that it has stopped: {failures}"
    );
    assert!(again.is_err());
}

#[tokio::test]
async fn a_queue_that_cannot_export_fails_the_shutdown_and_stops_all_the_same() {
    let memory = Memory::default();
    let observer = observing(&memory);
    memory.refuse_spans(true);
    told(&observer, a_run(false)).await;

    let shut = observer.shutdown().await;

    let failures = shut.unwrap_err();
    assert!(failures.failures()[0].starts_with("spans: "), "{failures}");
    assert_eq!(events_of(&memory.exported_wide()), ["lablet.run"]);
    assert_eq!(dropped(&memory.exported_wide()[0]), AnyValue::Int(4));
}

/// A destination that doesn't answer holds a shutdown for no longer than
/// the timeout it's given, which is far less than the five seconds the SDK
/// waits for a flush. What it bounds is the export that never ends.
#[tokio::test]
async fn a_destination_that_does_not_answer_holds_a_shutdown_only_for_its_timeout() {
    let brief = Duration::from_millis(100);
    let memory = Memory::default();
    let observer = OtelObserver::builder("0.1.0")
        .exporting_to(memory.spans(), memory.records(), memory.wide())
        .shutdown_timeout(brief)
        .build()
        .unwrap();
    memory.hold();
    told(&observer, a_run(false)).await;

    let began = Instant::now();
    let shut = observer.shutdown().await;
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

// Events the observer makes nothing of

#[tokio::test]
async fn nothing_comes_of_the_events_of_a_run_that_never_started() {
    let memory = Memory::default();
    let observer = observing(&memory);
    let mut events = a_run(true);
    events.remove(0);

    told(&observer, events).await;
    let propagated = observer.trace_context(&call_id("call_1"));
    observer.flush().await.unwrap();

    assert_eq!(propagated, None);
    assert!(memory.exports().is_empty(), "{:?}", memory.exports());
}

#[tokio::test]
async fn nothing_comes_of_the_events_of_a_run_other_than_the_one_in_progress() {
    let memory = Memory::default();
    let observer = observing(&memory);
    let other = RunId::new(OTHER_RUN).unwrap();
    let of_the_other = |event: RunEvent| RunEvent {
        run_id: other.clone(),
        ..event
    };
    observer.on(started(opening())).await;

    told(
        &observer,
        a_run(true).into_iter().skip(1).map(of_the_other).collect(),
    )
    .await;
    observer.flush().await.unwrap();
    let of_the_other_run = memory.exports().len();
    told(&observer, a_run(false).into_iter().skip(1).collect()).await;
    observer.flush().await.unwrap();

    assert_eq!(of_the_other_run, 0);
    assert_eq!(
        memory.exported_spans().len(),
        4,
        "the run in progress went on, and its events were its own"
    );
    assert_eq!(memory.exported_wide().len(), 1);
}

#[tokio::test]
async fn a_run_that_starts_takes_the_place_of_one_that_never_ended() {
    let memory = Memory::default();
    let observer = observing(&memory);
    let mut abandoned = a_run(false);
    abandoned.truncate(7);

    told(&observer, abandoned).await;
    let propagated = observer.trace_context(&call_id("call_1"));
    observer.on(started(opening())).await;
    let forgotten = observer.trace_context(&call_id("call_1"));
    told(
        &observer,
        vec![
            call_ended(1, "call_1", false),
            finished(context(), completed(&context(), two_counts(), None)),
        ],
    )
    .await;
    observer.flush().await.unwrap();

    assert!(propagated.is_some());
    assert_eq!(forgotten, None);
    assert_eq!(
        names(&memory.exported_spans()),
        ["chat scripted-1", "chat scripted-1", "invoke_agent lablet"],
        "the call the first run left open has no span, and the first run no root"
    );
    assert_eq!(memory.exported_wide().len(), 1);
    let root = &memory.exported_spans()[2];
    assert_ne!(
        root.span_context.trace_id(),
        memory.exported_spans()[0].span_context.trace_id()
    );
}

// The span an executor propagates

#[tokio::test]
async fn the_observer_answers_with_the_span_it_opened_for_a_call() {
    let memory = Memory::default();
    let observer = observing(&memory);
    told(&observer, a_run(false).into_iter().take(6).collect()).await;

    let before = observer.trace_context(&call_id("call_1"));
    observer.on(call_began(1, "call_1", false)).await;
    let during = observer.trace_context(&call_id("call_1"));
    let of_another = observer.trace_context(&call_id("call_2"));
    observer.on(call_ended(1, "call_1", false)).await;
    let after = observer.trace_context(&call_id("call_1"));
    observer.flush().await.unwrap();

    assert_eq!(before, None);
    assert_eq!(of_another, None);
    assert_eq!(after, None);
    let tool = &memory.exported_spans()[2];
    assert_eq!(tool.name, "execute_tool bash");
    assert_eq!(
        during,
        Some(TraceContext {
            traceparent: format!(
                "00-{}-{}-01",
                tool.span_context.trace_id(),
                tool.span_context.span_id()
            ),
            tracestate: None,
        })
    );
}

// What an export says of where it came from

#[tokio::test]
async fn every_signal_is_of_the_scope_that_names_the_registry_s_schema() {
    let memory = Memory::default();
    let observer = OtelObserver::builder("0.4.2")
        .exporting_to(memory.spans(), memory.records(), memory.wide())
        .build()
        .unwrap();

    told(&observer, a_run(true)).await;
    observer.flush().await.unwrap();

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
    assert_eq!(scopes.len(), 4 + 5 + 1);
    for scope in scopes {
        assert_eq!(scope.name(), "lablet");
        assert_eq!(scope.version(), Some("0.4.2"));
        assert_eq!(scope.schema_url(), Some(SCHEMA_URL));
        assert_eq!(scope.schema_url(), Some("https://lablet.dev/schemas/0.1.0"));
    }
}

#[tokio::test]
async fn every_exporter_is_told_the_service_the_sdk_and_what_the_composer_added() {
    let memory = Memory::default();

    let observer = OtelObserver::builder("0.4.2")
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
        assert_eq!(said(key::SERVICE_NAME), Some(Value::from("lablet")));
        assert_eq!(said(key::SERVICE_VERSION), Some(Value::from("0.4.2")));
        assert_eq!(said("team"), Some(Value::from("evals")));
        assert_eq!(said("deployment.environment.name"), Some(Value::from("ci")));
        assert_eq!(
            said(key::TELEMETRY_SDK_NAME),
            Some(Value::from("opentelemetry"))
        );
        assert_eq!(said(key::TELEMETRY_SDK_LANGUAGE), Some(Value::from("rust")));
        assert!(said(key::TELEMETRY_SDK_VERSION).is_some());
        assert_eq!(resource.len(), 7, "{resource:?}");
    }
    observer.shutdown().await.unwrap();
}

// Where a run is exported to

#[tokio::test]
async fn every_destination_is_handed_every_span_and_every_record_and_counts_its_own_losses() {
    let (first, second) = (Memory::default(), Memory::default());
    let observer = OtelObserver::builder("0.1.0")
        .exporting_to(first.spans(), first.records(), first.wide())
        .exporting_over_the_network_to(second.spans(), second.records(), second.wide())
        .build()
        .unwrap();
    second.refuse_records(true);

    told(&observer, a_run(true)).await;
    let flushed = observer.flush().await;

    let failures = flushed.unwrap_err();
    assert_eq!(failures.failures().len(), 1, "{failures}");
    assert!(
        failures.failures()[0].starts_with("otlp log records: "),
        "{failures}"
    );
    assert_eq!(first.exported_spans(), second.exported_spans());
    assert_eq!(first.exported_spans().len(), 4);
    assert_eq!(first.exported_records().len(), 5);
    assert!(second.exported_records().is_empty());
    assert_eq!(events_of(&first.exported_wide()), ["lablet.run"]);
    assert_eq!(events_of(&second.exported_wide()), ["lablet.run"]);
    assert_eq!(
        dropped(&first.exported_wide()[0]),
        AnyValue::Int(0),
        "the first lost nothing, and counts nothing of what the second did"
    );
    assert_eq!(dropped(&second.exported_wide()[0]), AnyValue::Int(5));
}

/// A destination that doesn't answer holds a flush for no longer than the
/// bound it's given, and holds no other destination at all: the file is
/// whole, wide event and all, while the network is stuck.
#[tokio::test]
async fn a_destination_that_does_not_answer_holds_a_flush_only_for_its_bound_and_no_other() {
    let brief = Duration::from_millis(100);
    let (file, network) = (Memory::default(), Memory::default());
    let observer = OtelObserver::builder("0.1.0")
        .exporting_over_the_network_to(network.spans(), network.records(), network.wide())
        .exporting_to(file.spans(), file.records(), file.wide())
        .flush_timeout(brief)
        .build()
        .unwrap();
    network.hold();
    told(&observer, a_run(true)).await;

    let began = Instant::now();
    let flushed = observer.flush().await;
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
    assert_eq!(file.exported_spans().len(), 4);
    assert_eq!(file.exported_records().len(), 5);
    assert_eq!(events_of(&file.exported_wide()), ["lablet.run"]);
    assert_eq!(dropped(&file.exported_wide()[0]), AnyValue::Int(0));
    assert!(network.exports().is_empty());

    // Once the destination answers, the thread the flush left makes the
    // run's wide event, and the shutdown exports it.
    network.release();
    observer.shutdown().await.unwrap();
    assert_eq!(network.exported_spans().len(), 4);
    assert_eq!(events_of(&network.exported_wide()), ["lablet.run"]);
}

#[tokio::test]
async fn a_flush_with_no_destination_returns_at_once_and_a_bound_of_nothing_still_answers() {
    let (memory, other) = (Memory::default(), Memory::default());
    let observer = OtelObserver::builder("0.1.0")
        .exporting_to(memory.spans(), memory.records(), memory.wide())
        .exporting_over_the_network_to(other.spans(), other.records(), other.wide())
        .flush_timeout(Duration::ZERO)
        .build()
        .unwrap();
    told(&observer, a_run(false)).await;

    let flushed = observer.flush().await;

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
    other.release();
    observer.shutdown().await.unwrap();
    assert_eq!(memory.exported_wide().len(), 1);
    assert_eq!(other.exported_wide().len(), 1);
}

#[tokio::test]
async fn an_observer_with_nowhere_to_export_to_takes_a_run_and_answers_for_its_calls() {
    let observer = OtelObserver::builder("0.1.0").build().unwrap();
    let mut events = a_run(true);
    let rest = events.split_off(7);

    told(&observer, events).await;
    let propagated = observer.trace_context(&call_id("call_1"));
    told(&observer, rest).await;

    assert!(propagated.is_some());
    assert_eq!(observer.flush().await, Ok(()));
    assert_eq!(observer.shutdown().await, Ok(()));
}

#[tokio::test]
async fn an_event_is_taken_while_an_export_waits_for_its_destination() {
    let memory = Memory::default();
    let observer = observing(&memory);
    memory.hold();

    let taken = tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..3 {
            told(&observer, a_run(true)).await;
        }
    })
    .await;
    memory.release();
    observer.flush().await.unwrap();

    assert_eq!(taken, Ok(()));
    assert_eq!(memory.exported_spans().len(), 12);
    assert_eq!(memory.exported_wide().len(), 3);
}

#[test]
fn a_builder_prints_what_it_was_given_and_gives_a_flush_and_a_shutdown_five_seconds_by_default() {
    let builder = OtelObserver::builder("0.1.0")
        .resource(vec![("team".to_owned(), "evals".to_owned())])
        .file(FileTarget::Stderr);

    assert_eq!(
        format!("{builder:?}"),
        "OtelObserverBuilder { version: \"0.1.0\", resource: [(\"team\", \"evals\")], \
         file: Some(Stderr), otlp: None, flush_timeout: 5s, shutdown_timeout: 5s, .. }"
    );
}

#[test]
fn a_builder_prints_neither_the_endpoint_nor_a_header_value_it_was_given() {
    let builder = OtelObserver::builder("0.1.0").otlp(OtlpSettings {
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
    let refused = OtelObserver::builder("0.1.0")
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
