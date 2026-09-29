//! O1 and O2, as far as they're of spans, and the clause of E15 that's of
//! the chat span.

use lablet_conformance::observer::assert_the_wide_event_is_declared;
use lablet_conformance::otlp::{SpanKind, Status};
use lablet_model::{RunLabels, StopReason, Usage};
use lablet_telemetry_registry::attribute as key;
use lablet_telemetry_registry::signals::{
    EVENT_GEN_AI_CLIENT_OPERATION_EXCEPTION_KEYS, EVENT_GEN_AI_CLIENT_OPERATION_EXCEPTION_REQUIRED,
    EVENT_LABLET_RETRY_KEYS, EVENT_LABLET_RETRY_REQUIRED, SPAN_LABLET_CHAT_KEYS,
    SPAN_LABLET_CHAT_REQUIRED, SPAN_LABLET_EXECUTE_TOOL_KEYS, SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
    SPAN_LABLET_INVOKE_AGENT_KEYS, SPAN_LABLET_INVOKE_AGENT_REQUIRED,
};
use serde_json::json;

use crate::harness::{
    CONFIG_DIGEST, FAILS_CALLS_ENDS, MODEL, RUN, STARTED_UNIX_MS, VERSION, assert_declared, count,
    sum, traced,
};

fn labelled() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: Some("terse-tool-descriptions".to_owned()),
        trial: Some("3".to_owned()),
    }
}

/// When a span started and how long it lasted, in milliseconds into the
/// run.
fn timing(span: &lablet_conformance::otlp::Span) -> (u64, u64) {
    (
        span.start_unix_nano / 1_000_000 - STARTED_UNIX_MS,
        span.duration_ms(),
    )
}

// O2

#[tokio::test(start_paused = true)]
async fn a_run_is_read_back_as_a_root_span_with_a_span_beneath_it_for_each_attempt_and_each_call() {
    let traced = traced("o2-spans", FAILS_CALLS_ENDS, |_| {}).await;

    let root = traced.root();
    assert_eq!(root.name, "invoke_agent lablet");
    assert_eq!(root.kind, SpanKind::Internal);
    assert_eq!(root.parent_span_id, None);
    assert_declared(
        "the root span",
        &root.attributes,
        SPAN_LABLET_INVOKE_AGENT_REQUIRED,
        SPAN_LABLET_INVOKE_AGENT_KEYS,
    );

    let chats = traced.chats();
    assert_eq!(chats.len(), 4);
    for chat in &chats {
        assert_eq!(chat.name, format!("chat {MODEL}"));
        assert_eq!(chat.kind, SpanKind::Client);
        assert_declared(
            "a chat span",
            &chat.attributes,
            SPAN_LABLET_CHAT_REQUIRED,
            SPAN_LABLET_CHAT_KEYS,
        );
    }

    let tools = traced.tools();
    let called: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(
        called,
        [
            "execute_tool bash",
            "execute_tool read_file",
            "execute_tool no_such_tool"
        ]
    );
    for tool in &tools {
        assert_eq!(tool.kind, SpanKind::Internal);
        assert_declared(
            "a tool span",
            &tool.attributes,
            SPAN_LABLET_EXECUTE_TOOL_REQUIRED,
            SPAN_LABLET_EXECUTE_TOOL_KEYS,
        );
    }

    assert_eq!(traced.exported.spans.len(), 1 + 4 + 3);
    for span in traced.chats().iter().chain(&traced.tools()) {
        assert_eq!(span.trace_id, root.trace_id);
        assert_eq!(span.parent_span_id.as_ref(), Some(&root.span_id));
        assert_eq!(span.flags & 0xff, 1, "every span says it was sampled");
    }
}

