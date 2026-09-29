//! Runs of the loop itself, around a scripted provider and the built-in
//! tools, on the clock as it runs.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lablet_model::{
    CacheScope, CompletionMode, FinishedRun, Prompts, RequestParams, RunContext, RunId, RunLabels,
    StopReason, Thinking, ToolCallOutcome, ToolResultContent,
};
use lablet_policy::{RetryPolicy, RetrySettings, StopPolicy};
use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
use lablet_run::{
    CallLimits, Cancellation, Clock, RunEvent, RunObserver, RunService, ToolFilter, ToolSet,
};
use lablet_tools_builtin::{BuiltinTools, Settings};

use crate::harness::{Scratch, link};

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";

const SECRET: &str = "what the model is not to read";

/// A variable cargo sets for every test, as a key is set for lablet.
const NOT_FOR_A_COMMAND: &str = "CARGO_MANIFEST_DIR";

struct TokioClock;

#[async_trait::async_trait]
impl Clock for TokioClock {
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into_std()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct Unobserved;

#[async_trait::async_trait]
impl RunObserver for Unobserved {
    async fn on(&self, _: RunEvent) {}
}

/// One run of the YAML script `script`, whose tools are `tools`.
async fn run(script: &str, tools: BuiltinTools) -> FinishedRun {
    let script = Script::read(ScriptSource {
        name: "run.yaml",
        text: script,
        format: ScriptFormat::Yaml,
    })
    .unwrap();
    let tools = ToolSet::build(
        vec![Arc::new(tools) as _],
        &ToolFilter::default(),
        CompletionMode::Natural,
        None,
    )
    .await
    .unwrap();
    let retry = RetryPolicy::new(RetrySettings {
        max_retries: 0,
        base: Duration::from_millis(100),
        max: Duration::from_secs(10),
        factor: 2.0,
        hint_max: Duration::from_secs(60),
        jitter: 0.0,
    })
    .unwrap();
    let mut service = RunService::new(
        Arc::new(FakeProvider::new("scripted-1", script)),
        Arc::new(tools),
        Arc::new(Unobserved),
        Arc::new(TokioClock),
        Arc::new(NeverCancelled),
        StopPolicy {
            max_turns: None,
            timeout: Duration::from_secs(600),
            max_total_tokens: None,
            max_consecutive_invalid_turns: NonZeroU32::new(3),
        },
        retry,
        RequestParams {
            max_tokens: 4_096,
            temperature: None,
            thinking: Thinking::ProviderDefault,
            effort: None,
            seed: None,
            cache_scope: CacheScope::Shared,
        },
        None,
        CallLimits {
            provider_timeout: Duration::from_secs(60),
            output_cap: None,
            max_concurrent_tool_calls: NonZeroU32::MIN,
        },
    );
    let context = RunContext {
        run_id: RunId::new(RUN).unwrap(),
        labels: RunLabels::default(),
        started_unix_ms: 1_790_000_000_000,
        config_digest: "0".repeat(64),
        agent_version: "0.1.0".to_owned(),
        resource: Vec::new(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    };
    let prompts = Prompts::new("You fix tests.", "Fix the failing test.").unwrap();
    service.run(context, prompts).await
}

/// What the model was sent of a call: its status, and its text.
fn sent(outcome: &ToolCallOutcome) -> (&'static str, String) {
    let text = outcome
        .content
        .iter()
        .map(|ToolResultContent::Text(text)| text.as_str())
        .collect();
    (outcome.status.as_str(), text)
}

#[tokio::test]
async fn a_command_past_the_timeout_is_an_error_result_of_kind_timeout_and_the_run_goes_on() {
    let scratch = Scratch::new("run-timeout");
    let tools = BuiltinTools::new(Settings {
        timeout: Duration::from_millis(50),
        ..scratch.settings()
    })
    .unwrap();

    let finished = run(
        r"
- response:
    content:
      - tool_use: { id: call_1, name: bash, input: { json: { command: sleep 60 } } }
    finish: tool_use
- response:
    content:
      - tool_use: { id: call_2, name: bash, input: { json: { command: echo on it goes } } }
    finish: tool_use
- response:
    content:
      - text: The command takes too long.
    finish: end_turn
",
        tools,
    )
    .await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!(outcome.result().text, "The command takes too long.");
    assert_eq!((outcome.turns, outcome.tool_calls), (3, 2));
    assert_eq!(finished.summary.tool_calls.errors, 1);
    let turns = finished.transcript.turns();
    let timed_out = &turns[0].tool_calls()[0];
    assert_eq!(timed_out.call_id.as_str(), "call_1");
    assert!(timed_out.status.is_error());
    assert_eq!(
        sent(timed_out),
        (
            "timeout",
            "bash was stopped after 50ms, the longest the call could take".to_owned()
        )
    );
    assert!(
        (50..10_000).contains(&timed_out.latency_ms),
        "the call took {} ms",
        timed_out.latency_ms
    );
    assert_eq!(
        sent(&turns[1].tool_calls()[0]),
        ("ok", "on it goes\nexit code: 0".to_owned())
    );
}

#[tokio::test]
async fn a_run_is_refused_what_is_outside_the_root_and_kept_from_lablet_s_environment() {
    let scratch = Scratch::new("run-refusals");
    std::fs::write(scratch.outside("secret.txt"), SECRET).unwrap();
    link(
        &scratch.outside("secret.txt"),
        &scratch.root().join("notes.txt"),
    );
    let held = std::env::var(NOT_FOR_A_COMMAND).expect("cargo sets it for a test");

    let finished = run(
        r"
- response:
    content:
      - tool_use: { id: call_1, name: read_file, input: { json: { path: ../secret.txt } } }
      - tool_use: { id: call_2, name: read_file, input: { json: { path: notes.txt } } }
      - tool_use: { id: call_3, name: bash, input: { json: { command: env } } }
      - tool_use: { id: call_4, name: bash, input: { json: { command: exit 3 } } }
    finish: tool_use
- response:
    content:
      - text: Done.
    finish: end_turn
",
        scratch.tools(),
    )
    .await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::Completed);
    assert_eq!((outcome.turns, outcome.tool_calls), (2, 4));
    assert_eq!(finished.summary.tool_calls.errors, 2);
    let calls = finished.transcript.turns()[0].tool_calls();
    let ids: Vec<&str> = calls.iter().map(|call| call.call_id.as_str()).collect();
    assert_eq!(ids, ["call_1", "call_2", "call_3", "call_4"]);
    assert_eq!(
        sent(&calls[0]),
        (
            "tool_error",
            "../secret.txt wasn't read: it leads outside the run's root directory".to_owned()
        )
    );
    assert_eq!(
        sent(&calls[1]),
        (
            "tool_error",
            "notes.txt wasn't read: it leads outside the run's root directory".to_owned()
        )
    );
    let (status, environment) = sent(&calls[2]);
    assert_eq!(status, "ok");
    assert!(environment.contains("PATH="), "{environment}");
    assert!(
        !environment.contains(NOT_FOR_A_COMMAND) && !environment.contains(&held),
        "{environment}"
    );
    assert_eq!(sent(&calls[3]), ("ok", "exit code: 3".to_owned()));
    assert!(!calls[3].status.is_error());
}
