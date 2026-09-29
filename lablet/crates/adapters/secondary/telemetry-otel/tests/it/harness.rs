//! The loop, built around a scripted provider and a tool executor that
//! answers from a table, on tokio's clock, with the observer exporting to a
//! file. A test pauses the clock, so what a script says a call took is what
//! the loop measures and what a span lasts.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lablet_conformance::otlp::{Attributes, Exported, LogRecord, Span};
use lablet_model::{
    CacheScope, CompletionMode, FinishedRun, KeptOutput, Prompts, RequestParams, RunContext, RunId,
    RunLabels, Thinking, ToolCallId, ToolConcurrency, ToolName, ToolSource, ToolSpec,
};
use lablet_policy::{RetryPolicy, RetrySettings, StopPolicy};
use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
use lablet_run::{
    CallLimits, Cancellation, Clock, RunService, ToolCall, ToolError, ToolErrorKind, ToolExecutor,
    ToolFilter, ToolOutput, ToolSet, TraceContext,
};
use lablet_telemetry_otel::{FileTarget, FlushError, OtelObserver};
use serde_json::json;

pub const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
pub const OTHER_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";
pub const STARTED_UNIX_MS: u64 = 1_790_000_000_000;
pub const CONFIG_DIGEST: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
pub const VERSION: &str = "0.4.2";
pub const MODEL: &str = "scripted-1";
pub const SYSTEM: &str = "You fix tests, tersely.";
pub const PROMPT: &str = "Fix the failing test in the parser.";

/// What `bash` answers, a second after it's called.
pub const BASH_SAYS: &str = "test result: ok. 2 passed";
/// What `read_file` answers, 300 ms after it's called.
pub const READ_FILE_SAYS: &str = "pub fn parse(input: &str) -> Ast";

/// A failed attempt, a response that calls both tools, a response that
/// calls a tool the run doesn't have, and a response that ends the run.
pub const FAILS_CALLS_ENDS: &str = r"
- error:
    kind: retryable
    message: 529 overloaded
    usage: { input_tokens: 800, cache_read_tokens: 600 }
    retry_after: 2s
    latency: 40ms
- response:
    content:
      - text: I'll run the tests and read the parser.
      - tool_use: { id: call_1, name: bash, input: { json: { command: cargo test --quiet } } }
      - tool_use: { id: call_2, name: read_file, input: { json: { path: src/parser.rs } } }
    usage: { input_tokens: 1000, output_tokens: 50, cache_read_tokens: 200 }
    finish: tool_use
    response_id: msg_01
    response_model: scripted-2026-09
    latency: 250ms
- response:
    content:
      - tool_use: { id: call_3, name: no_such_tool, input: { json: {} } }
    usage: { input_tokens: 1100, output_tokens: 20, reasoning_output_tokens: 5 }
    finish: tool_use
    latency: 100ms
- response:
    content:
      - text: The parser test passes now.
    usage: { input_tokens: 1200, output_tokens: 30 }
    finish: end_turn
    latency: 120ms
";

/// A directory of one test's own, removed when the test ends.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(test: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("lablet-otel-it-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }

    pub fn directory(&self) -> PathBuf {
        self.0.clone()
    }

    /// The file of the run `run`, when each run has its own.
    pub fn file_of(&self, run: &str) -> PathBuf {
        self.0.join(format!("lablet-{run}.otlp.jsonl"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

/// Serves `bash` and `read_file`, each of which answers the same text
/// every time, after the same wait, and keeps what each call carried.
#[derive(Default)]
pub struct Tools {
    propagated: Mutex<Vec<(ToolCallId, Option<TraceContext>)>>,
}

impl Tools {
    /// The span each call was handed to propagate, in the order the calls
    /// came.
    pub fn propagated(&self) -> Vec<(ToolCallId, Option<TraceContext>)> {
        self.propagated.lock().unwrap().clone()
    }
}

fn spec(name: &str, description: &str, concurrency: ToolConcurrency) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name).unwrap(),
        description: description.to_owned(),
        input_schema: json!({ "type": "object" }),
        source: ToolSource::Builtin,
        concurrency,
    }
}

