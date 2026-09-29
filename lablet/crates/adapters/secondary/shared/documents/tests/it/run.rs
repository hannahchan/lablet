//! A run built the way the loop builds one, which is the only way a
//! transcript comes to exist.

use std::num::NonZeroU32;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use lablet_model::{
    Answer, CacheScope, CompletionMode, ContentBlock, FinishReason, KeptOutput, McpLifetime,
    McpServer, McpServers, ModelRef, OutputCap, OutputCut, Prompts, ProviderApi, ProviderResponse,
    RequestParams, Responded, Run, RunContext, RunId, RunLabels, RunSetup, Schedule, StopReason,
    Thinking, TokenCounts, ToolCallEnd, ToolCallId, ToolCallStatus, ToolConcurrency, ToolInput,
    ToolName, ToolSource, ToolSpec, ToolUse, Transcript, Usage,
};
use serde_json::json;

pub(crate) const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn run_id() -> RunId {
    RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap()
}

fn labels() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: None,
        trial: Some("3".to_owned()),
    }
}

pub(crate) fn model() -> ModelRef {
    ModelRef {
        api: ProviderApi::Messages,
        name: "model-2026".to_owned(),
        replays_reasoning: true,
    }
}

/// What names the run, and beside it what a context holds that no document
/// publishes, none of it left at a default a test couldn't tell from a gap.
pub(crate) fn context() -> RunContext {
    RunContext {
        run_id: run_id(),
        labels: labels(),
        started_unix_ms: 1_790_000_000_123,
        config_digest: "9f2c6a1d0b7e4c35a8f1d2e3b4c5a6978877665544332211ffeeddccbbaa0099"
            .to_owned(),
        agent_version: "0.1.0".to_owned(),
        resource: vec![("team".to_owned(), "evals".to_owned())],
        transcript_path: Some("out/transcript.json".into()),
        skills_count: 2,
        mcp: Some(
            McpServers::new(
                McpLifetime::Run,
                vec![McpServer {
                    name: "docs".to_owned(),
                    version: "1.4.0".to_owned(),
                }],
            )
            .unwrap(),
        ),
        capture_content: true,
    }
}

fn name(tool: &str) -> ToolName {
    ToolName::new(tool).unwrap()
}

/// The tools the run offered, in the order it offered them.
pub(crate) fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: name("bash"),
            description: "Runs a command in a shell.".to_owned(),
            input_schema: json!({
                "properties": { "command": { "type": "string" } },
                "required": ["command"],
                "type": "object",
            }),
            source: ToolSource::Builtin,
            concurrency: ToolConcurrency::Exclusive,
        },
        ToolSpec {
            name: name("search"),
            description: "Searches the docs.".to_owned(),
            input_schema: json!({ "type": "object" }),
            source: ToolSource::Mcp {
                server: "docs".to_owned(),
            },
            concurrency: ToolConcurrency::Shared,
        },
    ]
}

fn setup() -> RunSetup {
    RunSetup {
        run_id: run_id(),
        labels: labels(),
        model: model(),
        endpoint: None,
        tools: tools().into_iter().map(|spec| spec.name).collect(),
        tools_bytes: 0,
        tools_digest: String::new(),
        system_prompt_digest: String::new(),
        completion: CompletionMode::Natural,
        max_turns: None,
        timeout: Duration::from_secs(600),
        request: RequestParams {
            max_tokens: 4_096,
            temperature: None,
            thinking: Thinking::default(),
            effort: None,
            seed: None,
            cache_scope: CacheScope::default(),
        },
    }
}

/// A run that has been given its prompts and nothing else.
pub(crate) fn started() -> Run {
    Run::start(
        setup(),
        Prompts::new("You fix tests.", "Fix the failing test.").unwrap(),
    )
}

/// The transcript of a run that stopped before it received a response.
pub(crate) fn without_a_turn() -> Transcript {
    started()
        .finish(StopReason::Cancelled, ms(5), None, None, None, None)
        .transcript
}

fn call(id: &str, tool: &str, input: ToolInput) -> ContentBlock {
    ContentBlock::ToolUse(ToolUse {
        id: ToolCallId::new(id).unwrap(),
        name: name(tool),
        input,
    })
}

/// What the loop answers each call of the first turn with: a tool that ran,
/// reported an error and had its output cut, and a call whose arguments
/// didn't parse, which the loop answered itself.
fn answer(call: &ToolUse) -> Answer {
    if call.name.as_str() == "bash" {
        let cap = OutputCap::new(24, OutputCut::Head).unwrap();
        let mut output = KeptOutput::new(Some(cap.keeps()));
        output.push("test result: FAILED. 1 passed; 1 failed");
        Answer::measured(
            ToolCallStatus::ran(ToolSource::Builtin, ToolCallEnd::ToolError),
            output,
            Some(cap),
            ms(1_200),
            ms(30),
        )
    } else {
        Answer::measured(
            ToolCallStatus::MalformedInput,
            KeptOutput::whole("the arguments aren't valid JSON"),
            None,
            ms(1_240),
            ms(0),
        )
    }
}

/// The transcript of a run of two turns: the first took two attempts,
/// reasoned, and called two tools, and the second finished.
pub(crate) fn of_two_turns() -> Transcript {
    let mut run = started();
    let _ = run.failed_attempt(ms(0), ms(250), None);
    let looking = ProviderResponse::new(
        vec![
            ContentBlock::Thinking {
                text: "The test names the function.".to_owned(),
                signature: Some("c2lnbmF0dXJl".to_owned()),
            },
            ContentBlock::Text("Looking.".to_owned()),
            call(
                "call_a",
                "bash",
                ToolInput::Json(json!({ "command": "cargo test" })),
            ),
            call(
                "call_b",
                "search",
                ToolInput::Unparsed(r#"{"query": "assert_eq"#.to_owned()),
            ),
        ],
        Usage::from_inclusive(TokenCounts {
            input: 100,
            output: 20,
            reasoning: None,
            cache_read: None,
            cache_write: None,
        }),
        FinishReason::from("tool_use".to_owned()),
        Some("msg_01".to_owned()),
        Some("model-2026-09-01".to_owned()),
    )
    .unwrap();
    let Responded::Pending(pending) = run.responded(looking, ms(300), ms(800)) else {
        panic!("the response called tools");
    };
    let schedule = Schedule {
        max_concurrent: NonZeroU32::MIN,
        concurrency: &|_| ToolConcurrency::Exclusive,
    };
    let run = block_on(pending.answer(schedule, |call| async move { answer(&call) }));

    let fixed = ProviderResponse::new(
        vec![ContentBlock::Text("Fixed.".to_owned())],
        Usage::from_inclusive(TokenCounts {
            input: 180,
            output: 5,
            reasoning: Some(0),
            cache_read: Some(100),
            cache_write: None,
        }),
        FinishReason::from("stop".to_owned()),
        None,
        None,
    )
    .unwrap();
    let Responded::Final(last) = run.responded(fixed, ms(1_300), ms(300)) else {
        panic!("the response called no tool");
    };
    last.finish(StopReason::Completed, ms(1_600), None, None, None, None)
        .transcript
}

/// Polls `future` to completion on this thread. The futures here are ready
/// when they're first polled, so a bound on the polls is a bound on a test
/// that would otherwise hang.
fn block_on<F: Future>(future: F) -> F::Output {
    const POLLS: usize = 1_000;
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    for _ in 0..POLLS {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
    panic!("the future didn't finish in {POLLS} polls");
}
