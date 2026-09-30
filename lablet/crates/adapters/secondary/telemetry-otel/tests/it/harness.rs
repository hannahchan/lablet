//! The loop, built around a scripted provider and a tool executor that
//! answers from a table, on tokio's clock, with the observer exporting to a
//! file. A test pauses the clock, so what a script says a call took is what
//! the loop measures and what a span lasts.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lablet_conformance::otlp::{Attributes, Exported, LogRecord, Span};
use lablet_model::{
    FinishedRun, KeptOutput, McpServers, Prompts, RequestParams, RunContext, RunLabels, ToolCallId,
    ToolConcurrency, ToolName, ToolSource, ToolSpec,
};
use lablet_policy::Pricing;
use lablet_provider_fake::FakeProvider;
use lablet_run::{
    RunService, ToolCall, ToolError, ToolErrorKind, ToolExecutor, ToolOutput, TraceContext,
};
use lablet_telemetry_otel::{FileTarget, FlushError, OtelObserver};
use lablet_test_support::{CancelledAfter, RunBuilder, Scratch, context, request, scripted};
use serde_json::json;

pub use lablet_test_support::{
    AGENT_VERSION as VERSION, CONFIG_DIGEST, MODEL, PROMPT, STARTED_UNIX_MS, SYSTEM,
};

pub const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";
pub const OTHER_RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNED";

/// What `bash` answers, a second after it's called.
pub const BASH_SAYS: &str = "test result: ok. 2 passed";
/// What `read_file` answers, 300 ms after it's called.
pub const READ_FILE_SAYS: &str = "pub fn parse(input: &str) -> Ast";

/// A failed attempt, a response that calls both tools, a response that
/// calls a tool the run doesn't have, and a response that ends the run. Its
/// namesakes in other crates differ on purpose: each calls the tools its
/// own loop serves and states what its own tests count.
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

/// The file of the run `run` in `scratch`, when each run has its own.
pub fn file_of(scratch: &Scratch, run: &str) -> PathBuf {
    scratch.at(&format!("lablet-{run}.otlp.jsonl"))
}

/// What the model is told `bash` does, unless a test says otherwise.
pub const BASH_DOES: &str = "Runs a command.";

/// Serves `bash` and `read_file`, each of which answers the same text
/// every time, after the same wait, and keeps what each call carried.
pub struct Tools {
    bash_does: String,
    bash_concurrency: ToolConcurrency,
    read_file_source: ToolSource,
    propagated: Mutex<Vec<(ToolCallId, Option<TraceContext>)>>,
}

impl Default for Tools {
    fn default() -> Self {
        Self::new(BASH_DOES, ToolConcurrency::Exclusive, ToolSource::Builtin)
    }
}

impl Tools {
    fn new(
        bash_does: &str,
        bash_concurrency: ToolConcurrency,
        read_file_source: ToolSource,
    ) -> Self {
        Self {
            bash_does: bash_does.to_owned(),
            bash_concurrency,
            read_file_source,
            propagated: Mutex::default(),
        }
    }

    /// The span each call was handed to propagate, in the order the calls
    /// came.
    pub fn propagated(&self) -> Vec<(ToolCallId, Option<TraceContext>)> {
        self.propagated.lock().unwrap().clone()
    }
}

fn spec(
    name: &str,
    description: &str,
    source: ToolSource,
    concurrency: ToolConcurrency,
) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name).unwrap(),
        description: description.to_owned(),
        input_schema: json!({ "type": "object" }),
        source,
        concurrency,
    }
}

