use serde_json::{Value, json};

use super::document::{TRANSCRIPT_SCHEMA_VERSION, TranscriptDocument};
use super::*;
use crate::{
    CompletionMode, ProviderKind, TokenCounts, ToolCallEnd, ToolCallStatus, ToolInput, ToolName,
    ToolResultContent, ToolSource,
};

/// A call to a tool the run offered, which ended `ended`.
fn ran(ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(ToolSource::Builtin, ended)
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

fn response(content: Vec<ContentBlock>, input: u64, output: u64) -> ProviderResponse {
    ProviderResponse::new(
        content,
        Usage::from_inclusive(TokenCounts {
            input,
            output,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
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
    ToolCallOutcome::measured(
        id(call_id),
        status,
        vec![ToolResultContent::Text(output.to_owned())],
        None,
        ms(900),
        ms(30),
    )
}

fn ok(call_id: &str, output: &str) -> ToolCallOutcome {
    outcome(call_id, ran(ToolCallEnd::Ok), output)
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

/// Records `completion` as the next turn, with `input` as its input.
fn turn(
    transcript: &mut Transcript,
    mut input: Vec<UserContent>,
    completion: ProviderResponse,
) -> Result<Turn, TranscriptError> {
    transcript
        .record(&mut input, completion, ms(0), ms(0), 1)
        .cloned()
}

/// A turn whose response makes the calls `tools`, in that order.
fn turn_calling(tools: &[&str]) -> Turn {
    let blocks = tools
        .iter()
        .enumerate()
        .map(|(n, tool)| tool_use(&format!("call_{n}"), tool))
        .collect();
    turn(&mut transcript(), prompt(), response(blocks, 1, 1)).unwrap()
}

#[test]
fn a_response_that_calls_no_tool_has_no_calls_in_either_mode() {
    let turn = turn_calling(&[]);

    assert_eq!(turn.calls(CompletionMode::Natural), Calls::None);
    assert_eq!(turn.calls(CompletionMode::Explicit), Calls::None);
}

#[test]
fn a_response_that_calls_ordinary_tools_has_tool_calls_in_either_mode() {
    let turn = turn_calling(&["bash", "read_file"]);

    assert_eq!(turn.calls(CompletionMode::Natural), Calls::Tools);
    assert_eq!(turn.calls(CompletionMode::Explicit), Calls::Tools);
}

#[test]
fn only_explicit_mode_reads_task_complete_as_the_end_of_the_task() {
    for response in [
        vec![CompletionMode::TASK_COMPLETE],
        vec!["bash", CompletionMode::TASK_COMPLETE],
        vec![CompletionMode::TASK_COMPLETE, "bash"],
    ] {
        let turn = turn_calling(&response);

        assert_eq!(
            turn.calls(CompletionMode::Explicit),
            Calls::TaskComplete,
            "{response:?}"
        );
        assert_eq!(
            turn.calls(CompletionMode::Natural),
            Calls::Tools,
            "{response:?}"
        );
    }
}

/// A turn with two tool calls and their outcomes, and a final turn.
fn two_calls_in_one_turn() -> Transcript {
    let mut transcript = transcript();
    let looking = vec![
        text("Looking."),
        tool_use("call_a", "read_file"),
        tool_use("call_b", "bash"),
    ];
    turn(&mut transcript, prompt(), response(looking, 100, 20)).unwrap();
    transcript
        .answer(vec![
            ok("call_a", "fn main() {}"),
            outcome("call_b", ran(ToolCallEnd::ToolError), "1 failed"),
        ])
        .unwrap();
    turn(
        &mut transcript,
        Vec::new(),
        response(vec![text("Fixed.")], 180, 5),
    )
    .unwrap();
    transcript
}

fn strings(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|&id| id.to_owned()).collect()
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
    let mut input = prompt();

    let turn = transcript
        .record(
            &mut input,
            response(vec![text("Hello.")], 12, 3),
            Duration::from_micros(1_500_999),
            Duration::from_micros(42_999),
            3,
        )
        .unwrap()
        .clone();

    assert!(input.is_empty());
    assert_eq!(turn.input(), prompt());
    assert_eq!(turn.response(), [text("Hello.")]);
    assert_eq!(
        turn.record(),
        &TurnRecord {
            usage: Usage::from_inclusive(TokenCounts {
                input: 12,
                output: 3,
                reasoning: 0,
                cache_read: 0,
                cache_write: 0
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

    turn(&mut transcript, prompt(), response(sent, 1, 1)).unwrap();
    turn(&mut without_them, prompt(), response(kept.clone(), 1, 1)).unwrap();

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
    turn(&mut transcript, said("And the docs?"), calls(&["call_c"])).unwrap();
    transcript.answer(vec![ok("call_c", "updated")]).unwrap();
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
    turn(&mut transcript, prompt(), calls(&["call_a"])).unwrap();
    assert_eq!(transcript.messages(&[]).len(), 2);

    transcript.answer(vec![ok("call_a", "out")]).unwrap();
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
    turn(&mut transcript, prompt(), response(blocks.clone(), 1, 1)).unwrap();

    assert_eq!(transcript.messages(&[])[1], Message::Assistant(&blocks));
}

#[test]
fn the_first_turn_is_refused_without_input() {
    let mut transcript = transcript();

    assert_eq!(
        turn(&mut transcript, Vec::new(), says("Hello.")),
        Err(TranscriptError::NothingFromTheUser { turn: 1 })
    );
    assert!(transcript.turns().is_empty());
}

#[test]
fn input_that_is_only_whitespace_is_nothing_from_the_user() {
    let mut transcript = transcript();

    for blank in ["", "   ", "\n\t"] {
        assert_eq!(
            turn(&mut transcript, said(blank), says("Hello.")),
            Err(TranscriptError::NothingFromTheUser { turn: 1 }),
            "{blank:?}"
        );
    }
    turn(&mut transcript, prompt(), says("Hello.")).unwrap();
    assert_eq!(transcript.turns()[0].input(), prompt());
}

/// The caller keeps its input to offer to the next turn, so a refusal can't
/// quietly take part of it. `turn` copies the input in, which would hide this.
#[test]
fn a_refused_turn_leaves_the_input_as_the_caller_had_it() {
    let mut transcript = transcript();
    let mut blank = said("   ");
    let mut waiting = prompt();

    assert_eq!(
        transcript
            .record(&mut blank, says("Hello."), ms(0), ms(0), 1)
            .unwrap_err(),
        TranscriptError::NothingFromTheUser { turn: 1 }
    );
    assert_eq!(blank, said("   "));

    transcript
        .record(&mut waiting, calls(&["call_a"]), ms(0), ms(0), 1)
        .unwrap();
    let mut next = prompt();
    assert_eq!(
        transcript
            .record(&mut next, says("Hello."), ms(0), ms(0), 1)
            .unwrap_err(),
        TranscriptError::UnansweredCalls {
            turn: 1,
            calls: vec!["call_a".to_owned()],
        }
    );
    assert_eq!(next, prompt());
}

#[test]
fn the_tool_calls_of_a_turn_are_answered_once() {
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), calls(&["call_a"])).unwrap();

    assert_eq!(
        transcript.answer(Vec::new()),
        Err(TranscriptError::OutcomesDontAnswerCalls {
            calls: vec!["call_a".to_owned()],
            outcomes: Vec::new(),
        })
    );
    transcript
        .answer(vec![outcome("call_a", ran(ToolCallEnd::Ok), "done")])
        .unwrap();
    assert_eq!(
        transcript.answer(vec![outcome("call_a", ran(ToolCallEnd::Ok), "again")]),
        Err(TranscriptError::AlreadyAnswered { turn: 1 })
    );
    assert_eq!(transcript.turns()[0].tool_calls().len(), 1);
}

#[test]
fn a_turn_that_called_no_tools_takes_no_outcomes() {
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), says("Done.")).unwrap();

    transcript.answer(Vec::new()).unwrap();
    assert!(transcript.turns()[0].tool_calls().is_empty());
}

#[test]
fn a_turn_after_one_that_called_no_tools_needs_input_of_its_own() {
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), says("One.")).unwrap();
    let before = transcript.clone();

    assert_eq!(
        turn(&mut transcript, Vec::new(), says("Two.")),
        Err(TranscriptError::NothingFromTheUser { turn: 2 })
    );
    assert_eq!(transcript, before);

    let second = turn(&mut transcript, said("Go on."), says("Two.")).unwrap();
    assert_eq!(second.input(), said("Go on."));
    assert_eq!(transcript.turns().len(), 2);
}

#[test]
fn a_turn_after_answered_tool_calls_needs_no_input_and_may_have_some() {
    for input in [Vec::new(), said("Also the docs.")] {
        let mut transcript = transcript();
        turn(&mut transcript, prompt(), calls(&["call_a"])).unwrap();
        transcript.answer(vec![ok("call_a", "out")]).unwrap();

        let next = turn(&mut transcript, input.clone(), says("Done.")).unwrap();

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
    assert_eq!(turn.tool_calls()[1].status, ran(ToolCallEnd::ToolError));
}

#[test]
fn outcomes_that_are_not_exactly_the_calls_in_call_order_are_refused() {
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), calls(&["call_a", "call_b"])).unwrap();
    let before = transcript.clone();

    for outcomes in [
        vec!["call_b", "call_a"],
        vec!["call_a"],
        vec!["call_b"],
        vec!["call_a", "call_b", "call_c"],
        vec!["call_a", "call_a"],
        vec!["call_a", "call_x"],
    ] {
        let error = transcript
            .answer(outcomes.iter().map(|id| ok(id, "out")).collect())
            .unwrap_err();

        assert_eq!(
            error,
            TranscriptError::OutcomesDontAnswerCalls {
                calls: strings(&["call_a", "call_b"]),
                outcomes: strings(&outcomes),
            }
        );
        assert_eq!(transcript, before);
    }
}

#[test]
fn outcomes_are_refused_when_no_response_made_a_call() {
    let mut transcript = transcript();
    let refused = TranscriptError::OutcomesDontAnswerCalls {
        calls: Vec::new(),
        outcomes: strings(&["call_a"]),
    };

    assert_eq!(
        transcript.answer(vec![ok("call_a", "out")]),
        Err(refused.clone())
    );
    turn(&mut transcript, prompt(), says("Hello.")).unwrap();
    assert_eq!(transcript.answer(vec![ok("call_a", "out")]), Err(refused));
}

#[test]
fn no_outcomes_leave_the_last_turn_as_it_was() {
    let mut empty = transcript();
    let mut answered = two_calls_in_one_turn();
    let before = answered.clone();

    assert_eq!(answered.answer(Vec::new()), Ok(()));
    assert_eq!(answered, before);
    assert_eq!(empty.answer(Vec::new()), Ok(()));
    assert_eq!(empty, transcript());
}

#[test]
fn only_the_last_turn_may_have_tool_calls_and_no_outcomes_whatever_input_follows() {
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), calls(&["call_a", "call_b"])).unwrap();
    let before = transcript.clone();

    for input in [Vec::new(), said("Never mind the tools.")] {
        assert_eq!(
            turn(&mut transcript, input, says("Next.")),
            Err(TranscriptError::UnansweredCalls {
                turn: 1,
                calls: strings(&["call_a", "call_b"]),
            })
        );
        assert_eq!(transcript, before);
    }

    transcript
        .answer(vec![ok("call_a", "one"), ok("call_b", "two")])
        .unwrap();
    assert!(turn(&mut transcript, Vec::new(), says("Next.")).is_ok());
}

