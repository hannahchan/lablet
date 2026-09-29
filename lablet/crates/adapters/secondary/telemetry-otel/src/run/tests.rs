use std::time::UNIX_EPOCH;

use lablet_model::{Cost, ProviderErrorKind, ToolCallEnd};
use lablet_run::NetworkTransport;
use lablet_telemetry_registry::signals::{
    EVENT_GEN_AI_CLIENT_INFERENCE_OPERATION_DETAILS_KEYS as CONTENT_KEYS,
    EVENT_GEN_AI_CLIENT_INFERENCE_OPERATION_DETAILS_REQUIRED as CONTENT_REQUIRED,
    EVENT_GEN_AI_CLIENT_OPERATION_EXCEPTION_KEYS as EXCEPTION_KEYS,
    EVENT_GEN_AI_CLIENT_OPERATION_EXCEPTION_REQUIRED as EXCEPTION_REQUIRED,
    EVENT_LABLET_RETRY_KEYS, EVENT_LABLET_RETRY_REQUIRED, SPAN_LABLET_CHAT_KEYS,
    SPAN_LABLET_CHAT_REQUIRED, SPAN_LABLET_EXECUTE_TOOL_KEYS, SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
    SPAN_LABLET_INVOKE_AGENT_KEYS, SPAN_LABLET_INVOKE_AGENT_REQUIRED,
};
use serde_json::json;

use super::*;
use crate::attributes::Held;
use crate::testing::{
    CONFIG_DIGEST, DURATION_MS, MODEL, PROMPT, RUN, STARTED_UNIX_MS, SYSTEM, assert_declared,
    beyond_required, call_id, calls, capturing, completed, context, docs, endpoint, every_count,
    every_parameter, held, labelled, opening, record, run_id, said, stopped, text, tool_name,
    two_counts,
};

fn open(opening: Opening) -> OpenRun {
    OpenRun::open(opening).0
}

fn ms(instant: SystemTime) -> u64 {
    u64::try_from(instant.duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap()
}

/// The one span `signals` hold.
fn span_of(signals: &Signals) -> &Span {
    assert_eq!(signals.spans.len(), 1, "{signals:?}");
    &signals.spans[0]
}

/// The first attempt of turn `turn`, which began `started_ms` into the run
/// with a request of 48,211 bytes and took `latency_ms`.
fn attempted(run: &mut OpenRun, turn: u32, started_ms: u64, latency_ms: u64) -> Attempt {
    run.attempt_began(turn, 1, 48_211);
    Attempt {
        turn,
        number: 1,
        started_ms,
        latency_ms,
    }
}

fn answered(run: &mut OpenRun, usage: Usage) -> Signals {
    attempted(run, 1, 5, 250);
    run.attempt_answered(1, 1, record(usage, 5, 250), None)
}

fn failing(kind: ProviderErrorKind) -> ProviderError {
    ProviderError::new(kind, "529 overloaded")
}

fn ok() -> ToolCallStatus {
    ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::Ok)
}

fn ended(status: ToolCallStatus) -> CallEnd {
    CallEnd {
        turn: 2,
        status,
        started_ms: 1_000,
        latency_ms: 40,
        output_bytes: 20,
        truncated_from_bytes: None,
        mcp: None,
        output: None,
    }
}

/// A call of `bash` that ended `status`.
fn called(run: &mut OpenRun, status: ToolCallStatus) -> Signals {
    run.call_began(
        call_id("call_1"),
        tool_name("bash"),
        Some(ToolSource::Builtin),
        96,
        None,
    );
    run.call_ended(&call_id("call_1"), ended(status))
}

// The root span

#[test]
fn the_root_span_is_named_for_the_agent_and_holds_what_the_registry_requires_of_it() {
    let (signals, _) = open(opening()).close(context(), completed(&context(), two_counts(), None));

    let root = span_of(&signals);
    assert_eq!(root.name, "invoke_agent lablet");
    assert_eq!(root.kind, SpanKind::Internal);
    assert_eq!(root.parent, None);
    assert_declared(
        "the root span",
        &root.attributes,
        SPAN_LABLET_INVOKE_AGENT_REQUIRED,
        SPAN_LABLET_INVOKE_AGENT_KEYS,
    );
    assert_eq!(
        beyond_required(&root.attributes, SPAN_LABLET_INVOKE_AGENT_REQUIRED),
        [""; 0],
        "a run that completed, named no label, and whose provider reported two counts"
    );
    assert_eq!(root.ended, Ended::Well);
    assert!(signals.records.is_empty());
}

#[test]
fn the_root_span_says_what_the_run_was_and_what_became_of_it() {
    let (signals, _) = open(opening()).close(context(), completed(&context(), two_counts(), None));

    let said = &span_of(&signals).attributes;
    for (key, value) in [
        (key::GEN_AI_CONVERSATION_ID, text(RUN)),
        (key::SESSION_ID, text(RUN)),
        (key::LABLET_CONFIG_DIGEST, text(CONFIG_DIGEST)),
        (key::GEN_AI_OPERATION_NAME, text("invoke_agent")),
        (key::GEN_AI_AGENT_NAME, text("lablet")),
        (key::GEN_AI_AGENT_VERSION, text("0.1.0")),
        (key::GEN_AI_REQUEST_MODEL, text(MODEL)),
        (key::LABLET_RUN_STOP_REASON, text("completed")),
        (key::LABLET_RUN_TURNS, Held::Int(1)),
        (key::LABLET_TOOL_CALLS_TOTAL, Held::Int(0)),
        (key::GEN_AI_USAGE_INPUT_TOKENS, Held::Int(1_200)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, Held::Int(80)),
    ] {
        assert_eq!(held(said, key), &value, "{key}");
    }
}