#[async_trait::async_trait]
impl ToolExecutor for Tools {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError> {
        Ok(vec![
            spec(
                "bash",
                &self.bash_does,
                ToolSource::Builtin,
                self.bash_concurrency,
            ),
            spec(
                "read_file",
                "Reads a file.",
                self.read_file_source.clone(),
                ToolConcurrency::Shared,
            ),
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
            ToolSource::Builtin,
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
    /// The system prompt.
    pub system: String,
    /// What the model is told `bash` does.
    pub bash_does: String,
    /// Whether `bash` may run beside other calls.
    pub bash_concurrency: ToolConcurrency,
    /// Where `read_file` comes from.
    pub read_file_source: ToolSource,
    /// How many tool calls may run at once.
    pub max_concurrent_tool_calls: NonZeroU32,
    /// The cap on turns, when the runs have one.
    pub max_turns: Option<NonZeroU32>,
    /// What the runs are priced at, when they're priced.
    pub pricing: Option<Pricing>,
    /// Where the context says the transcript is written.
    pub transcript_path: Option<PathBuf>,
    /// How many skill files the context says the system prompt holds.
    pub skills_count: u32,
    /// The MCP servers the context says serve the runs.
    pub mcp: Option<McpServers>,
    /// How long after the loop is built its runs are cancelled, when they
    /// are.
    pub cancelled_after: Option<Duration>,
}

impl Settings {
    /// Runs that capture nothing, have no labels, retry three times, run
    /// one tool call at a time, and each have a file of their own in
    /// `scratch`. Their tools are all built in and `bash` runs alone. They
    /// have nothing a run may be without: no cap on turns, no pricing, no
    /// transcript, no skills and no MCP servers. Nothing cancels them.
    pub fn in_scratch(scratch: &Scratch) -> Self {
        Self {
            target: FileTarget::EachRun {
                directory: scratch.path().to_owned(),
            },
            capture_content: false,
            labels: RunLabels::default(),
            max_retries: 3,
            request: request(),
            system: SYSTEM.to_owned(),
            bash_does: BASH_DOES.to_owned(),
            bash_concurrency: ToolConcurrency::Exclusive,
            read_file_source: ToolSource::Builtin,
            max_concurrent_tool_calls: NonZeroU32::MIN,
            max_turns: None,
            pricing: None,
            transcript_path: None,
            skills_count: 0,
            mcp: None,
            cancelled_after: None,
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
    system: String,
    transcript_path: Option<PathBuf>,
    skills_count: u32,
    mcp: Option<McpServers>,
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
            system,
            bash_does,
            bash_concurrency,
            read_file_source,
            max_concurrent_tool_calls,
            max_turns,
            pricing,
            transcript_path,
            skills_count,
            mcp,
            cancelled_after,
        } = settings;
        let provider = scripted(script);
        let tools = Arc::new(Tools::new(&bash_does, bash_concurrency, read_file_source));
        let observer = OtelObserver::builder(VERSION)
            .resource(vec![
                ("team".to_owned(), "evals".to_owned()),
                ("deployment.environment.name".to_owned(), "ci".to_owned()),
            ])
            .file(target)
            .build();
        let mut builder = RunBuilder::new(Arc::clone(&provider) as _);
        if let Some(after) = cancelled_after {
            builder = builder.cancellation(Arc::new(CancelledAfter::new(after)));
        }
        let service = builder
            .tools(vec![Arc::clone(&tools) as _, Arc::new(Writer) as _])
            .observer(Arc::new(observer.clone()))
            .max_turns(max_turns)
            .max_retries(max_retries)
            .request(request)
            .pricing(pricing)
            .max_concurrent_tool_calls(max_concurrent_tool_calls)
            .build()
            .await;
        Self {
            observer,
            provider,
            tools,
            service,
            capture_content,
            labels,
            system,
            transcript_path,
            skills_count,
            mcp,
        }
    }

    /// One run under the id `run`, and nothing after it: the observer
    /// isn't flushed.
    pub async fn run(&mut self, run: &str) -> FinishedRun {
        let context = RunContext {
            labels: self.labels.clone(),
            transcript_path: self.transcript_path.clone(),
            skills_count: self.skills_count,
            mcp: self.mcp.clone(),
            capture_content: self.capture_content,
            ..context(run)
        };
        let prompts = Prompts::new(self.system.as_str(), PROMPT).unwrap();
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
    let path = file_of(&scratch, RUN);
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

    /// The spans of the run's tool calls, in the order the calls ended,
    /// which is the order they were made in only while calls run one at a
    /// time.
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