#[test]
fn a_turn_whose_tools_never_ran_is_told_from_one_that_called_none_by_its_response() {
    let mut transcript = two_calls_in_one_turn();
    let called_none = transcript.turns()[1].clone();
    let done = response(vec![tool_use("call_done", "task_complete")], 10, 2);
    let never_ran = turn(&mut transcript, said("Finish."), done).unwrap();

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
    let tidied = turn(&mut transcript, said("And?"), response(blocks, 200, 9)).unwrap();

    assert_eq!(transcript.final_text(), "Also tidied.");
    assert_eq!(tidied.text(), "Also tidied.");
}

#[test]
fn final_text_is_empty_when_the_last_turn_has_no_text() {
    let mut transcript = transcript();
    turn(&mut transcript, prompt(), says("Looking.")).unwrap();
    turn(&mut transcript, said("Go on."), calls(&["call_a"])).unwrap();

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
            reasoning: 0,
            cache_read: 0,
            cache_write: 0
        })
    );
}

#[test]
fn consecutive_tool_errors_count_back_from_the_last_outcome_to_the_last_success() {
    let mut transcript = transcript();
    assert_eq!(transcript.consecutive_tool_errors(), 0);

    let statuses = [
        (ran(ToolCallEnd::Failed), ran(ToolCallEnd::Ok), 0),
        (ran(ToolCallEnd::Ok), ran(ToolCallEnd::Timeout), 1),
        (ToolCallStatus::Unknown, ran(ToolCallEnd::ToolError), 3),
    ];
    let mut input = prompt();
    for (first, second, expected) in statuses {
        turn(
            &mut transcript,
            std::mem::take(&mut input),
            calls(&["call_a", "call_b"]),
        )
        .unwrap();
        transcript
            .answer(vec![
                outcome("call_a", first, "out"),
                outcome("call_b", second, "out"),
            ])
            .unwrap();
        assert_eq!(transcript.consecutive_tool_errors(), expected);
    }

    turn(&mut transcript, Vec::new(), says("No tools.")).unwrap();
    assert_eq!(transcript.consecutive_tool_errors(), 3);
}