#[test]
fn the_root_span_lasts_from_the_run_s_start_for_as_long_as_the_run_took() {
    let (signals, closed) =
        open(opening()).close(context(), stopped(&context(), StopReason::Timeout));

    let root = span_of(&signals);
    assert_eq!(ms(root.start), STARTED_UNIX_MS);
    assert_eq!(ms(root.end), STARTED_UNIX_MS + DURATION_MS);
    assert_eq!(
        closed.at, root.end,
        "the wide event is timed at the run's end"
    );
    assert_eq!(closed.root, root.id);
}

#[test]
fn the_root_span_of_a_run_that_did_not_complete_names_the_stop_reason_as_its_error() {
    for (reason, said) in [
        (StopReason::RetriesExhausted, "529 overloaded"),
        (StopReason::Timeout, ""),
    ] {
        let (signals, _) = open(opening()).close(context(), stopped(&context(), reason));

        let root = span_of(&signals);
        assert_eq!(
            held(&root.attributes, key::ERROR_TYPE),
            &text(reason.as_str())
        );
        assert_eq!(
            held(&root.attributes, key::LABLET_RUN_STOP_REASON),
            &text(reason.as_str())
        );
        assert_eq!(root.ended, Ended::Badly(said.to_owned()), "{reason}");
        assert_declared(
            "the root span",
            &root.attributes,
            SPAN_LABLET_INVOKE_AGENT_REQUIRED,
            SPAN_LABLET_INVOKE_AGENT_KEYS,
        );
    }
}

#[test]
fn the_root_span_holds_the_counts_the_provider_reported_and_the_cost_of_a_priced_run() {
    let cost = Cost::new(0.0421).unwrap();
    let (signals, _) =
        open(opening()).close(context(), completed(&context(), every_count(), Some(cost)));

    let root = span_of(&signals);
    assert_eq!(
        beyond_required(&root.attributes, SPAN_LABLET_INVOKE_AGENT_REQUIRED),
        [
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
            key::LABLET_RUN_COST_USD,
        ]
    );
    for (key, value) in [
        (key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, Held::Int(30)),
        (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, Held::Int(1_000)),
        (key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS, Held::Int(100)),
        (key::LABLET_RUN_COST_USD, Held::Float(0.0421)),
    ] {
        assert_eq!(held(&root.attributes, key), &value, "{key}");
    }
    assert_declared(
        "the root span",
        &root.attributes,
        SPAN_LABLET_INVOKE_AGENT_REQUIRED,
        SPAN_LABLET_INVOKE_AGENT_KEYS,
    );
}

// The labels

#[test]
fn a_run_s_labels_are_on_every_span_and_every_record_of_it() {
    let labelled_context = || RunContext {
        labels: labelled(),
        ..capturing().context
    };
    let (mut run, offered) = OpenRun::open(Opening {
        context: labelled_context(),
        ..capturing()
    });
    let failed = attempted(&mut run, 1, 0, 40);
    let mut signals = vec![
        offered,
        run.attempt_failed(failed, &failing(ProviderErrorKind::Retryable), None),
        answered(&mut run, two_counts()),
        called(&mut run, ok()),
    ];
    signals.push(
        run.close(
            labelled_context(),
            stopped(&labelled_context(), StopReason::Cancelled),
        )
        .0,
    );

    let spans: Vec<_> = signals.iter().flat_map(|signals| &signals.spans).collect();
    let records: Vec<_> = signals
        .iter()
        .flat_map(|signals| &signals.records)
        .collect();
    assert_eq!(
        (spans.len(), records.len()),
        (4, 4),
        "a span of each kind, and a record of each kind but the wide event"
    );
    for attributes in spans
        .iter()
        .map(|span| &span.attributes)
        .chain(records.iter().map(|record| &record.attributes))
    {
        assert_eq!(
            held(attributes, key::LABLET_TASK_ID),
            &text("fix-failing-test")
        );
        assert_eq!(
            held(attributes, key::LABLET_EXPERIMENT_ID),
            &text("terse-tool-descriptions")
        );
        assert_eq!(held(attributes, key::LABLET_TRIAL), &text("3"));
        assert_eq!(held(attributes, key::GEN_AI_CONVERSATION_ID), &text(RUN));
        assert_eq!(held(attributes, key::SESSION_ID), &text(RUN));
        assert_eq!(
            held(attributes, key::LABLET_CONFIG_DIGEST),
            &text(CONFIG_DIGEST)
        );
    }
}

#[test]
fn a_label_the_run_request_did_not_name_is_left_off() {
    let (mut run, _) = OpenRun::open(Opening {
        context: RunContext {
            labels: lablet_model::RunLabels {
                experiment: None,
                ..labelled()
            },
            ..context()
        },
        ..opening()
    });

    let signals = answered(&mut run, two_counts());

    let attributes = &span_of(&signals).attributes;
    assert_eq!(attributes.held(key::LABLET_EXPERIMENT_ID), None);
    assert_eq!(
        held(attributes, key::LABLET_TASK_ID),
        &text("fix-failing-test")
    );
    assert_eq!(held(attributes, key::LABLET_TRIAL), &text("3"));
}

// Chat spans

#[test]
fn a_chat_span_is_named_for_the_model_and_holds_what_the_registry_requires_of_it() {
    let mut run = open(opening());

    let signals = answered(&mut run, two_counts());

    let chat = span_of(&signals);
    assert_eq!(chat.name, "chat scripted-1");
    assert_eq!(chat.kind, SpanKind::Client);
    assert_declared(
        "a chat span",
        &chat.attributes,
        SPAN_LABLET_CHAT_REQUIRED,
        SPAN_LABLET_CHAT_KEYS,
    );
    assert_eq!(chat.ended, Ended::Well);
    assert!(chat.events.is_empty());
    assert!(signals.records.is_empty());
}