#[tokio::test(start_paused = true)]
async fn a_run_has_one_wide_event_in_the_trace_and_the_context_of_its_root_span() {
    let traced = traced("o2-wide", FAILS_CALLS_ENDS, |_| {}).await;

    let (wide, root) = (traced.wide(), traced.root());
    assert_eq!(wide.trace_id, root.trace_id);
    assert_eq!(wide.span_id, root.span_id);
    assert_eq!(wide.flags, 1);
    assert_eq!(wide.time_unix_nano, root.end_unix_nano);
    assert_eq!(wide.severity_text, "INFO");
    assert_the_wide_event_is_declared(wide);
    assert_eq!(
        wide.attributes[key::LABLET_TELEMETRY_DROPPED_RECORDS],
        json!(0)
    );
}

// O1

#[tokio::test(start_paused = true)]
async fn every_line_is_an_export_from_the_resource_the_composer_added_to() {
    let traced = traced("o1-resource", FAILS_CALLS_ENDS, |settings| {
        settings.capture_content = true;
    })
    .await;

    let exported = &traced.exported;
    assert!(
        exported.lines >= 3,
        "spans, log records, and the wide event"
    );
    let lines: Vec<_> = traced.written.lines().collect();
    assert_eq!(lines.len(), exported.lines);
    for line in lines {
        assert!(
            line.starts_with("{\"resourceSpans\":[") || line.starts_with("{\"resourceLogs\":["),
            "{line}"
        );
    }
    assert!(traced.written.ends_with('\n'));

    let resources = exported
        .spans
        .iter()
        .map(|span| (&span.resource, &span.scope))
        .chain(
            exported
                .records
                .iter()
                .map(|record| (&record.resource, &record.scope)),
        );
    for (resource, scope) in resources {
        assert_eq!(resource[key::SERVICE_NAME], "lablet");
        assert_eq!(resource[key::SERVICE_VERSION], VERSION);
        assert_eq!(resource[key::TELEMETRY_SDK_NAME], "opentelemetry");
        assert_eq!(resource[key::TELEMETRY_SDK_LANGUAGE], "rust");
        assert!(resource[key::TELEMETRY_SDK_VERSION].is_string());
        assert_eq!(resource["team"], "evals");
        assert_eq!(resource["deployment.environment.name"], "ci");
        assert_eq!(resource.len(), 7);
        assert_eq!(scope.name, "lablet");
        assert_eq!(scope.version, VERSION);
        assert_eq!(scope.schema_url, lablet_telemetry_registry::SCHEMA_URL);
    }
}