/// A two-turn run with one tool call, as the document a grader reads.
fn document() -> Value {
    json!({
        "system": "You fix tests.",
        "turns": [
            {
                "input": [{ "text": "Fix the failing test." }],
                "response": [
                    { "text": "Looking." },
                    { "tool_use": { "id": "call_a", "name": "bash", "input": { "json": { "command": "cargo test" } } } },
                ],
                "record": {
                    "usage": {
                        "input_tokens": 100,
                        "output_tokens": 20,
                        "reasoning_output_tokens": 0,
                        "cache_read_tokens": 0,
                        "cache_write_tokens": 0,
                    },
                    "finish": "tool_use",
                    "response_id": "msg_1",
                    "response_model": "model-2026",
                    "started_ms": 0,
                    "latency_ms": 800,
                    "attempts": 2,
                },
                "tool_calls": [{
                    "call_id": "call_a",
                    "status": { "ran": { "source": "builtin", "ended": "tool_error" } },
                    "started_ms": 900,
                    "latency_ms": 30,
                    "truncated_from_bytes": null,
                    "content": [{ "text": "1 failed" }],
                }],
            },
            {
                "input": [],
                "response": [{ "text": "Fixed." }],
                "record": {
                    "usage": {
                        "input_tokens": 180,
                        "output_tokens": 5,
                        "reasoning_output_tokens": 0,
                        "cache_read_tokens": 100,
                        "cache_write_tokens": 0,
                    },
                    "finish": "end_turn",
                    "response_id": null,
                    "response_model": null,
                    "started_ms": 1000,
                    "latency_ms": 300,
                    "attempts": 1,
                },
                "tool_calls": [],
            },
        ],
    })
}