#[test]
fn a_chat_span_is_a_child_of_the_root_span_with_an_id_of_its_own() {
    let mut run = open(opening());
    let first = answered(&mut run, two_counts());
    let second = answered(&mut run, two_counts());

    let (root, _) = run.close(context(), stopped(&context(), StopReason::Cancelled));

    let root = span_of(&root).id;
    assert_eq!(span_of(&first).parent, Some(root));
    assert_eq!(span_of(&second).parent, Some(root));
    assert_ne!(span_of(&first).id, span_of(&second).id);
    assert_ne!(span_of(&first).id, root);
}

#[test]
fn a_chat_span_says_what_was_asked_and_what_was_answered() {
    let mut run = open(opening());
    run.attempt_began(3, 2, 48_211);

    let signals = run.attempt_answered(3, 2, record(two_counts(), 5, 250), None);

    let said = &span_of(&signals).attributes;
    for (key, value) in [
        (key::GEN_AI_OPERATION_NAME, text("chat")),
        (key::GEN_AI_PROVIDER_NAME, text("fake")),
        (key::GEN_AI_REQUEST_MODEL, text(MODEL)),
        (key::GEN_AI_REQUEST_MAX_TOKENS, Held::Int(4_096)),
        (key::LABLET_CHAT_PURPOSE, text("turn")),
        (key::LABLET_TURN, Held::Int(3)),
        (key::LABLET_ATTEMPT, Held::Int(2)),
        (key::LABLET_REQUEST_BYTES, Held::Int(48_211)),
        (key::GEN_AI_RESPONSE_ID, text("msg_01")),
        (key::GEN_AI_RESPONSE_MODEL, text("scripted-2026-09")),
        (
            key::GEN_AI_RESPONSE_FINISH_REASONS,
            Held::Texts(vec!["tool_use".to_owned()]),
        ),
        (key::GEN_AI_USAGE_INPUT_TOKENS, Held::Int(1_200)),
        (key::GEN_AI_USAGE_OUTPUT_TOKENS, Held::Int(80)),
    ] {
        assert_eq!(held(said, key), &value, "{key}");
    }
}

#[test]
fn a_chat_span_leaves_out_what_the_run_did_not_set_and_the_provider_did_not_report() {
    let mut run = open(opening());
    run.attempt_began(1, 1, 48_211);

    let signals = run.attempt_answered(
        1,
        1,
        TurnRecord {
            response_id: None,
            response_model: None,
            ..record(two_counts(), 5, 250)
        },
        None,
    );

    assert_eq!(
        beyond_required(&span_of(&signals).attributes, SPAN_LABLET_CHAT_REQUIRED),
        [
            key::GEN_AI_RESPONSE_FINISH_REASONS,
            key::GEN_AI_USAGE_INPUT_TOKENS,
            key::GEN_AI_USAGE_OUTPUT_TOKENS,
        ]
    );
}

#[test]
fn a_chat_span_holds_every_parameter_the_run_set_and_every_count_the_provider_reported() {
    let mut run = open(Opening {
        context: RunContext {
            labels: labelled(),
            ..context()
        },
        request: every_parameter(),
        endpoint: Some(endpoint()),
        ..opening()
    });

    let signals = answered(&mut run, every_count());

    let said = &span_of(&signals).attributes;
    for (key, value) in [
        (key::GEN_AI_REQUEST_TEMPERATURE, Held::Float(0.7)),
        (key::GEN_AI_REQUEST_SEED, Held::Int(-42)),
        (key::GEN_AI_REQUEST_REASONING_LEVEL, text("high")),
        (key::SERVER_ADDRESS, text("api.example.com")),
        (key::SERVER_PORT, Held::Int(443)),
        (key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS, Held::Int(30)),
        (key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS, Held::Int(1_000)),
        (key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS, Held::Int(100)),
    ] {
        assert_eq!(held(said, key), &value, "{key}");
    }
    assert_declared(
        "a chat span",
        said,
        SPAN_LABLET_CHAT_REQUIRED,
        SPAN_LABLET_CHAT_KEYS,
    );
    let mut every_key = said.keys();
    every_key.push(key::ERROR_TYPE);
    every_key.sort_unstable();
    assert_eq!(
        every_key, SPAN_LABLET_CHAT_KEYS,
        "with the error of an attempt that failed, these are every key the registry declares"
    );
}

#[test]
fn a_chat_span_starts_when_its_attempt_began_and_lasts_as_long_as_the_attempt_took() {
    let mut run = open(opening());
    run.attempt_began(1, 1, 48_211);
    run.attempt_began(1, 2, 48_211);

    let failed = run.attempt_failed(
        Attempt {
            turn: 1,
            number: 1,
            started_ms: 7,
            latency_ms: 40,
        },
        &failing(ProviderErrorKind::Retryable),
        Some(Duration::from_secs(2)),
    );
    let answered = run.attempt_answered(1, 2, record(two_counts(), 2_047, 250), None);

    let failed = span_of(&failed);
    assert_eq!(ms(failed.start), STARTED_UNIX_MS + 7);
    assert_eq!(ms(failed.end), STARTED_UNIX_MS + 47);
    let answered = span_of(&answered);
    assert_eq!(ms(answered.start), STARTED_UNIX_MS + 2_047);
    assert_eq!(ms(answered.end), STARTED_UNIX_MS + 2_297);
}

