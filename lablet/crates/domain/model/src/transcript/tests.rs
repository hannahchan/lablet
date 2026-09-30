use serde_json::json;

use super::*;
use crate::{
    Answer, KeptOutput, ProviderKind, TokenCounts, ToolCallEnd, ToolCallId, ToolCallStatus,
    ToolInput, ToolName, ToolResultContent, ToolSource,
};

/// A call to a tool the run offered from `source`, which ended `ended`.
fn ran(source: ToolSource, ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(source, ended)
}

const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn id(value: &str) -> ToolCallId {
    ToolCallId::new(value).unwrap()
}

fn text(text: &str) -> ContentBlock {
    ContentBlock::Text(text.to_owned())
}

fn call(call_id: &str, tool: &str) -> ToolUse {
    ToolUse {
        id: id(call_id),
        name: ToolName::new(tool).unwrap(),
        input: ToolInput::Json(json!({})),
    }
}

fn tool_use(call_id: &str, tool: &str) -> ContentBlock {
    ContentBlock::ToolUse(call(call_id, tool))
}

/// A response from a provider that reports its input and output and no
/// other count.
fn response(content: Vec<ContentBlock>, input: u64, output: u64) -> ProviderResponse {
    ProviderResponse::new(
        content,
        Usage::from_inclusive(TokenCounts {
            input,
            output,
            reasoning: None,
            cache_read: None,
            cache_write: None,
        }),
        FinishReason::EndTurn,
        Some("msg_1".to_owned()),
        Some("model-2026".to_owned()),
    )
    .unwrap()
}

fn says(words: &str) -> ProviderResponse {
    response(vec![text(words)], 1, 1)
}

fn calls(ids: &[&str]) -> ProviderResponse {
    response(ids.iter().map(|id| tool_use(id, "bash")).collect(), 1, 1)
}

fn outcome(call_id: &str, status: ToolCallStatus, output: &str) -> ToolCallOutcome {
    Answer::measured(status, KeptOutput::whole(output), None, ms(900), ms(30))
        .answering(id(call_id))
}

fn ok(call_id: &str, output: &str) -> ToolCallOutcome {
    outcome(call_id, ran(ToolSource::Builtin, ToolCallEnd::Ok), output)
}

fn transcript() -> Transcript {
    Transcript::new("You fix tests.".to_owned())
}

fn said(words: &str) -> Vec<UserContent> {
    vec![UserContent::Text(words.to_owned())]
}

fn prompt() -> Vec<UserContent> {
    said("Fix the failing test.")
}

/// Records `completion` as the next turn, with `input` as its input, the way
/// a run does, and returns it.
fn turn(
    transcript: &mut Transcript,
    input: Vec<UserContent>,
    completion: ProviderResponse,
) -> Turn {
    let turn = Turn::recorded(input, completion, ms(0), ms(0), 1);
    transcript.push(turn.clone());
    turn
}

/// Makes `outcomes` those of the last turn, the way a run's answer does.
fn answer(transcript: &mut Transcript, outcomes: Vec<ToolCallOutcome>) {
    transcript.turns.last_mut().unwrap().answered(outcomes);
}

/// A turn with two tool calls and their outcomes, and a final turn.
fn two_calls_in_one_turn() -> Transcript {
    let mut transcript = transcript();
    let looking = vec![
        text("Looking."),
        tool_use("call_a", "read_file"),
        tool_use("call_b", "bash"),
    ];
    turn(&mut transcript, prompt(), response(looking, 100, 20));
    answer(
        &mut transcript,
        vec![
            ok("call_a", "fn main() {}"),
            outcome(
                "call_b",
                ran(ToolSource::Builtin, ToolCallEnd::ToolError),
                "1 failed",
            ),
        ],
    );
    turn(
        &mut transcript,
        Vec::new(),
        response(vec![text("Fixed.")], 180, 5),
    );
    transcript
}

#[test]
fn a_new_transcript_holds_the_system_prompt_and_no_turns() {
    let transcript = transcript();

    assert_eq!(transcript.system(), "You fix tests.");
    assert!(transcript.turns().is_empty());
    assert_eq!(transcript.final_text(), "");
    assert_eq!(transcript.messages(&[]), []);
}

#[test]
fn the_first_request_is_the_input_that_waits_for_the_first_turn() {
    let prompt = prompt();

    assert_eq!(
        transcript().messages(&prompt),
        [Message::User {
            tool_results: Vec::new(),
            input: &prompt,
        }]
    );
}