#[test]
fn a_transcript_has_one_json_form() {
    let mut transcript = transcript();
    let looking = ProviderResponse::new(
        vec![
            text("Looking."),
            ContentBlock::ToolUse(ToolUse {
                input: ToolInput::Json(json!({ "command": "cargo test" })),
                ..call("call_a", "bash")
            }),
        ],
        Usage::from_inclusive(TokenCounts {
            input: 100,
            output: 20,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        }),
        FinishReason::ToolUse,
        Some("msg_1".to_owned()),
        Some("model-2026".to_owned()),
    )
    .unwrap();
    let fixed = ProviderResponse::new(
        vec![text("Fixed.")],
        Usage::from_inclusive(TokenCounts {
            input: 180,
            output: 5,
            reasoning: 0,
            cache_read: 100,
            cache_write: 0,
        }),
        FinishReason::EndTurn,
        None,
        None,
    )
    .unwrap();
    transcript
        .record(&mut prompt(), looking, ms(0), ms(800), 2)
        .unwrap();
    transcript
        .answer(vec![outcome(
            "call_a",
            ran(ToolCallEnd::ToolError),
            "1 failed",
        )])
        .unwrap();
    transcript
        .record(&mut Vec::new(), fixed, ms(1_000), ms(300), 1)
        .unwrap();

    assert_eq!(serde_json::to_value(&transcript).unwrap(), document());
}

/// A transcript of `turns` turns, each calling `calls` tools and answering
/// them, built the way a run builds one.
fn grown(system: &str, prompt_text: &str, turns: usize, calls: usize) -> Transcript {
    let mut transcript = Transcript::new(system.to_owned());
    let mut input = said(prompt_text);
    for turn_n in 0..turns {
        let ids: Vec<String> = (0..calls).map(|n| format!("call_{turn_n}_{n}")).collect();
        let blocks = ids
            .iter()
            .map(|id| tool_use(id, "bash"))
            .chain(std::iter::once(text("On it.")))
            .collect();
        transcript
            .record(&mut input, response(blocks, 1, 1), ms(0), ms(1), 1)
            .unwrap();
        if calls > 0 {
            transcript
                .answer(ids.iter().map(|id| ok(id, "out")).collect())
                .unwrap();
        } else {
            break;
        }
    }
    transcript
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(400))]

    /// Whatever a run does, the document it publishes has the run's system
    /// prompt, a turn for each of its turns, and a version. lablet doesn't
    /// read one back, so what's held here is that publishing is total: no
    /// shape a run can reach fails to render.
    #[test]
    fn every_transcript_a_run_can_build_publishes_as_a_document(
        system in "[ -~]{0,40}",
        prompt_text in "[ -~]{1,40}",
        turns in 1usize..4,
        calls in 0usize..3,
    ) {
        proptest::prop_assume!(!prompt_text.trim().is_empty());
        let transcript = grown(&system, &prompt_text, turns, calls);

        let value = serde_json::to_value(TranscriptDocument::of(&transcript)).unwrap();

        proptest::prop_assert_eq!(&value["schema_version"], &json!(TRANSCRIPT_SCHEMA_VERSION));
        proptest::prop_assert_eq!(value["system"].as_str().unwrap(), transcript.system());
        proptest::prop_assert_eq!(
            value["turns"].as_array().unwrap().len(),
            transcript.turns().len()
        );
        proptest::prop_assert_eq!(
            &value["turns"],
            &serde_json::to_value(transcript.turns()).unwrap()
        );
    }
}