#[test]
fn nothing_comes_of_the_end_of_an_attempt_that_never_began() {
    let mut run = open(capturing());
    run.attempt_began(1, 1, 48_211);

    let other_turn = run.attempt_answered(2, 1, record(two_counts(), 5, 250), Some(&[said("Hi")]));
    let other_attempt = run.attempt_failed(
        Attempt {
            turn: 1,
            number: 2,
            started_ms: 5,
            latency_ms: 250,
        },
        &failing(ProviderErrorKind::Fatal),
        None,
    );
    let twice = [
        run.attempt_answered(1, 1, record(two_counts(), 5, 250), None),
        run.attempt_answered(1, 1, record(two_counts(), 5, 250), None),
    ];

    assert_eq!(other_turn, Signals::default());
    assert_eq!(other_attempt, Signals::default());
    assert_eq!(twice[0].spans.len(), 1);
    assert_eq!(twice[1], Signals::default(), "an attempt ends once");
}

// A provider call that failed

#[test]
fn the_span_of_an_attempt_that_failed_names_the_class_of_its_error() {
    for kind in [
        ProviderErrorKind::Retryable,
        ProviderErrorKind::ContextExhausted,
        ProviderErrorKind::Auth,
        ProviderErrorKind::Fatal,
        ProviderErrorKind::Malformed,
    ] {
        let mut run = open(opening());
        let attempt = attempted(&mut run, 1, 5, 40);

        let signals = run.attempt_failed(attempt, &failing(kind), None);

        let chat = span_of(&signals);
        assert_eq!(chat.name, "chat scripted-1");
        assert_eq!(chat.kind, SpanKind::Client);
        assert_eq!(
            held(&chat.attributes, key::ERROR_TYPE),
            &text(kind.as_str())
        );
        assert_eq!(chat.ended, Ended::Badly("529 overloaded".to_owned()));
        assert_declared(
            "the chat span of a failed attempt",
            &chat.attributes,
            SPAN_LABLET_CHAT_REQUIRED,
            SPAN_LABLET_CHAT_KEYS,
        );
        assert_eq!(
            beyond_required(&chat.attributes, SPAN_LABLET_CHAT_REQUIRED),
            [key::ERROR_TYPE],
            "an attempt that reported no usage has none"
        );
    }
}

#[test]
fn the_span_of_an_attempt_that_failed_holds_the_usage_the_attempt_reported() {
    let mut run = open(opening());
    let attempt = attempted(&mut run, 1, 5, 40);
    let error = failing(ProviderErrorKind::Retryable).with_usage(Usage {
        input_tokens: 800,
        cache_read_tokens: Some(600),
        ..Usage::default()
    });

    let signals = run.attempt_failed(attempt, &error, None);

    let chat = span_of(&signals);
    assert_eq!(
        beyond_required(&chat.attributes, SPAN_LABLET_CHAT_REQUIRED),
        [
            key::ERROR_TYPE,
            key::GEN_AI_USAGE_INPUT_TOKENS,
            key::GEN_AI_USAGE_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
        ]
    );
    assert_eq!(
        held(&chat.attributes, key::GEN_AI_USAGE_INPUT_TOKENS),
        &Held::Int(800)
    );
    assert_eq!(
        held(&chat.attributes, key::GEN_AI_USAGE_OUTPUT_TOKENS),
        &Held::Int(0)
    );
    assert_eq!(
        held(&chat.attributes, key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS),
        &Held::Int(600)
    );
}

#[test]
fn a_failed_attempt_has_a_retry_event_that_says_whether_the_loop_tries_again_and_after_how_long() {
    for (retry, beyond, will_retry) in [
        (
            Some(Duration::from_micros(2_000_900)),
            vec![key::LABLET_RETRY_BACKOFF_MS],
            true,
        ),
        (None, Vec::new(), false),
    ] {
        let mut run = open(opening());
        run.attempt_began(1, 3, 48_211);
        let attempt = Attempt {
            turn: 1,
            number: 3,
            started_ms: 5,
            latency_ms: 40,
        };

        let signals = run.attempt_failed(attempt, &failing(ProviderErrorKind::Retryable), retry);

        let chat = span_of(&signals);
        assert_eq!(chat.events.len(), 1);
        let event = &chat.events[0];
        assert_eq!(event.name, "lablet.retry");
        assert_eq!(event.at, chat.end);
        assert_declared(
            "the retry event",
            &event.attributes,
            EVENT_LABLET_RETRY_REQUIRED,
            EVENT_LABLET_RETRY_KEYS,
        );
        assert_eq!(
            beyond_required(&event.attributes, EVENT_LABLET_RETRY_REQUIRED),
            beyond
        );
        assert_eq!(held(&event.attributes, key::LABLET_ATTEMPT), &Held::Int(3));
        assert_eq!(
            held(&event.attributes, key::LABLET_RETRY_WILL_RETRY),
            &Held::Flag(will_retry)
        );
        if will_retry {
            assert_eq!(
                held(&event.attributes, key::LABLET_RETRY_BACKOFF_MS),
                &Held::Int(2_000),
                "whole milliseconds, as the loop's own are cut"
            );
        }
    }
}

#[test]
fn a_failed_attempt_has_an_exception_record_in_the_context_of_its_span() {
    let mut run = open(opening());
    run.attempt_began(2, 3, 48_211);
    let attempt = Attempt {
        turn: 2,
        number: 3,
        started_ms: 5,
        latency_ms: 40,
    };

    let signals = run.attempt_failed(attempt, &failing(ProviderErrorKind::Auth), None);

    assert_eq!(signals.records.len(), 1);
    let exception = &signals.records[0];
    let chat = span_of(&signals);
    assert_eq!(exception.name, "gen_ai.client.operation.exception");
    assert_eq!(exception.severity, Severity::Warn);
    assert_eq!(exception.span, chat.id);
    assert_eq!(exception.at, chat.end);
    assert_declared(
        "the exception record",
        &exception.attributes,
        EXCEPTION_REQUIRED,
        EXCEPTION_KEYS,
    );
    for (key, value) in [
        (key::EXCEPTION_TYPE, text("auth")),
        (key::EXCEPTION_MESSAGE, text("529 overloaded")),
        (key::GEN_AI_OPERATION_NAME, text("chat")),
        (key::GEN_AI_PROVIDER_NAME, text("fake")),
        (key::GEN_AI_REQUEST_MODEL, text(MODEL)),
        (key::LABLET_TURN, Held::Int(2)),
        (key::LABLET_ATTEMPT, Held::Int(3)),
    ] {
        assert_eq!(held(&exception.attributes, key), &value, "{key}");
    }
}