#[test]
fn a_completion_becomes_a_turn_that_takes_the_input_and_records_the_rest_with_the_timing() {
    let mut transcript = transcript();
    let turn = Turn::recorded(
        prompt(),
        response(vec![text("Hello.")], 12, 3),
        Duration::from_micros(1_500_999),
        Duration::from_micros(42_999),
        3,
    );
    transcript.push(turn.clone());

    assert_eq!(turn.input(), prompt());
    assert_eq!(turn.response(), [text("Hello.")]);
    assert_eq!(
        turn.record(),
        &TurnRecord {
            usage: Usage::from_inclusive(TokenCounts {
                input: 12,
                output: 3,
                reasoning: None,
                cache_read: None,
                cache_write: None
            }),
            finish: FinishReason::EndTurn,
            response_id: Some("msg_1".to_owned()),
            response_model: Some("model-2026".to_owned()),
            started_ms: 1_500,
            latency_ms: 42,
            attempts: 3,
        }
    );
    assert!(turn.tool_calls().is_empty());
    assert_eq!(transcript.turns(), [turn]);
}

#[test]
fn text_blocks_that_are_empty_or_only_whitespace_are_dropped_and_nothing_else_is() {
    let thinking = ContentBlock::Thinking {
        text: String::new(),
        signature: Some("sig".to_owned()),
    };
    let sent = vec![
        text(""),
        thinking.clone(),
        text(" \n\t"),
        text("Done. "),
        tool_use("call_a", "bash"),
        text(""),
    ];
    let kept = vec![thinking, text("Done. "), tool_use("call_a", "bash")];
    let mut transcript = transcript();
    let mut without_them = transcript.clone();

    turn(&mut transcript, prompt(), response(sent, 1, 1));
    turn(&mut without_them, prompt(), response(kept.clone(), 1, 1));

    // Equal transcripts give the same final text and the same size for every
    // message, so nothing measured after the drop can tell the two apart.
    assert_eq!(transcript, without_them);
    assert_eq!(transcript.turns()[0].response(), kept);
    assert_eq!(transcript.final_text(), "Done. ");
    assert_eq!(
        serde_json::to_string(&transcript.messages(&[])).unwrap(),
        serde_json::to_string(&without_them.messages(&[])).unwrap()
    );
}

#[test]
fn each_response_follows_one_user_message_of_the_results_before_it_and_then_its_input() {
    let mut transcript = two_calls_in_one_turn();
    turn(&mut transcript, said("And the docs?"), calls(&["call_c"]));
    answer(&mut transcript, vec![ok("call_c", "updated")]);
    let (call_a, call_b, call_c) = (id("call_a"), id("call_b"), id("call_c"));
    let first = [ToolResultContent::Text("fn main() {}".to_owned())];
    let second = [ToolResultContent::Text("1 failed".to_owned())];
    let third = [ToolResultContent::Text("updated".to_owned())];
    let (prompt, docs, thanks) = (prompt(), said("And the docs?"), said("Thanks."));
    let result = |call_id, content, is_error| ToolResult {
        call_id,
        content,
        is_error,
    };

    assert_eq!(
        transcript.messages(&thanks),
        [
            Message::User {
                tool_results: Vec::new(),
                input: &prompt,
            },
            Message::Assistant(transcript.turns()[0].response()),
            Message::User {
                tool_results: vec![
                    result(&call_a, &first, false),
                    result(&call_b, &second, true)
                ],
                input: &[],
            },
            Message::Assistant(&[text("Fixed.")]),
            Message::User {
                tool_results: Vec::new(),
                input: &docs,
            },
            Message::Assistant(&[tool_use("call_c", "bash")]),
            Message::User {
                tool_results: vec![result(&call_c, &third, false)],
                input: &thanks,
            },
        ]
    );
}

#[test]
fn the_request_ends_with_the_results_the_next_turn_answers_or_with_the_last_response() {
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), calls(&["call_a"]));
    assert_eq!(transcript.messages(&[]).len(), 2);

    answer(&mut transcript, vec![ok("call_a", "out")]);
    let messages = transcript.messages(&[]);
    assert_eq!(messages.len(), 3);
    assert!(matches!(
        &messages[2],
        Message::User { tool_results, input } if tool_results.len() == 1 && input.is_empty()
    ));
}

#[test]
fn a_response_is_rendered_block_for_block_in_the_order_it_arrived() {
    let blocks = vec![
        ContentBlock::Thinking {
            text: "hm".to_owned(),
            signature: Some("sig".to_owned()),
        },
        ContentBlock::RedactedThinking {
            data: "encrypted".to_owned(),
        },
        text("On it."),
        ContentBlock::Opaque {
            provider: ProviderKind::Anthropic,
            payload: json!({ "type": "server_tool_use" }),
        },
        tool_use("call_a", "bash"),
    ];
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), response(blocks.clone(), 1, 1));

    assert_eq!(transcript.messages(&[])[1], Message::Assistant(&blocks));
}

#[test]
fn input_that_is_only_whitespace_is_dropped_from_the_turn_it_answers() {
    for blank in ["", "   ", "\n\t"] {
        let mut input = said(blank);
        input.extend(prompt());

        let turn = turn(&mut transcript(), input, says("Hello."));

        assert_eq!(turn.input(), prompt(), "{blank:?}");
    }
}

