//! The loop the cases run: a scripted provider and tools that answer from a
//! table, on tokio's clock, around whatever observer a case is handed.
//!
//! A case is called from a test that pauses tokio's clock, so what a script
//! says a call took is what the loop measures.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lablet_model::{
    CacheScope, CompletionMode, FinishedRun, KeptOutput, OutputCap, OutputCut, Prompts,
    RequestParams, RunContext, RunId, RunLabels, Thinking, ToolConcurrency, ToolName, ToolSource,
    ToolSpec,
};
use lablet_policy::{RetryPolicy, RetrySettings, StopPolicy};
use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
use lablet_run::{
    CallLimits, Cancellation, Clock, RunEvent, RunObserver, RunService, ToolCall, ToolError,
    ToolErrorKind, ToolExecutor, ToolFilter, ToolOutput, ToolSet,
};
use serde_json::json;

use crate::must;

/// The ids of the runs the cases make.
pub(super) const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
pub(super) const OTHER_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";

/// The longest output the model is sent whole, in bytes. What `dump`
/// answers is longer, and nothing else is.
const OUTPUT_CAP_BYTES: u64 = 160;

/// A run that does something of everything a run's totals count: provider
/// call attempts that fail, with and without usage to report, calls to four
/// tools, of which one call returns an error and two have their output cut,
/// a call whose arguments didn't parse, and a call to a tool the run
/// doesn't have.
pub(super) const EVERYTHING: &str = r#"
- error:
    kind: retryable
    message: 529 overloaded
    usage: { input_tokens: 800, output_tokens: 3, cache_read_tokens: 600, cache_write_tokens: 50 }
    retry_after: 2s
    latency: 40ms
- response:
    content:
      - text: I'll run the tests and read the parser.
      - tool_use: { id: call_1, name: bash, input: { json: { command: cargo test --quiet } } }
      - tool_use: { id: call_2, name: read_file, input: { json: { path: src/parser.rs } } }
      - tool_use: { id: call_3, name: dump, input: { json: { table: tokens } } }
    usage: { input_tokens: 1000, output_tokens: 50, cache_read_tokens: 200, cache_write_tokens: 90 }
    finish: tool_use
    latency: 250ms
- response:
    content:
      - tool_use: { id: call_4, name: no_such_tool, input: { json: {} } }
      - tool_use: { id: call_5, name: bash, input: { unparsed: '{"command": "car' } }
      - tool_use: { id: call_6, name: grep, input: { json: { pattern: fn parse } } }
      - tool_use: { id: call_7, name: dump, input: { json: { table: productions } } }
    usage: { input_tokens: 1100, output_tokens: 20, reasoning_output_tokens: 5 }
    finish: tool_use
    latency: 100ms
- error: { kind: retryable, message: 529 overloaded, latency: 30ms }
- error: { kind: retryable, message: 529 overloaded, usage: { input_tokens: 10 }, latency: 35ms }
- error: { kind: retryable, message: 529 overloaded, latency: 45ms }
- response:
    content:
      - text: The parser test passes now.
    usage: { input_tokens: 1200, output_tokens: 30 }
    finish: end_turn
    latency: 120ms
"#;

/// A run whose only provider call fails for good.
pub(super) const FAILS: &str = r"
- error: { kind: fatal, message: unknown model, latency: 15ms }
";

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

/// An observer that keeps nothing, for the run another run is compared
/// with.
pub(super) struct Unobserved;

#[async_trait::async_trait]
impl RunObserver for Unobserved {
    async fn on(&self, _: RunEvent) {}
}

/// Serves four tools, each of which answers the same every time, after the
/// same wait. One of them is an MCP server's, so an observer is handed both
/// sources.
struct Tools;

fn spec(
    name: &str,
    description: &str,
    source: ToolSource,
    concurrency: ToolConcurrency,
) -> ToolSpec {
    ToolSpec {
        name: tool_name(name),
        description: description.to_owned(),
        input_schema: json!({ "type": "object" }),
        source,
        concurrency,
    }
}

