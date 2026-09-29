use lablet_model::{TokenCounts, ToolCallId, ToolInput, ToolName, ToolUse};
use lablet_run::ERROR_MESSAGE_MAX_BYTES;
use serde_json::json;

use super::*;

const NAME: &str = "scripts/smoke.yaml";

fn yaml(text: &str) -> Result<Script, ScriptError> {
    Script::read(ScriptSource {
        name: NAME,
        text,
        format: ScriptFormat::Yaml,
    })
}

fn json(text: &str) -> Result<Script, ScriptError> {
    Script::read(ScriptSource {
        name: NAME,
        text,
        format: ScriptFormat::Json,
    })
}

/// What's wrong with a YAML script that's refused.
fn fault(text: &str) -> ScriptFault {
    let ScriptError { script, fault } = yaml(text).unwrap_err();
    assert_eq!(script, NAME);
    fault
}

/// The fault of an entry, as a refusal of entry `entry` for `reason`.
fn in_entry(entry: usize, reason: &str) -> ScriptFault {
    ScriptFault::Entry {
        entry,
        reason: reason.to_owned(),
    }
}

/// The fault of a block, as a refusal of block `block` of entry `entry`.
fn in_block(entry: usize, block: usize, reason: &str) -> ScriptFault {
    ScriptFault::Block {
        entry,
        block,
        reason: reason.to_owned(),
    }
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn text(text: &str) -> model::ContentBlock {
    model::ContentBlock::Text(text.to_owned())
}

fn call(id: &str, name: &str, input: ToolInput) -> model::ContentBlock {
    model::ContentBlock::ToolUse(ToolUse {
        id: ToolCallId::new(id).unwrap(),
        name: ToolName::new(name).unwrap(),
        input,
    })
}

/// A response that reported no usage and no ids.
fn says(content: Vec<model::ContentBlock>, finish: &str) -> ProviderResponse {
    ProviderResponse::new(
        content,
        model::Usage::default(),
        FinishReason::from(finish.to_owned()),
        None,
        None,
    )
    .unwrap()
}

/// The one entry of a script that holds one.
fn only(script: &Script) -> &Entry {
    assert_eq!(script.entries(), 1);
    script.entry(0).unwrap()
}

/// The failure the script's one entry injects.
fn injected(script: &Script) -> &ProviderError {
    only(script).answer.as_ref().unwrap_err()
}

/// The response the script's one entry answers with.
fn answered(script: &Script) -> &ProviderResponse {
    only(script).answer.as_ref().unwrap()
}

const EVERYTHING: &str = r#"
- response:
    content:
      - thinking:
          text: The user wants the files listed.
          signature: c2lnbmVk
      - thinking:
          text: ""
      - redacted_thinking:
          data: ZW5jcnlwdGVk
      - text: I'll list the files.
      - tool_use:
          id: call_1
          name: bash
          input:
            json: { command: ls, flags: [-l, -a] }
      - tool_use:
          id: call_2
          name: read_file
          input:
            unparsed: '{"path": "README'
      - opaque:
          provider: openai
          payload: { type: reasoning, encrypted_content: abc }
    usage:
      input_tokens: 1200
      output_tokens: 80
      reasoning_output_tokens: 30
      cache_read_tokens: 1000
      cache_write_tokens: 0
    finish: tool_use
    response_id: msg_01
    response_model: scripted-2026-09
    latency: 250ms
"#;

fn everything() -> Entry {
    Entry {
        latency: ms(250),
        answer: Ok(ProviderResponse::new(
            vec![
                model::ContentBlock::Thinking {
                    text: "The user wants the files listed.".to_owned(),
                    signature: Some("c2lnbmVk".to_owned()),
                },
                model::ContentBlock::Thinking {
                    text: String::new(),
                    signature: None,
                },
                model::ContentBlock::RedactedThinking {
                    data: "ZW5jcnlwdGVk".to_owned(),
                },
                text("I'll list the files."),
                call(
                    "call_1",
                    "bash",
                    ToolInput::Json(json!({ "command": "ls", "flags": ["-l", "-a"] })),
                ),
                call(
                    "call_2",
                    "read_file",
                    ToolInput::Unparsed(r#"{"path": "README"#.to_owned()),
                ),
                model::ContentBlock::Opaque {
                    provider: model::ProviderKind::Openai,
                    payload: json!({ "type": "reasoning", "encrypted_content": "abc" }),
                },
            ],
            model::Usage::from_inclusive(TokenCounts {
                input: 1_200,
                output: 80,
                reasoning: Some(30),
                cache_read: Some(1_000),
                cache_write: Some(0),
            }),
            FinishReason::ToolUse,
            Some("msg_01".to_owned()),
            Some("scripted-2026-09".to_owned()),
        )
        .unwrap()),
    }
}

#[test]
fn a_response_is_read_with_every_block_and_everything_else_it_states() {
    let script = yaml(EVERYTHING).unwrap();

    assert_eq!(script.name(), NAME);
    assert_eq!(only(&script), &everything());
}

#[test]
fn a_script_reads_the_same_from_json_as_from_yaml() {
    let text = r#"[
        {"response": {
            "content": [
                {"thinking": {"text": "The user wants the files listed.", "signature": "c2lnbmVk"}},
                {"thinking": {"text": "", "signature": null}},
                {"redacted_thinking": {"data": "ZW5jcnlwdGVk"}},
                {"text": "I'll list the files."},
                {"tool_use": {"id": "call_1", "name": "bash",
                    "input": {"json": {"command": "ls", "flags": ["-l", "-a"]}}}},
                {"tool_use": {"id": "call_2", "name": "read_file",
                    "input": {"unparsed": "{\"path\": \"README"}}},
                {"opaque": {"provider": "openai",
                    "payload": {"type": "reasoning", "encrypted_content": "abc"}}}
            ],
            "usage": {
                "input_tokens": 1200,
                "output_tokens": 80,
                "reasoning_output_tokens": 30,
                "cache_read_tokens": 1000,
                "cache_write_tokens": 0
            },
            "finish": "tool_use",
            "response_id": "msg_01",
            "response_model": "scripted-2026-09",
            "latency": "250ms"
        }}
    ]"#;

    assert_eq!(json(text).unwrap(), yaml(EVERYTHING).unwrap());
}

#[test]
fn entries_are_kept_in_the_order_they_are_written() {
    let script = yaml(
        r"
- response: { content: [{ text: first }], finish: end_turn }
- error: { kind: fatal, message: second }
- response: { content: [{ text: third }], finish: end_turn }
",
    )
    .unwrap();

    assert_eq!(script.entries(), 3);
    assert_eq!(
        script.entry(0).unwrap().answer,
        Ok(says(vec![text("first")], "end_turn"))
    );
    assert_eq!(
        script.entry(1).unwrap().answer,
        Err(ProviderError::new(ProviderErrorKind::Fatal, "second"))
    );
    assert_eq!(
        script.entry(2).unwrap().answer,
        Ok(says(vec![text("third")], "end_turn"))
    );
    assert_eq!(script.entry(3), None);
}

#[test]
fn what_a_response_leaves_out_reads_as_nothing() {
    let script = yaml("- response: { content: [], finish: end_turn }").unwrap();

    assert_eq!(
        only(&script),
        &Entry {
            latency: Duration::ZERO,
            answer: Ok(says(Vec::new(), "end_turn")),
        }
    );
    assert_eq!(answered(&script).usage, model::Usage::default());
}

#[test]
fn a_key_that_may_be_left_out_may_be_null() {
    let script = yaml(
        r"
- response:
    content: []
    finish: end_turn
    response_id: null
    response_model: ~
    latency: null
- error:
    kind: fatal
    message: null
    usage: null
    retry_after: null
    latency: null
",
    )
    .unwrap();

    assert_eq!(
        script.entry(0).unwrap(),
        &Entry {
            latency: Duration::ZERO,
            answer: Ok(says(Vec::new(), "end_turn")),
        }
    );
    assert_eq!(
        script.entry(1).unwrap(),
        &Entry {
            latency: Duration::ZERO,
            answer: Err(ProviderError::new(
                ProviderErrorKind::Fatal,
                r#"entry 2 of script "scripts/smoke.yaml" injects a failure of kind fatal"#
            )),
        }
    );
}

/// O14, the script's half: a count the script leaves out is one the
/// provider didn't report, and a count of zero is a count.
#[test]
fn a_count_a_usage_leaves_out_was_not_reported_and_a_zero_was() {
    let bare = yaml(
        "- response: { content: [], finish: end_turn, usage: { input_tokens: 12, output_tokens: 3 } }",
    )
    .unwrap();
    let zero = yaml(
        "- response: { content: [], finish: end_turn, usage: { input_tokens: 12, output_tokens: 3, cache_read_tokens: 0 } }",
    )
    .unwrap();

    assert_eq!(
        answered(&bare).usage,
        model::Usage {
            input_tokens: 12,
            output_tokens: 3,
            reasoning_output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }
    );
    assert_eq!(
        answered(&zero).usage,
        model::Usage {
            cache_read_tokens: Some(0),
            ..answered(&bare).usage
        }
    );
}

#[test]
fn a_finish_reason_is_read_in_whichever_providers_spelling_it_is_written() {
    for (written, reason) in [
        ("end_turn", FinishReason::EndTurn),
        ("stop", FinishReason::EndTurn),
        ("tool_calls", FinishReason::ToolUse),
        ("length", FinishReason::MaxTokens),
        ("model_context_window_exceeded", FinishReason::ContextWindow),
        ("content_filter", FinishReason::Refusal),
    ] {
        let script = yaml(&format!("- response: {{ content: [], finish: {written} }}")).unwrap();

        assert_eq!(answered(&script).finish, reason, "{written}");
    }
}

#[test]
fn a_finish_reason_no_provider_spells_is_kept_as_it_is_written() {
    let script = yaml("- response: { content: [], finish: pause_turn }").unwrap();

    assert_eq!(answered(&script).finish.as_str(), "pause_turn");
    assert!(matches!(answered(&script).finish, FinishReason::Other(_)));
}

#[test]
fn an_error_is_read_with_its_kind_its_message_its_usage_and_its_hint() {
    let script = yaml(
        r"
- error:
    kind: retryable
    message: 529 overloaded
    usage: { input_tokens: 1200, output_tokens: 0, cache_read_tokens: 1000 }
    retry_after: 2s
    latency: 40ms
",
    )
    .unwrap();

    assert_eq!(
        only(&script),
        &Entry {
            latency: ms(40),
            answer: Err(
                ProviderError::new(ProviderErrorKind::Retryable, "529 overloaded")
                    .with_usage(model::Usage::from_inclusive(TokenCounts {
                        input: 1_200,
                        output: 0,
                        reasoning: None,
                        cache_read: Some(1_000),
                        cache_write: None,
                    }))
                    .with_retry_after(Duration::from_secs(2))
            ),
        }
    );
}

/// The spelling a script writes, for every kind the domain has. The match
/// has no arm for the rest, so a kind the domain gains doesn't compile here
/// until a script can write it.
const fn written(kind: ProviderErrorKind) -> (&'static str, Kind) {
    match kind {
        ProviderErrorKind::Retryable => ("retryable", Kind::Retryable),
        ProviderErrorKind::ContextExhausted => ("context_exhausted", Kind::ContextExhausted),
        ProviderErrorKind::Auth => ("auth", Kind::Auth),
        ProviderErrorKind::Fatal => ("fatal", Kind::Fatal),
        ProviderErrorKind::Malformed => ("malformed", Kind::Malformed),
    }
}

#[test]
fn an_error_of_every_kind_is_written_as_a_runs_record_spells_the_kind() {
    for kind in [
        ProviderErrorKind::Retryable,
        ProviderErrorKind::ContextExhausted,
        ProviderErrorKind::Auth,
        ProviderErrorKind::Fatal,
        ProviderErrorKind::Malformed,
    ] {
        let (spelling, scripted) = written(kind);
        let script = yaml(&format!("- error: {{ kind: {spelling} }}")).unwrap();

        assert_eq!(spelling, kind.as_str());
        assert_eq!(ProviderErrorKind::from(scripted), kind);
        assert_eq!(injected(&script).kind, kind, "{spelling}");
    }
}

#[test]
fn an_error_without_a_message_says_which_entry_of_which_script_injected_it() {
    let script = yaml(
        r"
- response: { content: [], finish: end_turn }
- error: { kind: auth }
",
    )
    .unwrap();

    let failure = script.entry(1).unwrap().answer.as_ref().unwrap_err();

    assert_eq!(
        failure.message(),
        r#"entry 2 of script "scripts/smoke.yaml" injects a failure of kind auth"#
    );
    assert_eq!(failure.usage, None);
    assert_eq!(failure.retry_after, None);
}

#[test]
fn an_error_that_states_an_empty_usage_reported_one_of_nothing() {
    let script = yaml("- error: { kind: retryable, usage: {} }").unwrap();

    assert_eq!(injected(&script).usage, Some(model::Usage::default()));
}

#[test]
fn a_message_longer_than_the_ports_bound_is_cut_to_it() {
    let long = "x".repeat(ERROR_MESSAGE_MAX_BYTES + 1);

    let script = yaml(&format!("- error: {{ kind: fatal, message: {long} }}")).unwrap();

    assert_eq!(
        injected(&script).message(),
        "x".repeat(ERROR_MESSAGE_MAX_BYTES)
    );
}

#[test]
fn a_duration_is_written_as_the_configs_durations_are() {
    for (written, duration) in [
        ("0s", Duration::ZERO),
        ("250ms", ms(250)),
        ("2s", Duration::from_secs(2)),
        ("1m 30s", Duration::from_secs(90)),
    ] {
        let script = yaml(&format!(
            "- error: {{ kind: retryable, retry_after: {written}, latency: {written} }}"
        ))
        .unwrap();

        assert_eq!(only(&script).latency, duration, "{written}");
        assert_eq!(injected(&script).retry_after, Some(duration), "{written}");
    }
}

#[test]
fn a_word_that_older_yaml_read_as_a_boolean_is_the_word() {
    let script = yaml("- response: { content: [{ text: yes }], finish: end_turn }").unwrap();

    assert_eq!(answered(&script).content(), [text("yes")]);
}

#[test]
fn a_tool_call_id_may_be_used_again_by_a_later_response() {
    let script = yaml(
        r"
- response:
    content: [{ tool_use: { id: call_1, name: bash, input: { json: {} } } }]
    finish: tool_use
- response:
    content: [{ tool_use: { id: call_1, name: bash, input: { json: {} } } }]
    finish: tool_use
",
    )
    .unwrap();

    assert_eq!(script.entry(0), script.entry(1));
}

#[test]
fn a_refusal_names_the_script_and_says_where_in_it() {
    let refused = yaml(
        r"
- response: { content: [{ text: fine }], finish: end_turn }
- error: { kind: retryable }
- response:
    content:
      - text: fine
      - tool_use: { id: call_1, nme: bash, input: { json: {} } }
    finish: tool_use
",
    )
    .unwrap_err();

    assert_eq!(
        refused,
        ScriptError {
            script: NAME.to_owned(),
            fault: in_block(
                3,
                2,
                "unknown field `nme`, expected one of `id`, `name`, `input`"
            ),
        }
    );
    assert_eq!(
        refused.to_string(),
        r#"script "scripts/smoke.yaml": entry 3, block 2: unknown field `nme`, expected one of `id`, `name`, `input`"#
    );
}

#[test]
fn a_key_no_response_has_is_refused_and_never_read_as_a_default() {
    assert_eq!(
        fault("- response: { content: [], finsh: refusal, finish: end_turn }"),
        in_entry(
            1,
            "unknown field `finsh`, expected one of `content`, `usage`, `finish`, \
             `response_id`, `response_model`, `latency`"
        )
    );
    assert_eq!(
        fault("- response: { content: [], finish: end_turn, role: user }"),
        in_entry(
            1,
            "unknown field `role`, expected one of `content`, `usage`, `finish`, \
             `response_id`, `response_model`, `latency`"
        )
    );
}

#[test]
fn a_key_no_usage_has_is_refused_and_never_read_as_a_count_of_zero() {
    let reason = "unknown field `input_token`, expected one of `input_tokens`, \
                  `output_tokens`, `reasoning_output_tokens`, `cache_read_tokens`, \
                  `cache_write_tokens`";

    assert_eq!(
        fault("- response: { content: [], finish: end_turn, usage: { input_token: 12 } }"),
        in_entry(1, reason)
    );
    assert_eq!(
        fault("- error: { kind: retryable, usage: { input_token: 12 } }"),
        in_entry(1, reason)
    );
}

#[test]
fn a_key_no_error_has_is_refused() {
    assert_eq!(
        fault("- error: { kind: retryable, retry_afer: 2s }"),
        in_entry(
            1,
            "unknown field `retry_afer`, expected one of `kind`, `message`, `usage`, \
             `retry_after`, `latency`"
        )
    );
}

#[test]
fn a_key_no_block_has_is_refused_whatever_kind_of_block_it_is() {
    for (block, reason) in [
        (
            "tool_use: { id: a, name: bash, input: { json: {} }, cache: true }",
            "unknown field `cache`, expected one of `id`, `name`, `input`",
        ),
        (
            "thinking: { text: hm, signture: abc }",
            "unknown field `signture`, expected `text` or `signature`",
        ),
        (
            "redacted_thinking: { data: abc, text: hm }",
            "unknown field `text`, expected `data`",
        ),
        (
            "opaque: { provider: fake, payload: {}, kind: x }",
            "unknown field `kind`, expected `provider` or `payload`",
        ),
    ] {
        assert_eq!(
            fault(&format!(
                "- response: {{ content: [{{ {block} }}], finish: end_turn }}"
            )),
            in_block(1, 1, reason),
            "{block}"
        );
    }
}

#[test]
fn a_key_a_response_needs_is_missed_by_name() {
    assert_eq!(
        fault("- response: { content: [] }"),
        in_entry(1, "missing field `finish`")
    );
    assert_eq!(
        fault("- response: { finish: end_turn }"),
        in_entry(1, "missing field `content`")
    );
}

#[test]
fn a_key_an_error_needs_is_missed_by_name() {
    assert_eq!(
        fault("- error: { message: overloaded }"),
        in_entry(1, "missing field `kind`")
    );
}

#[test]
fn a_key_a_block_needs_is_missed_by_name() {
    for (block, reason) in [
        (
            "tool_use: { name: bash, input: { json: {} } }",
            "missing field `id`",
        ),
        (
            "tool_use: { id: a, input: { json: {} } }",
            "missing field `name`",
        ),
        ("tool_use: { id: a, name: bash }", "missing field `input`"),
        ("thinking: { signature: abc }", "missing field `text`"),
        ("redacted_thinking: {}", "missing field `data`"),
        ("opaque: { payload: {} }", "missing field `provider`"),
    ] {
        assert_eq!(
            fault(&format!(
                "- response: {{ content: [{{ {block} }}], finish: end_turn }}"
            )),
            in_block(1, 1, reason),
            "{block}"
        );
    }
}

#[test]
fn an_entry_that_is_neither_a_response_nor_an_error_is_refused() {
    assert_eq!(
        fault("- reponse: { content: [], finish: end_turn }"),
        in_entry(
            1,
            "unknown variant `reponse`, expected `response` or `error`"
        )
    );
    assert_eq!(
        fault("- 3"),
        in_entry(1, "invalid type: integer `3`, expected string or map")
    );
}

#[test]
fn an_entry_that_is_more_than_one_thing_is_refused() {
    let one_key = "invalid value: map, expected map with a single key";

    assert_eq!(
        fault(
            r"
- response: { content: [], finish: end_turn }
  error: { kind: fatal }
"
        ),
        in_entry(1, one_key)
    );
    assert_eq!(
        fault(
            r"
- response: { content: [], finish: end_turn }
  latency: 250ms
"
        ),
        in_entry(1, one_key),
        "a latency belongs to the response, inside it"
    );
}

#[test]
fn a_block_of_a_kind_no_response_holds_is_refused() {
    assert_eq!(
        fault(
            "- response: { content: [{ tool_result: { call_id: a, content: [] } }], finish: end_turn }"
        ),
        in_block(
            1,
            1,
            "unknown variant `tool_result`, expected one of `text`, `thinking`, \
             `redacted_thinking`, `tool_use`, `opaque`"
        )
    );
    assert_eq!(
        fault("- response: { content: [{ text: a, signature: b }], finish: end_turn }"),
        in_block(1, 1, "invalid value: map, expected map with a single key")
    );
}

#[test]
fn a_tool_input_is_json_or_unparsed_and_nothing_else() {
    assert_eq!(
        fault(
            "- response: { content: [{ tool_use: { id: a, name: bash, input: { command: ls } } }], finish: tool_use }"
        ),
        in_block(
            1,
            1,
            "unknown variant `command`, expected `json` or `unparsed`"
        )
    );
}

#[test]
fn a_provider_no_block_could_come_from_is_refused() {
    assert_eq!(
        fault(
            "- response: { content: [{ opaque: { provider: google, payload: {} } }], finish: end_turn }"
        ),
        in_block(
            1,
            1,
            "unknown variant `google`, expected one of `anthropic`, `openai`, `fake`"
        )
    );
}

#[test]
fn a_kind_of_error_no_provider_has_is_refused_with_the_kinds_there_are() {
    assert_eq!(
        fault("- error: { kind: retriable }"),
        in_entry(
            1,
            "unknown variant `retriable`, expected one of `retryable`, `context_exhausted`, \
             `auth`, `fatal`, `malformed`"
        )
    );
}

#[test]
fn a_tool_name_the_domain_refuses_is_refused_when_the_script_is_read() {
    let long = "n".repeat(ToolName::MAX_LEN + 1);
    for (name, reason) in [
        ("''", "tool name is empty".to_owned()),
        (
            "' bash'",
            r#"tool name " bash" has leading or trailing whitespace"#.to_owned(),
        ),
        (
            "read.file",
            r#"tool name "read.file" has a character other than an ASCII letter, a digit, `_`, or `-`"#
                .to_owned(),
        ),
        (
            long.as_str(),
            format!(r#"tool name "{long}" is longer than 64 characters"#),
        ),
    ] {
        assert_eq!(
            fault(&format!(
                "- response: {{ content: [{{ text: fine }}, {{ tool_use: {{ id: a, name: {name}, input: {{ json: {{}} }} }} }}], finish: tool_use }}"
            )),
            in_block(1, 2, &reason),
            "{name}"
        );
    }
}

#[test]
fn a_tool_call_id_the_domain_refuses_is_refused_when_the_script_is_read() {
    for (id, reason) in [
        ("''", "tool call id is empty"),
        (
            "'call_1 '",
            r#"tool call id "call_1 " has leading or trailing whitespace"#,
        ),
    ] {
        assert_eq!(
            fault(&format!(
                "- response: {{ content: [{{ tool_use: {{ id: {id}, name: bash, input: {{ json: {{}} }} }} }}], finish: tool_use }}"
            )),
            in_block(1, 1, reason),
            "{id}"
        );
    }
}

#[test]
fn a_tool_call_id_on_two_blocks_of_one_response_is_refused_when_the_script_is_read() {
    assert_eq!(
        fault(
            r"
- response: { content: [], finish: end_turn }
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: {} } }
      - text: and again
      - tool_use: { id: call_1, name: read_file, input: { json: {} } }
    finish: tool_use
"
        ),
        in_entry(
            2,
            r#"tool call id "call_1" is on more than one tool-use block of the response"#
        )
    );
}