#[test]
fn a_turn_after_answered_tool_calls_needs_no_input_and_may_have_some() {
    for input in [Vec::new(), said("Also the docs.")] {
        let mut transcript = transcript();
        turn(&mut transcript, prompt(), calls(&["call_a"]));
        answer(&mut transcript, vec![ok("call_a", "out")]);

        let next = turn(&mut transcript, input.clone(), says("Done."));

        assert_eq!(next.input(), input);
    }
}

#[test]
fn outcomes_that_answer_the_calls_each_once_and_in_order_become_the_turns_tool_calls() {
    let transcript = two_calls_in_one_turn();
    let turn = &transcript.turns()[0];

    let pairs: Vec<(&str, &str)> = turn
        .tool_uses()
        .zip(turn.tool_calls())
        .map(|(call, outcome)| (call.name.as_str(), outcome.call_id.as_str()))
        .collect();

    assert_eq!(pairs, [("read_file", "call_a"), ("bash", "call_b")]);
    assert_eq!(
        turn.tool_calls()[1].status,
        ran(ToolSource::Builtin, ToolCallEnd::ToolError)
    );
}

#[test]
fn a_turn_whose_tools_never_ran_is_told_from_one_that_called_none_by_its_response() {
    let mut transcript = two_calls_in_one_turn();
    let called_none = transcript.turns()[1].clone();
    let done = response(vec![tool_use("call_done", "task_complete")], 10, 2);
    let never_ran = turn(&mut transcript, said("Finish."), done);

    assert!(called_none.tool_calls().is_empty());
    assert_eq!(called_none.tool_uses().count(), 0);
    assert!(never_ran.tool_calls().is_empty());
    assert_eq!(
        never_ran.tool_uses().collect::<Vec<_>>(),
        [&call("call_done", "task_complete")]
    );
}

#[test]
fn final_text_is_the_text_of_the_last_turn_without_its_other_blocks() {
    let mut transcript = two_calls_in_one_turn();
    assert_eq!(transcript.final_text(), "Fixed.");

    let blocks = vec![
        ContentBlock::Thinking {
            text: "not the answer".to_owned(),
            signature: None,
        },
        text("Also "),
        ContentBlock::RedactedThinking {
            data: "encrypted".to_owned(),
        },
        tool_use("call_c", "bash"),
        ContentBlock::Opaque {
            provider: ProviderKind::Openai,
            payload: json!({ "type": "refusal" }),
        },
        text("tidied."),
    ];
    let tidied = turn(&mut transcript, said("And?"), response(blocks, 200, 9));

    assert_eq!(transcript.final_text(), "Also tidied.");
    assert_eq!(tidied.text(), "Also tidied.");
}

#[test]
fn final_text_is_empty_when_the_last_turn_has_no_text() {
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), says("Looking."));
    turn(&mut transcript, said("Go on."), calls(&["call_a"]));

    assert_eq!(transcript.final_text(), "");
}

#[test]
fn usage_is_summed_over_every_turn() {
    assert_eq!(transcript().usage(), Usage::default());
    assert_eq!(
        two_calls_in_one_turn().usage(),
        Usage::from_inclusive(TokenCounts {
            input: 280,
            output: 25,
            reasoning: None,
            cache_read: None,
            cache_write: None
        })
    );
}

/// Every field of both, so a field that `into_parts` dropped or took from
/// the wrong place fails here, before anything that writes a transcript
/// down reads it.
#[test]
fn a_transcript_and_its_turns_come_apart_into_every_part_they_hold() {
    let transcript = two_calls_in_one_turn();
    let turns = transcript.turns().to_vec();

    let parts = transcript.into_parts();

    assert_eq!(
        parts,
        TranscriptParts {
            system: "You fix tests.".to_owned(),
            turns: turns.clone(),
        }
    );
    let [first, second] = <[Turn; 2]>::try_from(turns).unwrap();
    assert_eq!(
        first.clone().into_parts(),
        TurnParts {
            input: prompt(),
            response: vec![
                text("Looking."),
                tool_use("call_a", "read_file"),
                tool_use("call_b", "bash"),
            ],
            record: first.record().clone(),
            tool_calls: vec![
                ok("call_a", "fn main() {}"),
                outcome(
                    "call_b",
                    ran(ToolSource::Builtin, ToolCallEnd::ToolError),
                    "1 failed"
                ),
            ],
        }
    );
    assert_eq!(first.record().usage.input_tokens, 100);
    assert_eq!(
        second.clone().into_parts(),
        TurnParts {
            input: Vec::new(),
            response: vec![text("Fixed.")],
            record: second.record().clone(),
            tool_calls: Vec::new(),
        }
    );
    assert_eq!(second.record().usage.input_tokens, 180);
}