#[async_trait::async_trait]
impl ToolExecutor for Tools {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError> {
        Ok(vec![
            spec("bash", "Runs a command.", ToolConcurrency::Exclusive),
            spec("read_file", "Reads a file.", ToolConcurrency::Shared),
        ])
    }

    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError> {
        self.propagated
            .lock()
            .unwrap()
            .push((call.id.clone(), call.trace_context.clone()));
        let (wait, says) = match call.name.as_str() {
            "bash" => (Duration::from_secs(1), BASH_SAYS.to_owned()),
            "read_file" => (Duration::from_millis(300), READ_FILE_SAYS.to_owned()),
            // A tool that's asked for `bytes` answers that many.
            "write" => (
                Duration::ZERO,
                "x".repeat(usize::try_from(call.input["bytes"].as_u64().unwrap()).unwrap()),
            ),
            other => {
                return Err(ToolError::new(
                    ToolErrorKind::Unknown,
                    format!("no tool is called {other}"),
                ));
            }
        };
        tokio::time::sleep(wait).await;
        let mut output = KeptOutput::new(call.keep);
        output.push(&says);
        Ok(ToolOutput {
            output,
            is_error: false,
            mcp: None,
        })
    }
}

/// Serves `write`, which answers as many bytes as it's asked for.
struct Writer;

#[async_trait::async_trait]
impl ToolExecutor for Writer {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError> {
        Ok(vec![spec(
            "write",
            "Writes bytes.",
            ToolConcurrency::Exclusive,
        )])
    }

    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError> {
        Tools::default().execute(call).await
    }
}

/// What a test says of the runs it makes.
pub struct Settings {
    /// Where the observer exports to.
    pub target: FileTarget,
    /// Whether the runs capture content.
    pub capture_content: bool,
    /// What the run request named the runs.
    pub labels: RunLabels,
    /// How often a failed provider call is tried again.
    pub max_retries: u32,
    /// What every provider call is asked.
    pub request: RequestParams,
}

impl Settings {
    /// Runs that capture nothing, have no labels, retry three times, and
    /// each have a file of their own in `scratch`.
    pub fn in_scratch(scratch: &Scratch) -> Self {
        Self {
            target: FileTarget::EachRun {
                directory: scratch.directory(),
            },
            capture_content: false,
            labels: RunLabels::default(),
            max_retries: 3,
            request: RequestParams {
                max_tokens: 4_096,
                temperature: None,
                thinking: Thinking::ProviderDefault,
                effort: None,
                seed: None,
                cache_scope: CacheScope::Shared,
            },
        }
    }
}

/// The loop and its observer.
pub struct Harness {
    pub observer: OtelObserver,
    pub provider: Arc<FakeProvider>,
    pub tools: Arc<Tools>,
    service: RunService,
    capture_content: bool,
    labels: RunLabels,
}

impl Harness {
    /// The loop around the YAML script `script`.
    pub async fn playing(script: &str, settings: Settings) -> Self {
        let Settings {
            target,
            capture_content,
            labels,
            max_retries,
            request,
        } = settings;
        let script = Script::read(ScriptSource {
            name: "scripts/run.yaml",
            text: script,
            format: ScriptFormat::Yaml,
        })
        .unwrap();
        let provider = Arc::new(FakeProvider::new(MODEL, script));
        let tools = Arc::new(Tools::default());
        let observer = OtelObserver::builder(VERSION)
            .resource(vec![
                ("team".to_owned(), "evals".to_owned()),
                ("deployment.environment.name".to_owned(), "ci".to_owned()),
            ])
            .file(target)
            .build();
        let offered = ToolSet::build(
            vec![Arc::clone(&tools) as _, Arc::new(Writer) as _],
            &ToolFilter::default(),
            CompletionMode::Natural,
            None,
        )
        .await
        .unwrap();
        let service = RunService::new(
            Arc::clone(&provider) as _,
            Arc::new(offered),
            Arc::new(observer.clone()),
            Arc::new(TokioClock),
            Arc::new(NeverCancelled),
            StopPolicy {
                max_turns: None,
                timeout: Duration::from_secs(3_600),
                max_total_tokens: None,
                max_consecutive_invalid_turns: NonZeroU32::new(3),
            },
            RetryPolicy::new(RetrySettings {
                max_retries,
                base: Duration::from_millis(100),
                max: Duration::from_secs(10),
                factor: 2.0,
                hint_max: Duration::from_secs(60),
                jitter: 0.0,
            })
            .unwrap(),
            request,
            None,
            CallLimits {
                provider_timeout: Duration::from_secs(60),
                output_cap: None,
                max_concurrent_tool_calls: NonZeroU32::MIN,
            },
        );
        Self {
            observer,
            provider,
            tools,
            service,
            capture_content,
            labels,
        }
    }