// Tool spans

#[test]
fn a_tool_span_is_named_for_the_tool_and_holds_what_the_registry_requires_of_it() {
    let mut run = open(opening());

    let signals = called(&mut run, ok());

    let tool = span_of(&signals);
    assert_eq!(tool.name, "execute_tool bash");
    assert_eq!(tool.kind, SpanKind::Internal);
    assert_declared(
        "a tool span",
        &tool.attributes,
        SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
        SPAN_LABLET_EXECUTE_TOOL_KEYS,
    );
    assert_eq!(
        beyond_required(&tool.attributes, SPAN_LABLET_EXECUTE_TOOL_REQUIRED),
        [
            key::GEN_AI_TOOL_TYPE,
            key::LABLET_TOOL_SOURCE,
            key::GEN_AI_TOOL_DESCRIPTION,
        ],
        "what the span of a call to a tool the run offered holds, when the call ended well"
    );
    assert_eq!(tool.ended, Ended::Well);
    assert!(signals.records.is_empty());
}

#[test]
fn a_tool_span_says_what_was_called_and_what_became_of_the_call() {
    let mut run = open(opening());

    let signals = called(&mut run, ok());

    let said = &span_of(&signals).attributes;
    for (key, value) in [
        (key::GEN_AI_OPERATION_NAME, text("execute_tool")),
        (key::GEN_AI_TOOL_NAME, text("bash")),
        (key::GEN_AI_TOOL_CALL_ID, text("call_1")),
        (key::GEN_AI_TOOL_TYPE, text("function")),
        (key::GEN_AI_TOOL_DESCRIPTION, text("What bash does.")),
        (key::LABLET_TOOL_SOURCE, text("builtin")),
        (key::LABLET_TURN, Held::Int(2)),
        (key::LABLET_TOOL_STATUS, text("ok")),
        (key::LABLET_TOOL_INPUT_BYTES, Held::Int(96)),
        (key::LABLET_TOOL_OUTPUT_BYTES, Held::Int(20)),
        (key::LABLET_TOOL_OUTPUT_TRUNCATED, Held::Flag(false)),
        (key::LABLET_TOOL_IS_ERROR, Held::Flag(false)),
    ] {
        assert_eq!(held(said, key), &value, "{key}");
    }
}

#[test]
fn a_tool_span_starts_when_its_call_began_and_lasts_as_long_as_the_call_took() {
    let mut run = open(opening());

    let signals = called(&mut run, ok());

    let tool = span_of(&signals);
    assert_eq!(ms(tool.start), STARTED_UNIX_MS + 1_000);
    assert_eq!(ms(tool.end), STARTED_UNIX_MS + 1_040);
}

#[test]
fn the_span_of_a_call_that_did_not_end_well_names_its_status_as_its_error() {
    let ran = |ended| ToolCallStatus::ran(ToolSource::Builtin, ended);
    for status in [
        ran(ToolCallEnd::ToolError),
        ran(ToolCallEnd::Timeout),
        ran(ToolCallEnd::Failed),
        ToolCallStatus::MalformedInput,
        ToolCallStatus::Rejected,
    ] {
        let mut run = open(opening());

        let signals = called(&mut run, status.clone());

        let tool = span_of(&signals);
        assert_eq!(
            held(&tool.attributes, key::ERROR_TYPE),
            &text(status.as_str())
        );
        assert_eq!(
            held(&tool.attributes, key::LABLET_TOOL_STATUS),
            &text(status.as_str())
        );
        assert_eq!(
            held(&tool.attributes, key::LABLET_TOOL_IS_ERROR),
            &Held::Flag(true)
        );
        assert_eq!(
            tool.ended,
            Ended::Badly(String::new()),
            "what a tool said of its failure is content, and a span holds none"
        );
        assert_eq!(
            held(&tool.attributes, key::LABLET_TOOL_SOURCE),
            &text("builtin"),
            "{status}: the name was one the run offered, whether or not a tool ran"
        );
        assert_declared(
            "a tool span",
            &tool.attributes,
            SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
            SPAN_LABLET_EXECUTE_TOOL_KEYS,
        );
    }
}

#[test]
fn the_span_of_a_call_to_a_name_no_tool_has_says_nothing_of_a_tool() {
    let mut run = open(opening());
    run.call_began(call_id("call_1"), tool_name("bsah"), None, 96, None);

    let signals = run.call_ended(&call_id("call_1"), ended(ToolCallStatus::Unknown));

    let tool = span_of(&signals);
    assert_eq!(tool.name, "execute_tool bsah");
    assert_eq!(
        beyond_required(&tool.attributes, SPAN_LABLET_EXECUTE_TOOL_REQUIRED),
        [key::ERROR_TYPE]
    );
    assert_eq!(held(&tool.attributes, key::ERROR_TYPE), &text("unknown"));
    assert_declared(
        "a tool span",
        &tool.attributes,
        SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
        SPAN_LABLET_EXECUTE_TOOL_KEYS,
    );
}