#[test]
fn a_value_of_the_wrong_kind_is_refused_and_never_turned_into_the_right_one() {
    assert_eq!(
        fault("- response: { content: [{ text: 42 }], finish: end_turn }"),
        in_block(1, 1, "invalid type: integer `42`, expected a string")
    );
    assert_eq!(
        fault(
            "- response: { content: [{ tool_use: { id: 1, name: bash, input: { json: {} } } }], finish: tool_use }"
        ),
        in_block(1, 1, "invalid type: integer `1`, expected a string")
    );
    assert_eq!(
        fault("- response: { content: [], finish: null }"),
        in_entry(1, "invalid type: null, expected a string")
    );
    assert_eq!(
        fault("- response: { content: { text: hello }, finish: end_turn }"),
        in_entry(1, "invalid type: map, expected a sequence")
    );
    assert_eq!(
        fault("- response: { content: [], finish: end_turn, usage: { input_tokens: -3 } }"),
        in_entry(1, "invalid value: integer `-3`, expected u64")
    );
    assert_eq!(
        fault("- response: { content: [], finish: end_turn, usage: null }"),
        in_entry(1, "invalid type: null, expected struct Usage")
    );
}

#[test]
fn a_duration_that_is_a_number_or_has_no_unit_is_refused() {
    assert_eq!(
        fault("- response: { content: [], finish: end_turn, latency: 250 }"),
        in_entry(
            1,
            "invalid type: integer `250`, expected a duration as text, such as `250ms` or `2s`"
        )
    );
    assert_eq!(
        fault("- error: { kind: retryable, retry_after: '2' }"),
        in_entry(
            1,
            r#""2" isn't a duration such as `250ms` or `2s`: time unit needed, for example 2sec or 2ms"#
        )
    );
    assert_eq!(
        fault("- error: { kind: retryable, latency: soon }"),
        in_entry(
            1,
            r#""soon" isn't a duration such as `250ms` or `2s`: expected number at 0"#
        )
    );
}