    /// One run under the id `run`, and nothing after it: the observer
    /// isn't flushed.
    pub async fn run(&mut self, run: &str) -> FinishedRun {
        let context = RunContext {
            run_id: RunId::new(run).unwrap(),
            labels: self.labels.clone(),
            started_unix_ms: STARTED_UNIX_MS,
            config_digest: CONFIG_DIGEST.to_owned(),
            agent_version: VERSION.to_owned(),
            resource: Vec::new(),
            transcript_path: None,
            skills_count: 0,
            mcp: None,
            capture_content: self.capture_content,
        };
        let prompts = Prompts::new(SYSTEM, PROMPT).unwrap();
        self.service.run(context, prompts).await
    }
}

/// A run, and what the observer exported of it.
pub struct Traced {
    pub finished: FinishedRun,
    /// The file, read back.
    pub exported: Exported,
    /// The file, as it was written.
    pub written: String,
    /// What the executor's calls were handed to propagate.
    pub propagated: Vec<(ToolCallId, Option<TraceContext>)>,
}

/// Plays `script` as one run that has a file of its own, and reads the file
/// back once the observer has been flushed.
pub async fn traced(test: &str, script: &str, settings: impl FnOnce(&mut Settings)) -> Traced {
    let scratch = Scratch::new(test);
    let mut of_the_run = Settings::in_scratch(&scratch);
    settings(&mut of_the_run);
    let mut harness = Harness::playing(script, of_the_run).await;

    let finished = harness.run(RUN).await;
    let flushed: Result<(), FlushError> = harness.observer.flush().await;

    flushed.unwrap();
    let path = scratch.file_of(RUN);
    Traced {
        finished,
        exported: Exported::read(&path).unwrap(),
        written: std::fs::read_to_string(&path).unwrap(),
        propagated: harness.tools.propagated(),
    }
}

impl Traced {
    /// The run's root span.
    pub fn root(&self) -> &Span {
        let roots = self.exported.spans_of("invoke_agent");
        assert_eq!(roots.len(), 1, "a run has one root span");
        roots[0]
    }

    /// The spans of the run's provider call attempts, in order.
    pub fn chats(&self) -> Vec<&Span> {
        self.exported.spans_of("chat")
    }

    /// The spans of the run's tool calls, in the order the calls ended.
    pub fn tools(&self) -> Vec<&Span> {
        self.exported.spans_of("execute_tool")
    }

    /// The run's wide event.
    pub fn wide(&self) -> &LogRecord {
        let wide = self.exported.records_of("lablet.run");
        assert_eq!(wide.len(), 1, "a run has one wide event");
        wide[0]
    }
}

/// The number `key` holds.
pub fn count(attributes: &Attributes, key: &str) -> u64 {
    attributes
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| panic!("`{key}` holds no count among {attributes:?}"))
}

/// The sum of what `key` holds over `spans`, where a span that doesn't
/// hold it adds nothing; `None` when none of them holds it.
pub fn sum(spans: &[&Span], key: &str) -> Option<u64> {
    spans
        .iter()
        .filter(|span| span.attributes.contains_key(key))
        .map(|span| count(&span.attributes, key))
        .reduce(|sum, count| sum + count)
}

/// Holds `attributes` to the registry's lists for their signal: every key
/// the signal always carries is there, and none is there that the signal
/// doesn't declare.
pub fn assert_declared(
    signal: &str,
    attributes: &Attributes,
    required: &[&str],
    declared: &[&str],
) {
    let keys: BTreeSet<&str> = attributes.keys().map(String::as_str).collect();
    let missing: Vec<_> = required
        .iter()
        .filter(|key| !keys.contains(**key))
        .collect();
    assert!(
        missing.is_empty(),
        "{signal} lacks {missing:?}, which the registry requires of it"
    );
    let undeclared: Vec<_> = keys.iter().filter(|key| !declared.contains(*key)).collect();
    assert!(
        undeclared.is_empty(),
        "{signal} holds {undeclared:?}, which the registry doesn't declare for it"
    );
}