#[test]
fn the_span_of_a_call_whose_output_was_cut_says_how_long_the_output_was() {
    let mut run = open(opening());
    run.call_began(
        call_id("call_1"),
        tool_name("bash"),
        Some(ToolSource::Builtin),
        96,
        None,
    );

    let signals = run.call_ended(
        &call_id("call_1"),
        CallEnd {
            truncated_from_bytes: Some(5_242_880),
            ..ended(ok())
        },
    );

    let said = &span_of(&signals).attributes;
    assert_eq!(
        held(said, key::LABLET_TOOL_OUTPUT_TRUNCATED),
        &Held::Flag(true)
    );
    assert_eq!(
        held(said, key::LABLET_TOOL_OUTPUT_ORIGINAL_BYTES),
        &Held::Int(5_242_880)
    );
    assert_eq!(held(said, key::LABLET_TOOL_OUTPUT_BYTES), &Held::Int(20));
}

#[test]
fn the_span_of_a_call_over_mcp_holds_what_the_call_carried_back() {
    let meta = |whole: bool| McpCallMeta {
        method: "tools/call".to_owned(),
        session_id: whole.then(|| "session-7".to_owned()),
        protocol_version: whole.then(|| "2025-06-18".to_owned()),
        jsonrpc_request_id: whole.then(|| "12".to_owned()),
        rpc_status_code: whole.then(|| "-32602".to_owned()),
        transport: if whole {
            NetworkTransport::Tcp
        } else {
            NetworkTransport::Pipe
        },
    };
    let over = |meta: McpCallMeta| {
        let mut run = open(Opening {
            context: RunContext {
                labels: labelled(),
                ..context()
            },
            ..opening()
        });
        run.call_began(
            call_id("call_1"),
            tool_name("mcp__docs__search"),
            Some(docs()),
            96,
            None,
        );
        let signals = run.call_ended(
            &call_id("call_1"),
            CallEnd {
                mcp: Some(meta),
                ..ended(ToolCallStatus::ran(docs(), ToolCallEnd::Failed))
            },
        );
        span_of(&signals).attributes.clone()
    };

    let whole = over(meta(true));
    let bare = over(meta(false));

    for (key, value) in [
        (key::GEN_AI_TOOL_TYPE, text("extension")),
        (key::LABLET_TOOL_SOURCE, text("mcp")),
        (
            key::GEN_AI_TOOL_DESCRIPTION,
            text("What mcp__docs__search does."),
        ),
        (key::MCP_METHOD_NAME, text("tools/call")),
        (key::MCP_SESSION_ID, text("session-7")),
        (key::MCP_PROTOCOL_VERSION, text("2025-06-18")),
        (key::JSONRPC_REQUEST_ID, text("12")),
        (key::RPC_RESPONSE_STATUS_CODE, text("-32602")),
        (key::NETWORK_TRANSPORT, text("tcp")),
    ] {
        assert_eq!(held(&whole, key), &value, "{key}");
    }
    let mut every_key = whole.keys();
    every_key.push(key::LABLET_TOOL_OUTPUT_ORIGINAL_BYTES);
    every_key.sort_unstable();
    assert_eq!(
        every_key, SPAN_LABLET_EXECUTE_TOOL_KEYS,
        "with the size of an output that was cut, these are every key the registry declares"
    );
    assert_eq!(
        beyond_required(&bare, SPAN_LABLET_EXECUTE_TOOL_REQUIRED),
        [
            key::LABLET_TASK_ID,
            key::LABLET_EXPERIMENT_ID,
            key::LABLET_TRIAL,
            key::GEN_AI_TOOL_TYPE,
            key::LABLET_TOOL_SOURCE,
            key::GEN_AI_TOOL_DESCRIPTION,
            key::ERROR_TYPE,
            key::MCP_METHOD_NAME,
            key::NETWORK_TRANSPORT,
        ]
    );
    assert_eq!(held(&bare, key::NETWORK_TRANSPORT), &text("pipe"));
}

// The span an executor propagates

#[test]
fn the_span_a_call_propagates_is_the_span_the_call_ends_as() {
    let mut run = open(opening());
    run.call_began(call_id("call_1"), tool_name("bash"), None, 96, None);
    run.call_began(call_id("call_2"), tool_name("bash"), None, 96, None);

    let propagated = run.propagated(&call_id("call_1")).unwrap();
    let other = run.propagated(&call_id("call_2")).unwrap();
    let signals = run.call_ended(&call_id("call_1"), ended(ok()));

    let tool = span_of(&signals);
    assert_eq!(
        propagated.traceparent,
        format!("00-{}-{}-01", run.trace(), tool.id)
    );
    assert_eq!(propagated.traceparent.len(), 55);
    assert_eq!(propagated.tracestate, None);
    assert_ne!(other.traceparent, propagated.traceparent);
    assert_eq!(
        tool.parent,
        Some(
            run.close(context(), stopped(&context(), StopReason::Cancelled))
                .1
                .root
        )
    );
}

#[test]
fn a_call_propagates_a_span_only_while_it_runs() {
    let mut run = open(opening());

    let before = run.propagated(&call_id("call_1"));
    run.call_began(call_id("call_1"), tool_name("bash"), None, 96, None);
    let during = run.propagated(&call_id("call_1"));
    run.call_ended(&call_id("call_1"), ended(ok()));
    let after = run.propagated(&call_id("call_1"));

    assert_eq!(before, None);
    assert!(during.is_some());
    assert_eq!(after, None);
}

#[test]
fn nothing_comes_of_the_end_of_a_call_that_never_began() {
    let mut run = open(capturing());
    run.call_began(call_id("call_1"), tool_name("bash"), None, 96, None);

    let other = run.call_ended(&call_id("call_2"), ended(ok()));
    let twice = [
        run.call_ended(&call_id("call_1"), ended(ok())),
        run.call_ended(&call_id("call_1"), ended(ok())),
    ];

    assert_eq!(other, Signals::default());
    assert_eq!(twice[0].spans.len(), 1);
    assert_eq!(twice[1], Signals::default(), "a call ends once");
}