#[test]
fn text_that_holds_no_list_is_refused_with_what_it_holds() {
    for (text, found) in [
        ("", "nothing"),
        ("# a script to be written\n", "nothing"),
        ("true", "a boolean"),
        ("3", "a number"),
        ("response", "a string"),
        ("response: { content: [], finish: end_turn }", "a mapping"),
    ] {
        assert_eq!(fault(text), ScriptFault::NotAList { found }, "{text}");
    }
    assert_eq!(
        yaml("responses: []").unwrap_err().to_string(),
        r#"script "scripts/smoke.yaml": it holds a mapping where a list of entries belongs"#
    );
}

#[test]
fn a_list_of_no_entries_is_refused() {
    assert_eq!(fault("[]"), ScriptFault::Empty);
    assert_eq!(
        json("[]").unwrap_err().to_string(),
        r#"script "scripts/smoke.yaml": it holds no entry, so it could answer no provider call"#
    );
}

#[test]
fn text_that_is_not_yaml_is_refused_with_the_line_it_fails_at() {
    assert_eq!(
        fault("- response: { content: [\n- error: { kind: fatal }\n"),
        ScriptFault::Syntax {
            format: ScriptFormat::Yaml,
            reason: r#""-" is only valid inside a block at line 2, column 1"#.to_owned(),
        }
    );
}