#[tokio::test(start_paused = true)]
async fn the_join_keys_and_the_run_s_labels_are_on_every_span_and_every_record() {
    let traced = traced("o1-join", FAILS_CALLS_ENDS, |settings| {
        settings.capture_content = true;
        settings.labels = labelled();
    })
    .await;

    let exported = &traced.exported;
    assert_eq!(exported.spans.len(), 8);
    assert_eq!(
        exported.records.len(),
        1 + 1 + 4 + 3 + 1,
        "the tools, the exception, a record of each attempt and each call, and the wide event"
    );
    let signals = exported
        .spans
        .iter()
        .map(|span| (&span.name, &span.attributes))
        .chain(
            exported
                .records
                .iter()
                .map(|record| (&record.event_name, &record.attributes)),
        );
    for (signal, attributes) in signals {
        for (key, value) in [
            (key::GEN_AI_CONVERSATION_ID, RUN),
            (key::SESSION_ID, RUN),
            (key::LABLET_CONFIG_DIGEST, CONFIG_DIGEST),
            (key::LABLET_TASK_ID, "fix-failing-test"),
            (key::LABLET_EXPERIMENT_ID, "terse-tool-descriptions"),
            (key::LABLET_TRIAL, "3"),
        ] {
            assert_eq!(
                attributes.get(key),
                Some(&json!(value)),
                "{key} of {signal}"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_run_without_labels_leaves_them_off_every_span_and_every_record() {
    let traced = traced("o1-no-labels", FAILS_CALLS_ENDS, |settings| {
        settings.capture_content = true;
    })
    .await;

    let exported = &traced.exported;
    let attributes = exported
        .spans
        .iter()
        .map(|span| &span.attributes)
        .chain(exported.records.iter().map(|record| &record.attributes));
    for attributes in attributes {
        for key in [
            key::LABLET_TASK_ID,
            key::LABLET_EXPERIMENT_ID,
            key::LABLET_TRIAL,
        ] {
            assert_eq!(attributes.get(key), None, "{key}");
        }
        assert_eq!(attributes[key::GEN_AI_CONVERSATION_ID], RUN);
    }
}

#[tokio::test(start_paused = true)]
async fn the_chat_spans_of_a_run_number_its_turns_and_its_retries() {
    let traced = traced("o1-chats", FAILS_CALLS_ENDS, |_| {}).await;

    let summary = &traced.finished.summary;
    assert_eq!((summary.outcome.turns, summary.provider.retries), (3, 1));
    let chats = traced.chats();
    assert_eq!(chats.len(), 3 + 1);
    let numbered: Vec<_> = chats
        .iter()
        .map(|chat| {
            assert_eq!(chat.attributes[key::LABLET_CHAT_PURPOSE], "turn");
            (
                count(&chat.attributes, key::LABLET_TURN),
                count(&chat.attributes, key::LABLET_ATTEMPT),
            )
        })
        .collect();
    assert_eq!(numbered, [(1, 1), (1, 2), (2, 1), (3, 1)]);
    assert_eq!(count(&traced.root().attributes, key::LABLET_RUN_TURNS), 3);
}

#[tokio::test(start_paused = true)]
async fn a_run_that_ended_on_a_failed_call_has_one_chat_span_more_than_its_turns_and_retries() {
    let traced = traced(
        "o1-ended-on-failure",
        r"
- response:
    content:
      - tool_use: { id: call_1, name: read_file, input: { json: { path: src/parser.rs } } }
    usage: { input_tokens: 1000, output_tokens: 50 }
    finish: tool_use
- error: { kind: retryable, message: 529 overloaded }
- error: { kind: retryable, message: 529 overloaded }
",
        |settings| settings.max_retries = 1,
    )
    .await;

    let summary = &traced.finished.summary;
    assert_eq!(summary.outcome.stop_reason(), StopReason::RetriesExhausted);
    assert_eq!((summary.outcome.turns, summary.provider.retries), (1, 1));
    assert_eq!(traced.chats().len(), 1 + 1 + 1);
    let root = traced.root();
    assert_eq!(root.attributes[key::ERROR_TYPE], "retries_exhausted");
    assert_eq!(
        root.attributes[key::LABLET_RUN_STOP_REASON],
        "retries_exhausted"
    );
    assert_eq!(root.status, Status::Error("529 overloaded".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn the_counts_of_the_spans_sum_to_what_the_run_measured() {
    let traced = traced("o1-sums", FAILS_CALLS_ENDS, |_| {}).await;

    let summary = &traced.finished.summary;
    let (answered, failed): (Vec<_>, Vec<_>) = traced
        .chats()
        .into_iter()
        .partition(|chat| !chat.attributes.contains_key(key::ERROR_TYPE));
    let spent = |chats: &[&lablet_conformance::otlp::Span]| {
        sum(chats, key::GEN_AI_USAGE_INPUT_TOKENS).map(|input_tokens| Usage {
            input_tokens,
            output_tokens: sum(chats, key::GEN_AI_USAGE_OUTPUT_TOKENS).unwrap(),
            reasoning_output_tokens: sum(chats, key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS),
            cache_read_tokens: sum(chats, key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS),
            cache_write_tokens: sum(chats, key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS),
        })
    };
    assert_eq!(spent(&answered), Some(summary.outcome.usage));
    assert_eq!(
        summary.outcome.usage,
        Usage {
            input_tokens: 3_300,
            output_tokens: 100,
            reasoning_output_tokens: Some(5),
            cache_read_tokens: Some(200),
            cache_write_tokens: None,
        }
    );
    assert_eq!(spent(&failed), summary.failed_usage);
    assert_eq!(
        summary.failed_usage,
        Some(Usage {
            input_tokens: 800,
            cache_read_tokens: Some(600),
            ..Usage::default()
        })
    );
    let reasons: Vec<_> = answered
        .iter()
        .map(|chat| chat.attributes[key::GEN_AI_RESPONSE_FINISH_REASONS].clone())
        .collect();
    assert_eq!(
        reasons,
        [
            json!(["tool_use"]),
            json!(["tool_use"]),
            json!(["end_turn"])
        ]
    );

    let tools = traced.tools();
    assert_eq!(tools.len() as u64, summary.outcome.tool_calls);
    let errors = tools
        .iter()
        .filter(|tool| tool.attributes[key::LABLET_TOOL_IS_ERROR] == true)
        .count();
    assert_eq!(errors as u64, summary.tool_calls.errors);
    let unknown = tools
        .iter()
        .filter(|tool| tool.attributes[key::LABLET_TOOL_STATUS] == "unknown")
        .count();
    assert_eq!(unknown as u64, summary.tool_calls.unknown);
    assert_eq!(
        sum(&tools, key::LABLET_TOOL_INPUT_BYTES),
        Some(summary.tool_calls.input_bytes)
    );
    assert_eq!(
        sum(&tools, key::LABLET_TOOL_OUTPUT_BYTES),
        Some(summary.tool_calls.output_bytes)
    );
    for (name, stats) in &summary.per_tool {
        let of_the_tool: Vec<_> = tools
            .iter()
            .copied()
            .filter(|tool| tool.attributes[key::GEN_AI_TOOL_NAME] == name.as_str())
            .collect();
        assert_eq!(of_the_tool.len() as u64, stats.calls, "{name}");
        assert_eq!(
            of_the_tool
                .iter()
                .map(|tool| tool.duration_ms())
                .sum::<u64>(),
            stats.latency_ms,
            "{name}"
        );
    }
    assert_eq!(summary.per_tool.len(), 2);

    let root = &traced.root().attributes;
    assert_eq!(count(root, key::LABLET_TOOL_CALLS_TOTAL), 3);
    assert_eq!(count(root, key::GEN_AI_USAGE_INPUT_TOKENS), 3_300);
    assert_eq!(count(root, key::GEN_AI_USAGE_OUTPUT_TOKENS), 100);
    assert_eq!(count(root, key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS), 5);
    assert_eq!(count(root, key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS), 200);
}

#[tokio::test(start_paused = true)]
async fn the_spans_last_as_long_as_the_run_measured_and_start_when_it_says() {
    let traced = traced("o1-latency", FAILS_CALLS_ENDS, |_| {}).await;

    let summary = &traced.finished.summary;
    let chats: Vec<_> = traced.chats().into_iter().map(timing).collect();
    assert_eq!(
        chats,
        [(0, 40), (2_040, 250), (3_590, 100), (3_690, 120)],
        "the attempt that failed, the wait the server asked for, and the tool phase between"
    );
    assert_eq!(
        chats.iter().map(|(_, lasted)| lasted).sum::<u64>(),
        summary.provider.latency.total_ms()
    );
    assert_eq!(
        chats.iter().map(|(_, lasted)| *lasted).max(),
        Some(summary.provider.latency.max_ms())
    );

    let tools: Vec<_> = traced.tools().into_iter().map(timing).collect();
    assert_eq!(tools, [(2_290, 1_000), (3_290, 300), (3_690, 0)]);
    assert_eq!(
        tools.iter().map(|(_, lasted)| lasted).sum::<u64>(),
        summary.tool_calls.latency_ms
    );
    let outcomes: Vec<_> = traced
        .finished
        .transcript
        .turns()
        .iter()
        .flat_map(lablet_model::Turn::tool_calls)
        .map(|outcome| (outcome.started_ms, outcome.latency_ms))
        .collect();
    assert_eq!(
        tools, outcomes,
        "a tool span is timed as the transcript times its call"
    );

    assert_eq!(
        timing(traced.root()),
        (0, summary.outcome.duration_ms),
        "the root span is the run"
    );
    assert_eq!(summary.outcome.duration_ms, 3_810);
}

// The spans, one by one

#[tokio::test(start_paused = true)]
async fn a_chat_span_says_what_the_attempt_was_asked_and_what_it_answered() {
    let traced = traced("chat-span", FAILS_CALLS_ENDS, |settings| {
        settings.request.temperature = Some(0.2);
        settings.request.seed = Some(7);
    })
    .await;

    let answered = traced.chats()[1];
    assert_eq!(
        serde_json::to_value(&answered.attributes).unwrap(),
        json!({
            "gen_ai.conversation.id": RUN,
            "session.id": RUN,
            "lablet.config.digest": CONFIG_DIGEST,
            "gen_ai.operation.name": "chat",
            "gen_ai.provider.name": "fake",
            "gen_ai.request.model": MODEL,
            "gen_ai.request.max_tokens": 4_096,
            "gen_ai.request.temperature": 0.2,
            "gen_ai.request.seed": 7,
            "lablet.chat.purpose": "turn",
            "lablet.turn": 1,
            "lablet.attempt": 2,
            "lablet.request.bytes": traced_request_bytes(&traced, 1),
            "gen_ai.response.id": "msg_01",
            "gen_ai.response.model": "scripted-2026-09",
            "gen_ai.response.finish_reasons": ["tool_use"],
            "gen_ai.usage.input_tokens": 1_000,
            "gen_ai.usage.output_tokens": 50,
            "gen_ai.usage.cache_read.input_tokens": 200,
        })
    );
    assert_eq!(answered.status, Status::Unset);
    assert!(answered.events.is_empty());
}

/// The size of the request of a run's attempt, which grows with the
/// conversation and is the same for an attempt and its retry.
fn traced_request_bytes(traced: &crate::harness::Traced, chat: usize) -> u64 {
    let chats = traced.chats();
    let bytes: Vec<_> = chats
        .iter()
        .map(|chat| count(&chat.attributes, key::LABLET_REQUEST_BYTES))
        .collect();
    assert_eq!(
        bytes[0], bytes[1],
        "a retry sends what the attempt before it sent"
    );
    assert!(bytes[1] < bytes[2] && bytes[2] < bytes[3], "{bytes:?}");
    bytes[chat]
}

#[tokio::test(start_paused = true)]
async fn the_span_of_a_failed_attempt_says_how_it_failed_and_what_the_loop_did_next() {
    let traced = traced("failed-chat-span", FAILS_CALLS_ENDS, |_| {}).await;

    let failed = traced.chats()[0];
    assert_eq!(failed.attributes[key::ERROR_TYPE], "retryable");
    assert_eq!(failed.status, Status::Error("529 overloaded".to_owned()));
    assert_eq!(
        count(&failed.attributes, key::GEN_AI_USAGE_INPUT_TOKENS),
        800
    );
    assert_eq!(
        count(
            &failed.attributes,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS
        ),
        600
    );
    assert_eq!(failed.events.len(), 1);
    let retry = &failed.events[0];
    assert_eq!(retry.name, "lablet.retry");
    assert_eq!(retry.time_unix_nano, failed.end_unix_nano);
    assert_declared(
        "the retry event",
        &retry.attributes,
        EVENT_LABLET_RETRY_REQUIRED,
        EVENT_LABLET_RETRY_KEYS,
    );
    assert_eq!(
        serde_json::to_value(&retry.attributes).unwrap(),
        json!({
            "lablet.attempt": 1,
            "lablet.retry.will_retry": true,
            "lablet.retry.backoff_ms": 2_000,
        })
    );

    let exceptions = traced
        .exported
        .records_of("gen_ai.client.operation.exception");
    assert_eq!(exceptions.len(), 1);
    let exception = exceptions[0];
    assert_eq!(exception.span_id, failed.span_id);
    assert_eq!(exception.trace_id, failed.trace_id);
    assert_eq!(exception.time_unix_nano, failed.end_unix_nano);
    assert_eq!(exception.severity_number, 13);
    assert_eq!(exception.severity_text, "WARN");
    assert_declared(
        "the exception record",
        &exception.attributes,
        EVENT_GEN_AI_CLIENT_OPERATION_EXCEPTION_REQUIRED,
        EVENT_GEN_AI_CLIENT_OPERATION_EXCEPTION_KEYS,
    );
    assert_eq!(exception.attributes[key::EXCEPTION_TYPE], "retryable");
    assert_eq!(
        exception.attributes[key::EXCEPTION_MESSAGE],
        "529 overloaded"
    );
    assert_eq!(count(&exception.attributes, key::LABLET_ATTEMPT), 1);
}

// E15, the clause of the chat span

#[tokio::test(start_paused = true)]
async fn the_chat_span_of_a_call_whose_key_was_rejected_names_auth_as_its_error() {
    let traced = traced(
        "e15",
        r"
- error: { kind: auth, message: the key was rejected, latency: 15ms }
- response: { content: [{ text: never played }], finish: end_turn }
",
        |_| {},
    )
    .await;

    let outcome = &traced.finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::ProviderError);
    let chats = traced.chats();
    assert_eq!(chats.len(), 1, "a rejected key is never tried again");
    let chat = chats[0];
    assert_eq!(chat.attributes[key::ERROR_TYPE], "auth");
    assert_eq!(
        chat.status,
        Status::Error("the key was rejected".to_owned())
    );
    assert_eq!(timing(chat), (0, 15));
    assert_eq!(
        chat.events[0].attributes[key::LABLET_RETRY_WILL_RETRY],
        false
    );
    assert_eq!(
        chat.events[0].attributes.get(key::LABLET_RETRY_BACKOFF_MS),
        None
    );
    assert_declared(
        "the chat span",
        &chat.attributes,
        SPAN_LABLET_CHAT_REQUIRED,
        SPAN_LABLET_CHAT_KEYS,
    );
    let exception = traced
        .exported
        .records_of("gen_ai.client.operation.exception")[0];
    assert_eq!(exception.attributes[key::EXCEPTION_TYPE], "auth");
    assert_eq!(exception.span_id, chat.span_id);
    let root = traced.root();
    assert_eq!(root.attributes[key::ERROR_TYPE], "provider_error");
    assert_eq!(
        root.status,
        Status::Error("the key was rejected".to_owned())
    );
    assert_eq!(count(&root.attributes, key::LABLET_RUN_TURNS), 0);
    assert!(traced.tools().is_empty());
}

// O14, as far as it's of spans

#[tokio::test(start_paused = true)]
async fn a_count_the_provider_did_not_report_is_on_no_span() {
    let traced = traced(
        "o14",
        r"
- response:
    content: [{ text: Done. }]
    usage: { input_tokens: 1200, output_tokens: 30 }
    finish: end_turn
",
        |_| {},
    )
    .await;

    assert_eq!(
        traced.finished.summary.outcome.usage,
        Usage {
            input_tokens: 1_200,
            output_tokens: 30,
            ..Usage::default()
        }
    );
    for span in [traced.chats()[0], traced.root()] {
        for key in [
            key::GEN_AI_USAGE_REASONING_OUTPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS,
            key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS,
        ] {
            assert_eq!(span.attributes.get(key), None, "{key} of {}", span.name);
        }
        assert_eq!(
            count(&span.attributes, key::GEN_AI_USAGE_INPUT_TOKENS),
            1_200
        );
        assert_eq!(count(&span.attributes, key::GEN_AI_USAGE_OUTPUT_TOKENS), 30);
    }
    assert_eq!(traced.root().status, Status::Unset);
    assert_eq!(traced.root().attributes.get(key::ERROR_TYPE), None);
}

#[tokio::test(start_paused = true)]
async fn a_count_of_zero_that_the_provider_reported_is_a_count() {
    let traced = traced(
        "o14-zero",
        r"
- response:
    content: [{ text: Done. }]
    usage: { input_tokens: 1200, output_tokens: 30, cache_read_tokens: 0 }
    finish: end_turn
",
        |_| {},
    )
    .await;

    for span in [traced.chats()[0], traced.root()] {
        assert_eq!(
            span.attributes
                .get(key::GEN_AI_USAGE_CACHE_READ_INPUT_TOKENS),
            Some(&json!(0))
        );
        assert_eq!(
            span.attributes
                .get(key::GEN_AI_USAGE_CACHE_WRITE_INPUT_TOKENS),
            None
        );
    }
}

// Tool spans

#[tokio::test(start_paused = true)]
async fn a_tool_span_says_what_was_called_and_what_became_of_the_call() {
    let traced = traced("tool-spans", FAILS_CALLS_ENDS, |_| {}).await;

    let tools = traced.tools();
    assert_eq!(
        serde_json::to_value(&tools[0].attributes).unwrap(),
        json!({
            "gen_ai.conversation.id": RUN,
            "session.id": RUN,
            "lablet.config.digest": CONFIG_DIGEST,
            "gen_ai.operation.name": "execute_tool",
            "gen_ai.tool.name": "bash",
            "gen_ai.tool.call.id": "call_1",
            "gen_ai.tool.type": "function",
            "gen_ai.tool.description": "Runs a command.",
            "lablet.tool.source": "builtin",
            "lablet.turn": 1,
            "lablet.tool.status": "ok",
            "lablet.tool.input.bytes": 32,
            "lablet.tool.output.bytes": 25,
            "lablet.tool.output.truncated": false,
            "lablet.tool.is_error": false,
        })
    );
    assert_eq!(tools[0].status, Status::Unset);

    let unknown = tools[2];
    assert_eq!(unknown.attributes[key::ERROR_TYPE], "unknown");
    assert_eq!(unknown.attributes[key::LABLET_TOOL_STATUS], "unknown");
    assert_eq!(unknown.attributes[key::LABLET_TOOL_IS_ERROR], true);
    assert_eq!(count(&unknown.attributes, key::LABLET_TURN), 2);
    for key in [
        key::GEN_AI_TOOL_TYPE,
        key::GEN_AI_TOOL_DESCRIPTION,
        key::LABLET_TOOL_SOURCE,
    ] {
        assert_eq!(unknown.attributes.get(key), None, "{key}");
    }
    assert_eq!(unknown.status, Status::Error(String::new()));
}

#[tokio::test(start_paused = true)]
async fn the_span_an_executor_was_handed_is_the_span_its_call_was_exported_as() {
    let traced = traced("propagated", FAILS_CALLS_ENDS, |_| {}).await;

    let tools = traced.tools();
    let handed: Vec<_> = traced
        .propagated
        .iter()
        .map(|(call, context)| {
            let context = context.as_ref().unwrap();
            assert_eq!(context.tracestate, None);
            (call.as_str().to_owned(), context.traceparent.clone())
        })
        .collect();
    let exported: Vec<_> = tools[..2]
        .iter()
        .map(|tool| {
            (
                tool.attributes[key::GEN_AI_TOOL_CALL_ID]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                format!("00-{}-{}-01", tool.trace_id, tool.span_id),
            )
        })
        .collect();
    assert_eq!(handed, exported);
    assert_eq!(handed.len(), 2, "the call to no tool reached no executor");
}