#[test]
fn a_run_is_the_run_of_its_own_id_and_of_no_other() {
    let run = open(opening());

    assert!(run.is(&run_id()));
    assert!(!run.is(&RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNED").unwrap()));
}

// Content

fn parsed(held: &Held) -> serde_json::Value {
    let Held::Text(json) = held else {
        panic!("{held:?} isn't text");
    };
    serde_json::from_str(json).unwrap()
}

#[test]
fn a_run_that_captures_no_content_has_no_record_of_any() {
    let (mut run, offered) = OpenRun::open(opening());
    let attempt = attempted(&mut run, 1, 0, 40);
    let failed = run.attempt_failed(attempt, &failing(ProviderErrorKind::Retryable), None);
    let answered = answered(&mut run, two_counts());
    let called = called(&mut run, ok());

    assert_eq!(offered, Signals::default());
    assert_eq!(answered.records, []);
    assert_eq!(called.records, []);
    let names: Vec<_> = failed.records.iter().map(|record| record.name).collect();
    assert_eq!(names, ["gen_ai.client.operation.exception"]);
}

/// The loop decides what an observer is handed, and an observer that was
/// handed content by a run that doesn't capture any keeps it to itself.
#[test]
fn content_that_came_with_the_events_of_a_run_that_captures_none_is_not_recorded() {
    let (mut run, offered) = OpenRun::open(Opening {
        system_prompt: Some(SYSTEM.to_owned()),
        prompt: Some(PROMPT.to_owned()),
        ..opening()
    });
    attempted(&mut run, 1, 5, 250);
    let answered = run.attempt_answered(1, 1, record(two_counts(), 5, 250), Some(&[said("Hi")]));
    run.call_began(
        call_id("call_1"),
        tool_name("bash"),
        Some(ToolSource::Builtin),
        96,
        Some(&json!({ "command": "ls" })),
    );
    let called = run.call_ended(
        &call_id("call_1"),
        CallEnd {
            output: Some(vec![ToolResultContent::Text("src".to_owned())]),
            ..ended(ok())
        },
    );

    assert_eq!(offered, Signals::default());
    assert_eq!(answered.records, []);
    assert_eq!(called.records, []);
}

#[test]
fn a_run_that_captures_content_opens_with_a_record_of_the_tools_it_offers() {
    let (run, offered) = OpenRun::open(capturing());

    assert!(offered.spans.is_empty());
    assert_eq!(offered.records.len(), 1);
    let record = &offered.records[0];
    assert_eq!(record.name, "gen_ai.client.inference.operation.details");
    assert_eq!(record.severity, Severity::Info);
    assert_eq!(ms(record.at), STARTED_UNIX_MS);
    assert_declared(
        "the record of the tools",
        &record.attributes,
        CONTENT_REQUIRED,
        CONTENT_KEYS,
    );
    assert_eq!(
        beyond_required(&record.attributes, CONTENT_REQUIRED),
        [key::GEN_AI_TOOL_DEFINITIONS]
    );
    assert_eq!(
        held(&record.attributes, key::GEN_AI_OPERATION_NAME),
        &text("invoke_agent")
    );
    let definitions = parsed(held(&record.attributes, key::GEN_AI_TOOL_DEFINITIONS));
    assert_eq!(definitions[0]["name"], "bash");
    assert_eq!(definitions[1]["name"], "mcp__docs__search");
    assert_eq!(definitions.as_array().unwrap().len(), 2);
    let (root, _) = run.close(context(), stopped(&context(), StopReason::Cancelled));
    assert_eq!(
        record.span,
        span_of(&root).id,
        "it's in the root span's context"
    );
}

#[test]
fn a_provider_call_has_a_record_of_what_it_was_sent_and_what_it_answered() {
    let mut run = open(capturing());
    attempted(&mut run, 1, 5, 250);

    let signals = run.attempt_answered(
        1,
        1,
        record(two_counts(), 5, 250),
        Some(&[said("On it."), calls("call_1", "bash")]),
    );

    assert_eq!(signals.records.len(), 1);
    let exchanged = &signals.records[0];
    let chat = span_of(&signals);
    assert_eq!(exchanged.name, "gen_ai.client.inference.operation.details");
    assert_eq!(exchanged.span, chat.id);
    assert_eq!(exchanged.at, chat.end);
    assert_declared(
        "the record of a provider call",
        &exchanged.attributes,
        CONTENT_REQUIRED,
        CONTENT_KEYS,
    );
    assert_eq!(
        beyond_required(&exchanged.attributes, CONTENT_REQUIRED),
        [
            key::LABLET_TURN,
            key::GEN_AI_SYSTEM_INSTRUCTIONS,
            key::GEN_AI_INPUT_MESSAGES,
            key::GEN_AI_OUTPUT_MESSAGES,
        ]
    );
    assert_eq!(
        held(&exchanged.attributes, key::GEN_AI_OPERATION_NAME),
        &text("chat")
    );
    assert_eq!(held(&exchanged.attributes, key::LABLET_TURN), &Held::Int(1));
    assert_eq!(
        parsed(held(&exchanged.attributes, key::GEN_AI_SYSTEM_INSTRUCTIONS)),
        json!([{ "type": "text", "content": SYSTEM }])
    );
    assert_eq!(
        parsed(held(&exchanged.attributes, key::GEN_AI_INPUT_MESSAGES)),
        json!([{ "role": "user", "parts": [{ "type": "text", "content": PROMPT }] }])
    );
    let output = parsed(held(&exchanged.attributes, key::GEN_AI_OUTPUT_MESSAGES));
    assert_eq!(output[0]["parts"][0]["content"], "On it.");
    assert_eq!(output[0]["parts"][1]["id"], "call_1");
    assert!(
        chat.attributes.keys().iter().all(|key| ![
            key::GEN_AI_SYSTEM_INSTRUCTIONS,
            key::GEN_AI_INPUT_MESSAGES,
            key::GEN_AI_OUTPUT_MESSAGES
        ]
        .contains(key)),
        "content never appears on a span"
    );
}

#[test]
fn a_tool_call_has_a_record_of_its_arguments_and_its_result() {
    let mut run = open(capturing());
    run.call_began(
        call_id("call_1"),
        tool_name("bash"),
        Some(ToolSource::Builtin),
        96,
        Some(&json!({ "command": "ls", "cwd": null })),
    );

    let signals = run.call_ended(
        &call_id("call_1"),
        CallEnd {
            output: Some(vec![ToolResultContent::Text(
                "no such directory".to_owned(),
            )]),
            ..ended(ToolCallStatus::ran(
                ToolSource::Builtin,
                ToolCallEnd::ToolError,
            ))
        },
    );

    assert_eq!(signals.records.len(), 1);
    let returned = &signals.records[0];
    let tool = span_of(&signals);
    assert_eq!(returned.name, "gen_ai.client.inference.operation.details");
    assert_eq!(returned.span, tool.id);
    assert_eq!(returned.at, tool.end);
    assert_declared(
        "the record of a tool call",
        &returned.attributes,
        CONTENT_REQUIRED,
        CONTENT_KEYS,
    );
    assert_eq!(
        beyond_required(&returned.attributes, CONTENT_REQUIRED),
        [
            key::GEN_AI_TOOL_NAME,
            key::GEN_AI_TOOL_CALL_ID,
            key::LABLET_TURN,
            key::GEN_AI_TOOL_CALL_ARGUMENTS,
            key::GEN_AI_TOOL_CALL_RESULT,
        ]
    );
    for (key, value) in [
        (key::GEN_AI_OPERATION_NAME, text("execute_tool")),
        (key::GEN_AI_TOOL_NAME, text("bash")),
        (key::GEN_AI_TOOL_CALL_ID, text("call_1")),
        (key::LABLET_TURN, Held::Int(2)),
    ] {
        assert_eq!(held(&returned.attributes, key), &value, "{key}");
    }
    assert_eq!(
        parsed(held(&returned.attributes, key::GEN_AI_TOOL_CALL_ARGUMENTS)),
        json!({ "command": "ls", "cwd": null })
    );
    assert_eq!(
        parsed(held(&returned.attributes, key::GEN_AI_TOOL_CALL_RESULT)),
        json!({
            "content": [{ "type": "text", "text": "no such directory" }],
            "isError": true,
        })
    );
    assert!(
        tool.attributes.keys().iter().all(|key| ![
            key::GEN_AI_TOOL_CALL_ARGUMENTS,
            key::GEN_AI_TOOL_CALL_RESULT
        ]
        .contains(key)),
        "content never appears on a span"
    );
}

#[test]
fn the_record_of_a_call_whose_arguments_did_not_parse_holds_the_result_alone() {
    let mut run = open(capturing());
    run.call_began(
        call_id("call_1"),
        tool_name("bash"),
        Some(ToolSource::Builtin),
        14,
        None,
    );

    let signals = run.call_ended(
        &call_id("call_1"),
        CallEnd {
            output: Some(vec![ToolResultContent::Text("not JSON".to_owned())]),
            ..ended(ToolCallStatus::MalformedInput)
        },
    );

    assert_eq!(
        beyond_required(&signals.records[0].attributes, CONTENT_REQUIRED),
        [
            key::GEN_AI_TOOL_NAME,
            key::GEN_AI_TOOL_CALL_ID,
            key::LABLET_TURN,
            key::GEN_AI_TOOL_CALL_RESULT,
        ]
    );
}

#[test]
fn a_later_provider_call_was_sent_the_conversation_so_far() {
    let mut run = open(capturing());
    attempted(&mut run, 1, 5, 250);
    run.attempt_answered(
        1,
        1,
        record(two_counts(), 5, 250),
        Some(&[calls("call_1", "bash")]),
    );
    run.call_began(
        call_id("call_1"),
        tool_name("bash"),
        Some(ToolSource::Builtin),
        96,
        Some(&json!({})),
    );
    run.call_ended(
        &call_id("call_1"),
        CallEnd {
            output: Some(vec![ToolResultContent::Text("2 passed".to_owned())]),
            ..ended(ok())
        },
    );
    let failed = attempted(&mut run, 2, 2_000, 40);
    let failed = run.attempt_failed(failed, &failing(ProviderErrorKind::Retryable), None);
    run.attempt_began(2, 2, 48_211);

    let answered = run.attempt_answered(
        2,
        2,
        record(two_counts(), 3_000, 250),
        Some(&[said("Done.")]),
    );

    let names: Vec<_> = failed.records.iter().map(|record| record.name).collect();
    assert_eq!(
        names,
        [
            "gen_ai.client.operation.exception",
            "gen_ai.client.inference.operation.details"
        ]
    );
    let sent_to_failed = &failed.records[1];
    assert_eq!(sent_to_failed.span, span_of(&failed).id);
    assert_eq!(
        beyond_required(&sent_to_failed.attributes, CONTENT_REQUIRED),
        [
            key::LABLET_TURN,
            key::GEN_AI_SYSTEM_INSTRUCTIONS,
            key::GEN_AI_INPUT_MESSAGES,
        ],
        "an attempt that failed answered nothing"
    );
    let sent = parsed(held(
        &answered.records[0].attributes,
        key::GEN_AI_INPUT_MESSAGES,
    ));
    assert_eq!(
        parsed(held(&sent_to_failed.attributes, key::GEN_AI_INPUT_MESSAGES)),
        sent
    );
    let roles: Vec<_> = sent
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["user", "assistant", "tool"]);
    assert_eq!(
        sent[2]["parts"][0]["response"]["content"][0]["text"],
        "2 passed"
    );
}