#[test]
fn text_that_is_not_json_is_refused_with_the_line_it_fails_at() {
    let refused = json("[\n  {\"error\": {\"kind\": \"fatal\"}}\n").unwrap_err();

    assert_eq!(
        refused.fault,
        ScriptFault::Syntax {
            format: ScriptFormat::Json,
            reason: "EOF while parsing a list at line 3 column 0".to_owned(),
        }
    );
    assert_eq!(
        refused.to_string(),
        r#"script "scripts/smoke.yaml": it can't be read as JSON: EOF while parsing a list at line 3 column 0"#
    );
}

#[test]
fn yaml_is_not_read_as_json_nor_json_as_anything_looser() {
    assert!(matches!(
        json("- error: { kind: fatal }").unwrap_err().fault,
        ScriptFault::Syntax {
            format: ScriptFormat::Json,
            ..
        }
    ));
    assert!(matches!(
        json("[{'error': {'kind': 'fatal'}}]").unwrap_err().fault,
        ScriptFault::Syntax {
            format: ScriptFormat::Json,
            ..
        }
    ));
}

#[test]
fn a_key_written_twice_is_refused_in_either_format() {
    let ScriptFault::Syntax { format, reason } =
        fault("- response:\n    content: []\n    finish: end_turn\n    finish: refusal\n")
    else {
        panic!("a repeated key is refused as the text is parsed");
    };
    assert_eq!(format, ScriptFormat::Yaml);
    assert!(reason.contains("duplicate mapping key: finish"), "{reason}");
    assert!(reason.contains("line 4"), "{reason}");

    assert_eq!(
        json(r#"[{"response": {"content": [], "finish": "end_turn", "finish": "refusal"}}]"#)
            .unwrap_err()
            .fault,
        ScriptFault::Syntax {
            format: ScriptFormat::Json,
            reason: r#"the key "finish" is written twice in one mapping at line 1 column 60"#
                .to_owned(),
        }
    );
}

#[test]
fn a_second_yaml_document_is_refused() {
    let ScriptFault::Syntax { reason, .. } =
        fault("- error: { kind: fatal }\n---\n- error: { kind: auth }\n")
    else {
        panic!("a second document is refused as the text is parsed");
    };

    assert!(reason.contains("multiple YAML documents"), "{reason}");
}

#[test]
fn a_format_prints_its_name() {
    assert_eq!(ScriptFormat::Yaml.to_string(), "YAML");
    assert_eq!(ScriptFormat::Json.to_string(), "JSON");
}
