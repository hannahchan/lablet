use serde_json::json;

use super::*;
use crate::{ToolName, ToolResultContent};

fn id(value: &str) -> ToolCallId {
    ToolCallId::new(value).unwrap()
}

fn text(text: &str) -> ContentBlock {
    ContentBlock::Text(text.to_owned())
}

fn tool_use(call_id: &str, tool: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id(call_id),
        name: ToolName::new(tool).unwrap(),
        input: json!({}),
    }
}

fn tool_result(call_id: &str, output: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        call_id: id(call_id),
        content: vec![ToolResultContent::Text(output.to_owned())],
        is_error: false,
    }
}

fn record(input: u64, output: u64, cache_read: u64) -> TurnRecord {
    TurnRecord {
        usage: Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_write_tokens: 0,
        },
        response_model: Some("model-2026".to_owned()),
    }
}

/// A prompt, a turn with two tool calls whose results come back in the
/// opposite order, and a final turn.
fn two_calls_in_one_turn() -> Transcript {
    let mut transcript = Transcript::new("You fix tests.".to_owned());
    transcript.push_user(vec![text("Fix the failing test.")]);
    transcript.push_assistant(
        vec![
            text("Looking."),
            tool_use("call_a", "read_file"),
            tool_use("call_b", "bash"),
        ],
        record(100, 20, 0),
    );
    transcript.push_user(vec![
        tool_result("call_b", "1 failed"),
        tool_result("call_a", "fn main() {}"),
    ]);
    transcript.push_assistant(vec![text("Fixed.")], record(180, 5, 100));
    transcript
}

#[test]
fn a_new_transcript_holds_the_system_prompt_and_nothing_else() {
    let transcript = Transcript::new("You fix tests.".to_owned());

    assert_eq!(transcript.system, "You fix tests.");
    assert!(transcript.messages.is_empty());
    assert!(transcript.turns.is_empty());
    assert!(transcript.steps().is_empty());
}

#[test]
fn pushing_sets_the_role_and_keeps_one_record_for_each_assistant_message() {
    let transcript = two_calls_in_one_turn();

    let roles: Vec<Role> = transcript
        .messages
        .iter()
        .map(|message| message.role)
        .collect();
    assert_eq!(
        roles,
        [Role::User, Role::Assistant, Role::User, Role::Assistant]
    );
    assert_eq!(transcript.turns, [record(100, 20, 0), record(180, 5, 100)]);
    assert!(transcript.messages.iter().all(|m| m.validate().is_ok()));
}

#[test]
fn each_result_of_a_turn_with_two_tool_calls_pairs_with_its_call_by_id() {
    let transcript = two_calls_in_one_turn();
    let steps = transcript.steps();
    let step = steps[0];

    let pairs: Vec<(&str, &str, Option<&ContentBlock>)> = step
        .message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse { id, name, .. } => {
                Some((id.as_str(), name.as_str(), step.result_of(id)))
            }
            _ => None,
        })
        .collect();

    assert_eq!(
        pairs,
        [
            (
                "call_a",
                "read_file",
                Some(&tool_result("call_a", "fn main() {}"))
            ),
            ("call_b", "bash", Some(&tool_result("call_b", "1 failed"))),
        ]
    );
}

#[test]
fn one_assistant_turn_is_one_step_with_its_own_metrics_and_model() {
    let transcript = two_calls_in_one_turn();
    let steps = transcript.steps();

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].message, &transcript.messages[1]);
    assert_eq!(steps[0].record, Some(&record(100, 20, 0)));
    assert_eq!(steps[0].observation, Some(&transcript.messages[2]));
    assert_eq!(steps[1].message, &transcript.messages[3]);
    assert_eq!(steps[1].record, Some(&record(180, 5, 100)));
    assert_eq!(steps[1].observation, None);

    // What a trajectory export reads for the second step's metrics.
    let usage = steps[1].record.unwrap().usage;
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens
        ),
        (180, 5, 100)
    );
    assert_eq!(
        steps[1].record.unwrap().response_model.as_deref(),
        Some("model-2026")
    );
}

#[test]
fn a_call_the_run_never_executed_has_no_result() {
    let mut transcript = Transcript::new(String::new());
    transcript.push_user(vec![text("Do it.")]);
    transcript.push_assistant(
        vec![tool_use("call_done", "task_complete")],
        record(10, 2, 0),
    );

    let steps = transcript.steps();
    assert_eq!(steps[0].observation, None);
    assert_eq!(steps[0].result_of(&id("call_done")), None);
}

#[test]
fn a_user_message_without_tool_results_is_not_an_observation() {
    let mut transcript = Transcript::new(String::new());
    transcript.push_assistant(vec![text("Which file?")], record(10, 2, 0));
    transcript.push_user(vec![text("src/lib.rs")]);

    assert_eq!(transcript.steps()[0].observation, None);
}

#[test]
fn results_are_looked_up_in_the_turns_own_observation_even_when_ids_repeat_across_turns() {
    let mut transcript = Transcript::new(String::new());
    transcript.push_user(vec![text("Go.")]);
    transcript.push_assistant(vec![tool_use("call_1", "bash")], record(10, 2, 0));
    transcript.push_user(vec![tool_result("call_1", "first")]);
    transcript.push_assistant(vec![tool_use("call_1", "bash")], record(20, 2, 0));
    transcript.push_user(vec![tool_result("call_1", "second")]);

    let steps = transcript.steps();
    assert_eq!(
        steps[0].result_of(&id("call_1")),
        Some(&tool_result("call_1", "first"))
    );
    assert_eq!(
        steps[1].result_of(&id("call_1")),
        Some(&tool_result("call_1", "second"))
    );
}

#[test]
fn a_deserialised_transcript_short_of_records_still_yields_every_step() {
    let mut transcript = two_calls_in_one_turn();
    transcript.turns.truncate(1);

    let steps = transcript.steps();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].record, Some(&record(100, 20, 0)));
    assert_eq!(steps[1].record, None);
}

#[test]
fn a_transcript_has_one_json_form() {
    let mut transcript = Transcript::new("Be brief.".to_owned());
    transcript.push_user(vec![text("Hi.")]);
    transcript.push_assistant(vec![text("Hello.")], record(12, 3, 8));
    let expected = json!({
        "system": "Be brief.",
        "messages": [
            { "role": "user", "content": [{ "text": "Hi." }] },
            { "role": "assistant", "content": [{ "text": "Hello." }] },
        ],
        "turns": [{
            "usage": {
                "input_tokens": 12,
                "output_tokens": 3,
                "cache_read_tokens": 8,
                "cache_write_tokens": 0,
            },
            "response_model": "model-2026",
        }],
    });

    assert_eq!(serde_json::to_value(&transcript).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<Transcript>(expected).unwrap(),
        transcript
    );
}