fn tool_name(name: &str) -> ToolName {
    must(ToolName::new(name), "naming a tool")
}

#[async_trait::async_trait]
impl ToolExecutor for Tools {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError> {
        let docs = ToolSource::Mcp {
            server: "docs".to_owned(),
        };
        Ok(vec![
            spec(
                "bash",
                "Runs a command.",
                ToolSource::Builtin,
                ToolConcurrency::Exclusive,
            ),
            spec(
                "read_file",
                "Reads a file.",
                ToolSource::Builtin,
                ToolConcurrency::Shared,
            ),
            spec("grep", "Searches the files.", docs, ToolConcurrency::Shared),
            spec(
                "dump",
                "Prints a table.",
                ToolSource::Builtin,
                ToolConcurrency::Exclusive,
            ),
        ])
    }

    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError> {
        let (wait, says, is_error) = match call.name.as_str() {
            "bash" => (1_000, "test result: ok. 2 passed".to_owned(), false),
            "read_file" => (300, "pub fn parse(input: &str) -> Ast".to_owned(), false),
            "grep" => (20, "no file matches".to_owned(), true),
            "dump" => (70, "row ".repeat(50), false),
            other => {
                return Err(ToolError::new(
                    ToolErrorKind::Unknown,
                    format!("no tool is called {other}"),
                ));
            }
        };
        tokio::time::sleep(Duration::from_millis(wait)).await;
        let mut output = KeptOutput::new(call.keep);
        output.push(&says);
        Ok(ToolOutput {
            output,
            is_error,
            mcp: None,
        })
    }
}

/// The loop around `script`, a YAML script, telling `observer` of its runs.
pub(super) async fn playing(script: &str, observer: Arc<dyn RunObserver>) -> RunService {
    let script = Script::read(ScriptSource {
        name: "conformance.yaml",
        text: script,
        format: ScriptFormat::Yaml,
    });
    let script = must(script, "reading the script");
    let offered = ToolSet::build(
        vec![Arc::new(Tools) as _],
        &ToolFilter::default(),
        CompletionMode::Natural,
        None,
    )
    .await;
    let retry = RetryPolicy::new(RetrySettings {
        max_retries: 3,
        base: Duration::from_millis(100),
        max: Duration::from_secs(10),
        factor: 2.0,
        hint_max: Duration::from_secs(60),
        jitter: 0.0,
    });
    let output_cap = OutputCap::new(OUTPUT_CAP_BYTES, OutputCut::Head);
    RunService::new(
        Arc::new(FakeProvider::new("scripted-1", script)),
        Arc::new(must(offered, "settling the tools")),
        observer,
        Arc::new(TokioClock),
        Arc::new(NeverCancelled),
        StopPolicy {
            max_turns: None,
            timeout: Duration::from_secs(3_600),
            max_total_tokens: None,
            max_consecutive_invalid_turns: NonZeroU32::new(3),
        },
        must(retry, "making the retry policy"),
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
            output_cap: Some(must(output_cap, "making the output cap")),
            max_concurrent_tool_calls: NonZeroU32::MIN,
        },
    )
}

/// One run of `service` under the id `run`.
pub(super) async fn run(service: &mut RunService, run: &str) -> FinishedRun {
    let context = RunContext {
        run_id: must(RunId::new(run), "naming the run"),
        labels: RunLabels::default(),
        started_unix_ms: 1_790_000_000_000,
        config_digest: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
            .to_owned(),
        agent_version: "0.4.2".to_owned(),
        transcript_path: None,
        skills_count: 0,
        mcp: None,
        capture_content: false,
    };
    let prompts = Prompts::new(
        "You fix tests, tersely.",
        "Fix the failing test in the parser.",
    );
    service
        .run(context, must(prompts, "making the prompts"))
        .await
}
