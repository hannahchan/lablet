# Lablet specification

Status: draft for build. Companion to [brief.md](brief.md), [quality-bar.md](quality-bar.md), [build-plan.md](build-plan.md), and [acceptance.md](acceptance.md).

## 1. What a run is

A run takes a config and a task prompt and executes one agent loop:

```
transcript = (system, turns: []);  input = prompt
loop:
    if cancelled or stop policy says stop (timeout, tokens): break          # point A
    response = provider.complete(system, messages(transcript, input), tool specs)   # retried per call on transient error
    transcript.turns += turn(input, response);  input = nothing             # the input it answers, the response, and the record of its provider call
    if stop policy says stop (refused, output_truncated, context_exhausted, completed, ended_without_completion): break   # point R; a task_complete call is recorded here, never executed
    outcomes = []
    for each tool_use block in response: outcomes += tools.execute(call)    # errors become error results; output over the cap is cut
    the turn's tool_calls = outcomes                                        # rendered as ONE message of results
    if cancelled or stop policy says stop (tool errors, turns, timeout, tokens): break   # point B
emit outcome
```

A turn is one model response with what prompted it and what came of it: the input the user supplied, the record of the provider call that produced the response, and the outcome of each tool call it made. The first turn's input is the task prompt; a later turn follows a tool phase and has none. The flat list a provider call sends is rendered from the turns: before each response comes one user message holding the results of the previous turn's tool calls and then the turn's input. The loop never assembles a message.

The stop policy is evaluated at three points: before each provider call (point A), after each provider response and before any tool runs (point R), and after each tool phase (point B). Cancellation, the run timeout, and the token budget are checked at A and B; the turn cap and the consecutive tool error cap at point B only. A run therefore never makes a provider call after the condition that should have stopped it. Point R reads only the response, so a response that finishes the task completes the run even when it also used up the timeout or the token budget. It reads the finish reason before the tool calls: a response the model refused, or one that was cut short, stops the run there and none of its tool calls runs.

When several conditions hold at one point, the first in this order is the stop reason, so one state always gives one reason:

| Point | Order                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A     | `cancelled`, `timeout`, `max_total_tokens`                                                                                                                                                                                                                                                                                                                                                                                              |
| R     | finish reason `refusal`: `refused`. Finish reason `max_tokens`: `output_truncated`. Finish reason `context_window`: `context_exhausted`. All three hold whatever the response called, in both modes. Otherwise explicit mode and `task_complete` called: `completed`, whatever else the response holds. Otherwise any tool call: the run goes on. Otherwise `completed` in natural mode and `ended_without_completion` in explicit mode |
| B     | `cancelled`, `tool_errors_exhausted`, `max_turns`, `timeout`, `max_total_tokens`                                                                                                                                                                                                                                                                                                                                                        |

Cancellation leads because the loop polls its port before it asks the policy, which doesn't see cancellation at all. The tool error cap comes next at B because it alone says the run was failing, not merely long. At R the finish reason leads because a response cut off at `max_tokens` or at the context window can end inside a tool call's arguments, and a cut-off input can still parse as a valid, smaller one. So a truncated response never has its tool calls executed, and a `task_complete` call in one doesn't complete the run. A finish reason lablet doesn't know (`FinishReason::Other`) is read as a normal end: from an OpenAI-compatible server that's the usual case, and the wide event carries every finish reason for whoever needs to tell.

Every limit is met when the run reaches it, not when it passes it. With `max_turns` 2 the run stops after the tool phase of turn 2, having made two provider calls; a response on turn 2 that finishes the task still completes the run, because point R comes first. The run stops with `timeout` once the elapsed time equals `run.timeout`, with `max_total_tokens` once input plus output tokens equal the budget, and with `tool_errors_exhausted` on the third consecutive error result when the cap is 3.

`run.max_total_tokens` counts billed tokens: the input plus output tokens of every provider call, summed. Each call sends the whole conversation again, so context that's re-sent counts on every turn. A ten-turn run whose context grows evenly to 20,000 tokens bills about 110,000 input tokens (2,000 + 4,000 + ... + 20,000), not 20,000, so it reaches a budget of 100,000 on its tenth call.

Every provider call, tool call, retry, and stop decision is reported to an observer, which is how telemetry leaves the process.

### ProviderResponse modes

| Mode                | Completes when                                                                                                                                                                                                                                                                                      | Ends without completing when                                                         |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `natural` (default) | The model returns a turn with no tool calls whose finish reason isn't `refusal`, `max_tokens`, or `context_window`.                                                                                                                                                                                 | Never. Other stop reasons still apply.                                               |
| `explicit`          | The model calls the built-in `task_complete` tool in a response with none of those three finish reasons. The call is intercepted by the loop, never executed, not counted in `tool_calls`, and its JSON argument is the run's structured result. Other tool calls in the same response are ignored. | The model returns a turn with no tool calls. Stop reason `ended_without_completion`. |

### Stop reasons

`completed`, `ended_without_completion`, `max_turns`, `timeout`, `max_total_tokens`, `output_truncated` (finish reason `max_tokens`; the response's tool calls, if any, aren't executed), `context_exhausted` (the provider rejected the request as too long, or cut the response short at the context window), `retries_exhausted`, `tool_errors_exhausted`, `cancelled`, `provider_error` (any other non-retryable error), `refused` (the model declined to answer or a content filter withheld the response; a refusal is never a completed run).

### Retries

- **Provider errors** are classified by the adapter as retryable (transport failure, rate limit, 5xx, overloaded, per-call timeout), context exhausted (the provider's context-length error), or fatal (auth, bad request, unknown model). Retryable errors are retried with exponential backoff up to `run.max_retries` times **per provider call**; the counter resets on success. `max_retries` counts retries, not attempts: `3` allows four attempts, and `0` never retries. `RetryPolicy` takes the same number. Attempts are numbered from 1, as `lablet.attempt` reports them. When attempt `n` fails and `n` is at most `run.max_retries`, the loop waits and tries again; otherwise the run stops with `retries_exhausted`. So with `max_retries: 3`, two failures and a success is a completed call with two retries, and four failures exhaust it. Before each wait the loop asks `StopPolicy::allows_wait`, so backoff can't carry a run far past the run timeout: if the elapsed time plus the wait reaches `run.timeout`, the run stops with `timeout` instead of sleeping. The wait after attempt `n` is `base * factor^(n - 1)` capped at `max`, with no jitter, so the same attempt always waits the same time. `base` and `max` come from `run.retry_backoff_base` and `run.retry_backoff_max`; `factor` is 2.
- **Malformed provider responses** (the adapter can't map the payload to the domain model, including a tool call whose arguments aren't valid JSON, or whose name or id isn't a valid `ToolName` or `ToolCallId`) count as retryable. That differs from a well-formed name that no tool has, which is an error result for the model. An OpenAI-compatible server that sends an empty or repeated call id gets a synthesised unique one from the adapter, not a `Malformed`.
- **Tool errors** aren't retried by lablet. The error is returned to the model as an error tool result. A call to a tool name that's not in `specs()` is also an error result. After `run.max_consecutive_tool_errors` consecutive error results the run stops with `tool_errors_exhausted` at point B. A successful tool call resets the counter.
- **Tool timeouts** (`run.tool_timeout`) are tool errors.
- **Tool output** longer than `tools.max_output_bytes` is cut by the loop, not by the tool: the tool's own text is kept from the start up to the cap, at a character boundary, and one line such as `[truncated: the first 100000 of 5242880 bytes]` ends it. The cap bounds the tool's text, not the result: the marker is added on top of it, so what the model receives is a few dozen bytes longer, once per call that was cut. Every tool is cut the same way, an error result included, and a cut result isn't an error.
- **Provider timeouts** (`run.provider_timeout`) are enforced per call by the adapter's HTTP client via the deadline on the request. The run-level `run.timeout` is checked at points A and B only, so a run may overrun it by at most one provider or tool call.

### Cancellation

Cancellation is polled at points A and B via the `Cancellation` port; the CLI wires it to Ctrl-C. A run in the middle of a long tool call stops after that call returns or times out. Interrupting a call in flight is an open question.

### Transcript

When `run.transcript_path` is set, the whole conversation is written there as JSON when the run ends, whatever the stop reason. The document is the serde form of `Transcript` (§3): `system` and `turns`. Each turn holds its `input`, which is the task prompt for the first turn and empty after that, since a run takes one prompt (the shape keeps room for a user who speaks again, which the loop doesn't offer yet); the model's `response`; the `record` of the provider call behind it (usage, finish reason, response id and model, when the call started as an offset from the start of the run, how long it took, and how many attempts it needed); and `tool_calls`, the outcome of each tool call the response made (call id, status, which says whether a tool ran and where it came from, start offset, latency, the size before the output cap when it cut, and the content the model was sent). A grader can therefore see a truncated or refused turn, which call failed and how, and where the time went, and the numbers agree with the run's totals because the totals are computed from them. A turn with no outcomes either called no tools or its tools never ran; its response says which. A run that received no response has no turns, so its transcript holds the system prompt alone. This is independent of telemetry and `capture_content`; it's what a grader lablet or a composing framework reads.

### Outcome

Written to stdout as one JSON document when the run ends, whatever the stop reason:

```json
{
  "run_id": "01J...",
  "stop_reason": "completed",
  "turns": 7,
  "usage": {
    "input_tokens": 0,
    "output_tokens": 0,
    "cache_read_tokens": 0,
    "cache_write_tokens": 0
  },
  "tool_calls": 5,
  "duration_ms": 12345,
  "result": {
    "text": "final assistant text",
    "structured": { "...": "task_complete argument, explicit mode only" }
  },
  "error": null
}
```

`turns` is the number of turns in the transcript, which is the number of model responses the run received, so a run whose first provider call fails has `turns: 0`. `result.text` is the concatenated `Text` blocks of the last turn's response (`Transcript::final_text`), or `""` if there is none. `result.structured` is always present: the `task_complete` argument when the run completed in explicit mode, `null` otherwise, natural mode included. `error` is `null` or a string, and is likewise always present. What the two hold follows from the stop reason. A run that failed (`retries_exhausted`, `provider_error`, `context_exhausted`, `tool_errors_exhausted`) always has an `error`: the provider's text, or lablet's own sentence when there is none, which names the context window for `context_exhausted`, the tool-error cap for `tool_errors_exhausted`, and a failed provider call otherwise. Every other run has `error: null`, and only a `completed` run has a `result.structured`. A document that breaks these rules is refused when it's read. `duration_ms` is a whole number of milliseconds. The document is the derived serde form of `RunOutcome` (§3), and `lablet/tests/fixtures/outcome.json` holds one example that a `lablet-model` test reads, compares with a value built in code, and writes back unchanged. `usage` totals are summed over every successful provider call; a failed attempt reports no usage (`ProviderError` carries none), and `lablet.provider.retries` counts the attempts made beyond the first of a call. A run whose only provider call fails fatally reports zero retries. Chat spans number `turns + retries`, plus one when the run ended on a failed provider call. `input_tokens` **includes** cached tokens, as the GenAI semantic conventions require; `cache_read_tokens` and `cache_write_tokens` are subsets of it, so consumers that want uncached input subtract them. `reasoning_output_tokens` is likewise a part of `output_tokens`, billed at the output rate by both providers.

Exit codes: `0` completed, `2` ended with any other stop reason (`refused` included), `1` the run never started (config error, MCP server failed to start, provider rejected the credentials on the first call before any `RunStarted` event is emitted).

### Wide event

When a run ends, every observer emits exactly one wide event: a single record carrying everything worth knowing about the run, so an analyst can answer most questions from one row without joining spans. It's emitted after `RunFinished`, whatever the stop reason, and is the last thing the run produces. Its content is `RunContext`, what only the composition root knows, plus `RunSummary`, what the loop knew and measured (§3), flattened to the attribute names below.

| Group    | Attributes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| identity | `gen_ai.conversation.id` and `session.id` (both the run id), `gen_ai.agent.name` (always `lablet`), `gen_ai.agent.version`, `lablet.config.digest`                                                                                                                                                                                                                                                                                                                                                                                        |
| setup    | `gen_ai.provider.name`, `gen_ai.request.model`, `server.address` and `server.port` (when the provider is reached over the network), `gen_ai.request.max_tokens`, `gen_ai.request.seed`, `gen_ai.request.reasoning.level`, `gen_ai.request.temperature`, `lablet.request.thinking`, `lablet.run.completion_mode`, `lablet.run.max_turns`, `lablet.run.timeout_ms`, `lablet.tools.names` (string[]), `lablet.tools.count`, `lablet.mcp.servers` (string[]), `lablet.prompt.system_bytes`, `lablet.prompt.user_bytes`, `lablet.skills.count` |
| outcome  | `lablet.run.stop_reason`, `error.type`, `lablet.run.error`, `lablet.run.duration_ms`, `lablet.run.turns`, `lablet.result.text_bytes`, `lablet.result.has_structured`, `lablet.run.transcript_path`                                                                                                                                                                                                                                                                                                                                        |
| provider | `lablet.provider.retries`, `lablet.provider.latency_ms.total`, `lablet.provider.latency_ms.max`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.reasoning.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_write.input_tokens`, `gen_ai.response.finish_reasons` (string[], one per call), `lablet.run.cost_usd` and the four `lablet.pricing.*_usd_per_mtok` rates (when pricing configured)                                                                                        |
| tools    | `lablet.tool_calls.total`, `lablet.tool_calls.errors`, `lablet.tool_calls.unknown`, `lablet.tool_calls.truncated`, `lablet.tool_calls.latency_ms.total`, `lablet.tool_calls.input_bytes.total`, `lablet.tool_calls.output_bytes.total`, `lablet.tool.calls.<name>`, `lablet.tool.errors.<name>`, `lablet.tool.latency_ms.<name>` (templates, int; only for a tool the run offered)                                                                                                                                                        |
| content  | `lablet.result.text` and `lablet.result.structured` (JSON string) only when `telemetry.capture_content` is on                                                                                                                                                                                                                                                                                                                                                                                                                             |

Naming rule: a GenAI or core semantic-convention attribute is used wherever one exists. A `lablet.*` attribute is added only when the run can't be described without it, and every one carries a one-line justification in the registry. Per-tool values use Weaver `template[int]` attributes, which allow a dynamic suffix only; there are no map-typed attributes. `gen_ai.usage.input_tokens` includes cached tokens and `gen_ai.usage.output_tokens` includes the reasoning tokens, so neither subset is ever added to its total; `run.max_total_tokens` counts `input + output` over every call. A run reports the rates it was priced at beside its cost, because a derived number a consumer can't recompute is a number it has to trust. Extensions considered and deferred are listed in the research catalogue, not here.

The wide event is an OTel log record and reaches every configured exporter (OTLP over the network, the OTLP/JSON file) identically. Spans remain the per-step detail; the wide event is the per-run row.

Aggregatability rules: the wide event has a fixed flat shape (no nested maps; template attributes with a bounded key set for per-tool values: a key exists only for a call the tool executor resolved to a tool it serves, and a call to a name it can't resolve, which the model is free to invent, counts in the totals and in `lablet.tool_calls.unknown`. What bounds the key set is the executor's own tool set, which §5 makes the port's contract; `lablet.tools.names` is the same set, reported separately); the join keys `gen_ai.conversation.id`, `session.id`, and `lablet.config.digest` appear on the wide event, on every span, and on every other log record so any consumer can group by them without a join; composer-supplied `telemetry.resource` attributes (task id, experiment id, and the like) live on the OTel Resource only, which travels with every export batch as OTLP defines, because their keys are chosen by the composer and can't be declared in the registry, and live-check reports undeclared span attributes as violations; only raw counts, bytes, tokens, and durations are emitted, never ratios or averages, since those belong to the aggregation.

## 2. Architecture

Explicit architecture, in the shape of the UsefulBytes repository, sized down. Rings are directory prefixes inside the `lablet/` workspace; directory `foo/bar/` is package `lablet-bar`, with three exceptions: `apps/lablet` is `lablet`, `tests/conformance` is `lablet-conformance`, and `tests/mcp-server` is `lablet-test-mcp-server`.

```
lablet/
  crates/domain/model            conversation, tools, usage, stop reasons, run identity, summary
  crates/domain/policy           pure stop and retry decisions, pricing
  crates/application/run         RunService (the loop) and its secondary ports
  crates/adapters/secondary/
    provider-anthropic           ModelProvider over the Anthropic Messages API
    provider-openai              ModelProvider over OpenAI-compatible chat completions (Ollama, vLLM, gateways)
    provider-fake                ModelProvider that plays scripted responses; a product feature, not test scaffolding
    tools-builtin                ToolExecutor for bash, read_file, write_file (task_complete lives in the loop)
    tools-mcp                    ToolExecutor over rmcp (stdio and streamable HTTP clients)
    telemetry-otel               RunObserver mapping events to OTel spans and logs once, with pluggable exporters: OTLP network, OTLP/JSON file
    shared/telemetry-registry    generated from the Weaver registry: attribute name constants and enums
  apps/lablet                    composition root: config, build(), CLI. Library and binary.
  telemetry/                     Weaver registry (YAML), vendored upstream registries, policies, templates
  tests/conformance              package lablet-conformance: shared ToolExecutor and RunObserver cases
  tests/mcp-server               package lablet-test-mcp-server: a tiny rmcp stdio server for tests
  xtask/                         (repo root, not a member) lint-layers, gates
```

Dependency direction, enforced by `cargo xtask lint-layers` (a workspace crate is placed by its path, an external crate is matched by name; dev-dependencies aren't held to the ring rules):

| Ring                                | May depend on                                                                          | May not use                                                                 |
| ----------------------------------- | -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| domain                              | domain                                                                                 | tokio, reqwest, tracing, opentelemetry, rmcp, tonic, axum, hyper            |
| application                         | domain                                                                                 | tokio, reqwest, opentelemetry, rmcp, tonic, axum, hyper (`tracing` allowed) |
| adapters and adapter shared kernels | application, domain, and the shared kernels of their own ring; never a sibling adapter | (no restriction)                                                            |
| composition root                    | everything except test support                                                         | (no restriction)                                                            |
| test support (`tests/*`)            | anything                                                                               | (no restriction)                                                            |

A listed name forbids its whole family: a crate matches when any `-` or `_` separated part of its name equals the listed name, so `opentelemetry` also forbids `opentelemetry_sdk` and `opentelemetry-otlp`. Every dependency of a member is inherited from `[workspace.dependencies]` (`lint-manifests` allows nothing else), so that table is where a rename (`package = "..."`) is resolved to the real crate name and where a workspace crate's path places it in a ring; a dependency that can't be read from it's an error, not a pass. Target-specific tables are walked. No crate outside `tests/` may list a `tests/` crate in `[dependencies]` or `[build-dependencies]`. A workspace member that falls in no ring is an error.

`serde` and `serde_json` are allowed in every ring: the domain model derives `Serialize` and `Deserialize` once, and every JSON surface (transcript, fake-provider scripts) reuses it. Provider wire formats are still separate types in their adapters.

There are no primary adapters. The composition root calls `RunService` directly, which is also the library surface.

Ports are object-safe `async_trait` traits, `Send + Sync`, injected as `Arc<dyn Trait>` through constructors. Ports live in the application crate that consumes them. Errors are `thiserror` enums owned by the layer that defines the contract; adapters map their own errors into the port's error at the `impl` boundary and never leak their types inward.

Windows isn't supported. Paths, the `bash` tool, and the musl release build assume a Unix host.

## 3. Domain model (`lablet-model`)

Pure types with serde derives. `serde_json::Value` is the JSON data currency. A rule about one value or about an aggregate is held by the type: fields are private wherever a rule would otherwise be a convention (identifiers, a provider response's content, the transcript and its turns, an outcome's stop reason, result, and error, a `Cost`, an `UnknownReason`), and reading such a type from its serde form goes through the same rule. `Run` keeps its fields private too, and has no serde form. Every other field is public.

```rust
// Identifiers: validated by `new` and, through serde `try_from`, on deserialisation. Each serialises as a bare string.
pub struct RunId(String);                 // non-empty, no surrounding whitespace; by convention a ULID, generated by the composition root
pub struct ToolCallId(String);            // non-empty, no surrounding whitespace; the provider's id for the call
pub struct ToolName(String);              // the above, and 1 to ToolName::MAX_LEN (64) of [a-zA-Z0-9_-], which is what both provider APIs accept
pub enum IdError { Empty { kind: &'static str }, SurroundingWhitespace { kind: &'static str, value: String }, ToolNameCharacter { value: String }, ToolNameTooLong { value: String } }

// Content
pub enum ContentBlock {                                               // one block of a model response; a tool result isn't one
    Text(String),
    Thinking { text: String, signature: Option<String> },            // Anthropic: text may be empty (display omitted), signature present; None for a provider that signs nothing
    RedactedThinking { data: String },                                // Anthropic: replayed unchanged
    ToolUse(ToolUse),
    Opaque { provider: ProviderKind, payload: serde_json::Value },   // any other provider-specific block, replayed unchanged
}
pub struct ToolUse { pub id: ToolCallId, pub name: ToolName, pub input: serde_json::Value }
impl ToolUse { pub fn input_bytes(&self) -> u64; }                   // byte length of `input` as compact JSON
pub enum UserContent { Text(String) }                                // what a user supplies to a turn: text today; nothing a model or a tool produces fits
pub enum ToolResultContent { Text(String) }                          // tool results are text; an enum so another kind can arrive without a serde break
impl ToolResultContent { pub fn omitted(kind: &str, mime_type: &str, bytes: u64) -> ToolResultContent; }   // Text("[image omitted: image/png, 48213 bytes]")

// Messages: the flat form a provider call sends. Rendered from the transcript and borrowed from it; serialise only.
pub enum Message<'a> { User { tool_results: Vec<ToolResult<'a>>, input: &'a [UserContent] }, Assistant(&'a [ContentBlock]) }   // a user message: the results of the turn before, then the input
pub struct ToolResult<'a> { pub call_id: &'a ToolCallId, pub content: &'a [ToolResultContent], pub is_error: bool }

// Transcript: private fields; built through Run, read through accessors, deserialised through the same rules
pub struct Transcript { system: String, turns: Vec<Turn> }
pub struct Turn { input: Vec<UserContent>, response: Vec<ContentBlock>, record: TurnRecord, tool_calls: Vec<ToolCallOutcome> }   // the first turn's input is the task prompt
pub struct TurnRecord { pub usage: Usage, pub finish: FinishReason, pub response_id: Option<String>, pub response_model: Option<String>, pub started_ms: u64, pub latency_ms: u64, pub attempts: u32 }
impl Transcript {
    pub fn system(&self) -> &str; pub fn turns(&self) -> &[Turn];
    pub fn final_text(&self) -> String;                               // the text of the last turn, or ""
}
impl Turn {
    pub fn input(&self) -> &[UserContent]; pub fn response(&self) -> &[ContentBlock]; pub fn record(&self) -> &TurnRecord; pub fn tool_calls(&self) -> &[ToolCallOutcome];
    pub fn tool_uses(&self) -> impl Iterator<Item = &ToolUse>;        // the n-th outcome answers the n-th call
    pub fn text(&self) -> String;                                     // the Text blocks, concatenated
    pub fn calls(&self, mode: CompletionMode) -> Calls;               // what the stop policy reads at point R
}
pub enum TranscriptError { Response(ResponseError), NothingFromTheUser { turn: usize }, UnansweredCalls { turn: usize, calls: Vec<String> }, AlreadyAnswered { turn: usize }, OutcomesDontAnswerCalls { calls: Vec<String>, outcomes: Vec<String> } }
pub enum Calls { None, Tools, TaskComplete }   // how a completion mode reads a response; Turn::calls is the only producer

// Provider
pub enum ProviderKind { Anthropic, Openai, Fake }
pub struct ModelRef { pub provider: ProviderKind, pub name: String }
pub struct Endpoint { pub host: String, pub port: u16 }
pub struct TokenCounts { pub input: u64, pub output: u64, pub reasoning: u64, pub cache_read: u64, pub cache_write: u64 }   // what a provider reports; `input` means what the constructor taking it says
pub struct Usage { pub input_tokens: u64, pub output_tokens: u64, pub reasoning_output_tokens: u64, pub cache_read_tokens: u64, pub cache_write_tokens: u64 }
impl Usage {
    pub fn from_inclusive(counts: TokenCounts) -> Usage;              // the provider's input count includes cached tokens (OpenAI-compatible)
    pub fn from_uncached(counts: TokenCounts) -> Usage;               // it leaves them out (Anthropic); both cache counts are added in
    pub fn total(&self) -> u64; pub fn uncached_input_tokens(&self) -> u64;                                // and Add, AddAssign, Default; all saturating
}
pub struct ProviderResponse { pub(crate) content: Vec<ContentBlock>, pub usage: Usage, pub finish: FinishReason, pub response_id: Option<String>, pub response_model: Option<String> }
impl ProviderResponse {
    pub fn new(content: Vec<ContentBlock>, usage: Usage, finish: FinishReason, response_id: Option<String>, response_model: Option<String>) -> Result<ProviderResponse, ResponseError>;
    pub fn content(&self) -> &[ContentBlock];
}
pub enum ResponseError { DuplicateToolUse { id: String } }
pub enum FinishReason { EndTurn, ToolUse, MaxTokens, ContextWindow, Refusal, Other(UnknownReason) }   // From<String> canonicalises and is the only way to an Other
pub struct UnknownReason(String);         // as_str(); private field, so normalisation can't be skipped
pub struct Rates { pub input: f64, pub output: f64, pub cache_read: f64, pub cache_write: f64 }   // USD per million tokens, each finite and at least 0, checked on both paths
pub struct RateError { pub name: &'static str, pub value: f64 }
pub struct Cost(f64);                     // USD; Cost::new(usd) -> Result<Cost, CostError>, usd()
pub struct CostError(f64);                // the amount wasn't a finite number of at least 0
pub struct RequestParams { pub max_tokens: u32, pub temperature: Option<f64>, pub thinking: Thinking, pub effort: Option<Effort>, pub seed: Option<u64> }
pub enum Thinking { ProviderDefault, Adaptive, Budget(NonZeroU32), Disabled }   // Default is ProviderDefault
pub enum Effort { Low, Medium, High, XHigh, Max }

// Tools
pub struct ToolSpec { pub name: ToolName, pub description: String, pub input_schema: serde_json::Value, pub source: ToolSource }
pub enum ToolSource { Builtin, Mcp { server: String } }   // Display: "builtin", "mcp:docs"; as_str: the variant only
pub enum ToolCallStatus { Unknown, Ran { source: ToolSource, ended: ToolCallEnd } }   // ran(source, ended); as_str flattens to `lablet.tool.status`, and to the span's error.type for every value but ok; source() -> Option<&ToolSource>
pub enum ToolCallEnd { Ok, ToolError, Timeout, Failed }
pub struct ToolCallOutcome { pub call_id: ToolCallId, pub status: ToolCallStatus, pub started_ms: u64, pub latency_ms: u64, pub truncated_from_bytes: Option<u64>, pub content: Vec<ToolResultContent> }
impl ToolCallOutcome {
    pub fn measured(call_id: ToolCallId, status: ToolCallStatus, content: Vec<ToolResultContent>, max_output_bytes: Option<u64>, started: Duration, latency: Duration) -> ToolCallOutcome;   // applies the output cap
    pub fn output_bytes(&self) -> u64;                                // summed byte length of the content the model was sent
}

// Run
pub enum CompletionMode { Natural, Explicit }
impl CompletionMode { pub const TASK_COMPLETE: &str; }   // "task_complete": the loop offers it, intercepts it, and Turn::calls reads it
pub enum StopReason { Completed, EndedWithoutCompletion, MaxTurns, Timeout, MaxTotalTokens, OutputTruncated, ContextExhausted, RetriesExhausted, ToolErrorsExhausted, Cancelled, ProviderError, Refused }
pub enum StopClass { Completed, Stopped, Failed }
impl StopReason { pub fn class(self) -> StopClass; }                  // Failed: context_exhausted, retries_exhausted, tool_errors_exhausted, provider_error
pub struct RunOutcome { pub run_id: RunId, stop_reason: StopReason, pub turns: u32, pub usage: Usage, pub tool_calls: u64, pub duration_ms: u64, result: TaskResult, error: Option<String> }
impl RunOutcome { pub fn stop_reason(&self) -> StopReason; pub fn result(&self) -> &TaskResult; pub fn error(&self) -> Option<&str>; }   // the three fields a rule ties together are private
pub enum OutcomeError { ErrorWithoutFailure { stop_reason: StopReason }, FailureWithoutError { stop_reason: StopReason }, StructuredWithoutCompletion { stop_reason: StopReason } }
pub struct TaskResult { pub text: String, pub structured: Option<serde_json::Value> }
pub struct RunContext { pub run_id: RunId, pub config_digest: String, pub agent_version: String, pub resource: Vec<(String, String)>, pub transcript_path: Option<PathBuf>, pub skills_count: u32, pub mcp_servers: Vec<String>, pub capture_content: bool }
pub struct ToolStats { pub calls: u64, pub errors: u64, pub latency_ms: u64 }   // one tool's share of the run; the `lablet.tool.*.<name>` templates
pub struct RunSummary { pub model: ModelRef, pub endpoint: Option<Endpoint>, pub tools: Vec<ToolName>, pub completion: CompletionMode, pub max_turns: NonZeroU32, pub timeout_ms: u64, pub request: RequestParams, pub prompt_system_bytes: u64, pub prompt_user_bytes: u64, pub provider_retries: u64, pub provider_latency_total_ms: u64, pub provider_latency_max_ms: u64, pub finish_reasons: Vec<FinishReason>, pub tool_calls_errors: u64, pub tool_calls_unknown: u64, pub tool_latency_total_ms: u64, pub tool_input_bytes: u64, pub tool_output_bytes: u64, pub tool_calls_truncated: u64, pub per_tool: BTreeMap<ToolName, ToolStats>, pub rates: Option<Rates>, pub cost: Option<Cost>, pub outcome: RunOutcome }
pub struct FinishedRun { pub summary: RunSummary, pub transcript: Transcript }   // what RunService::run returns
// RunSummary and FinishedRun serialise and don't deserialise: their invariants span fields, and only Run::finish establishes them.

// The running record of a run. Not serialised; these are the model's only types that hold a Duration.
pub struct RunSetup { pub run_id: RunId, pub model: ModelRef, pub endpoint: Option<Endpoint>, pub tools: Vec<ToolName>, pub completion: CompletionMode, pub max_turns: NonZeroU32, pub timeout: Duration, pub request: RequestParams }
pub struct Progress { pub turns: u32, pub elapsed: Duration, pub usage: Usage, pub consecutive_tool_errors: u32 }   // what the stop policy reads; Default
impl Run {
    pub fn start(setup: RunSetup, system: String, prompt: String) -> Run;   // the prompt waits as the first turn's input
    pub fn transcript(&self) -> &Transcript;
    pub fn messages(&self) -> Vec<Message<'_>>;                                 // what the next provider call sends
    pub fn failed_attempt(&mut self, latency: Duration);                        // a provider call attempt that failed
    pub fn completion(&mut self, completion: ProviderResponse, started: Duration, latency: Duration) -> Result<&Turn, TranscriptError>;   // one that succeeded becomes the next turn
    pub fn tool_calls(&mut self, outcomes: Vec<ToolCallOutcome>) -> Result<(), TranscriptError>;   // what happened to the last turn's tool calls
    pub fn progress(&self, elapsed: Duration) -> Progress;
    pub fn usage(&self) -> Usage;                                               // what the run's cost is priced from
    pub fn finish(self, stop: StopReason, duration: Duration, structured: Option<serde_json::Value>, error: Option<String>, rates: Option<Rates>, cost: Option<Cost>) -> FinishedRun;
}
```

`RunContext` holds what only the composition root knows and hands to the loop. `RunSummary` holds what the loop knew and measured: the limits and request parameters it was built with, the model and endpoint its provider reports, and everything it counted. Observers don't reconstruct it, so every observer reports identical numbers. The wide event is the two flattened together. `RunSummary` doesn't repeat what its `outcome` already holds: `gen_ai.usage.*` comes from `outcome.usage`, `lablet.tool_calls.total` from `outcome.tool_calls`, `lablet.run.turns` from `outcome.turns`, `lablet.run.duration_ms` from `outcome.duration_ms`, and `lablet.run.stop_reason` and `lablet.run.error` from `outcome.stop_reason()` and `outcome.error()`. `lablet.tools.count` is the length of `tools`, so no two fields can disagree.

**Transcript.** `Transcript` is the conversation of one run, the loop's working state, and the document a grader or a composer reads. It's made of turns, not messages. A `Turn` is the `input` the user supplied, the model's `response` to it, the `TurnRecord` of the provider call behind the response, and the `ToolCallOutcome` of each tool call it made. So a response can't be without its record, a result can't be without its call, and the roles aren't data that could be wrong. The first turn's input is the task prompt. A run takes one prompt, so every later turn's input is empty today; the shape keeps room for multi-turn user input, which the loop doesn't offer yet. Input is `UserContent`, which is text, so a tool call or a tool result in it isn't representable: results are rendered from outcomes and never stored as input.

`Run::messages()` renders the flat form for the next provider call, borrowing everything. Before each turn's response comes one `User` message that holds the results of the previous turn's tool calls, in call order, and then the turn's input, whichever of the two there are; the last message is the one the next turn will answer, made of the last turn's results and the input that's waiting; a finished run has neither, so its rendered list ends with the assistant. Anthropic accepts both in one user message, results first; an OpenAI-compatible server gets one `tool` message for each result and then a user message for the input. The rendered result's `is_error` is `ToolCallStatus::is_error`, true for every status but a tool that ran and ended `ok`, so the flag can't disagree with the status it comes from. Blocks are stored and rendered verbatim and in order, which Anthropic requires of `Thinking`, `RedactedThinking`, and `Opaque` blocks in the latest assistant turn. `Message` is an enum of the two roles, so `ContentBlock` needs no tool-result variant that a response could misuse.

Four rules remain that a structure doesn't hold, and they're enforced where a transcript changes and again when one is read from its serde form, through a private raw type that also refuses a field it doesn't know:

- Something from the user comes before every response: a turn has input, or the turn before it has tool call outcomes. Otherwise two assistant messages would be adjacent, or the conversation would open with one. So the first turn always has input. `Run::responded` refuses a turn that breaks this (`NothingFromTheUser`).
- The tool calls of one response have distinct ids. `ProviderResponse::new` refuses a repeat (`ResponseError::DuplicateToolUse`), which an adapter reports as `Malformed`, and a completion's `content` can't be set any other way.
- A turn's outcomes answer exactly the tool calls of its response, each call id once and in call order, or the turn has no outcomes at all because its tools never ran: the response was cut short or refused, its `task_complete` call was intercepted, or the run was cancelled or reached a limit first. `Run::tool_calls` refuses anything else.
- Only the last turn may have tool calls and no outcomes, whatever input the next turn would bring. `Run::responded` refuses a turn that would follow one (`UnansweredCalls`).

Whether a turn without outcomes called no tools or its tools never ran is read from whether its response holds tool calls; nothing else records it. The name and input of a call are in the response's `ToolUse` block and nowhere else, and an outcome's sizes are measured rather than stored (`ToolUse::input_bytes`, `ToolCallOutcome::output_bytes`), so nothing is written in two places that could disagree. The one size it stores is `truncated_from_bytes`, what the output measured before the cap cut it, which nothing left can measure.

**Empty text.** When a provider response becomes a turn, `Text` blocks that are empty or only whitespace are dropped from the response, and from the turn's input by the same rule, which is why a prompt of only whitespace is nothing from the user. This is the only way a stored turn differs from what the provider sent and from what the loop was handed. Models do emit such blocks, typically just before a tool call, and Anthropic refuses both kinds when the response is replayed; rejecting the completion instead would mark it `Malformed` and spend a retry on a block that carries nothing. The drop happens once, before anything reads the turn, so `final_text()` and every byte count taken from `Run::messages()` describe what's replayed. Reading a transcript document applies the same rule, so a hand-written document that holds a blank block normalises on its first read and is byte-identical on every read after that. A grader that reads a transcript and writes it back should expect that one change.

**Time.** The domain reads no clock. The loop passes each offset and latency as a `Duration` measured on its `Clock`, and the model turns it into whole milliseconds in one function, so a total and the values it sums are cut the same way and no observer rounds for itself. A turn's record holds `started_ms`, the offset from the start of the run at which the successful attempt began, its `latency_ms`, and `attempts`, the number of attempts the call took; an outcome holds `started_ms` and `latency_ms` of its tool call. A duration that reaches telemetry as a `*_ms` attribute is a `u64` field named `*_ms` (`duration_ms`, `timeout_ms`, the latency fields). No serialised type holds a `Duration`; `RunSetup`, `Progress`, and the parameters of `Run` and `ToolCallOutcome::measured` do.

**ATIF.** The transcript maps onto an ATIF v1.8 trajectory field for field, so the phase 10 export is a map and not a reconstruction. `system` is the `system` step that opens the trajectory, a turn's input, when it has any, is a step with source `user`, and the rest of the turn is one `agent` step with `llm_call_count` 1:

| ATIF                              | From                                                                                                                                                               |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Step.message` of the `user` step | the text of the turn's `input`                                                                                                                                     |
| `Step.timestamp`                  | the run's start plus `record.started_ms`                                                                                                                           |
| `Step.model_name`                 | `record.response_model`                                                                                                                                            |
| `Step.message`                    | `Turn::text()`                                                                                                                                                     |
| `Step.reasoning_content`          | the text of the response's `Thinking` blocks                                                                                                                       |
| `Step.tool_calls[]`               | the response's `ToolUse` blocks: `tool_call_id` is `id`, `function_name` is `name`, `arguments` is `input`                                                         |
| `Step.observation.results[]`      | the turn's `tool_calls`, in the same order: `source_call_id` is `call_id`, `content` is the text of `content`                                                      |
| `ObservationResult.extra`         | the outcome's `status`, which holds the source, and its `started_ms`, `latency_ms`, `truncated_from_bytes`: ATIF has no slot for an error, a time, or a truncation |
| `Step.metrics`                    | `prompt_tokens` is `usage.input_tokens`, `completion_tokens` is `usage.output_tokens`, `cached_tokens` is `usage.cache_read_tokens`, a subset in both              |
| `Metrics.extra`                   | `usage.cache_write_tokens`, and the record's `finish`, `response_id`, `latency_ms`, `attempts`                                                                     |

`RedactedThinking` and `Opaque` blocks are replay material for the provider that sent them and have no place in a trajectory.

**Tool calls.** A `ToolCallOutcome` is what happened to one call, and one value feeds the transcript, the totals, and the `execute_tool` span. `status` says in one value whether a tool ran: `Unknown`, no configured tool has the name, or `Ran { source, ended }`, where `ended` is `Ok`; `ToolError`, the tool ran and reported an error, as MCP's `isError` does; `Timeout`; or `Failed`, the executor failed. `as_str` flattens the two levels to the five `lablet.tool.status` values, of which every one but `ok` is the span's `error.type` (§5), and `source()` is a source exactly when a tool ran. So "the model called a tool the run doesn't have" is one fact, not a status, a missing source, and a name absent from `RunSetup::tools` that could disagree: `finish` reads the outcome's own status, and the executor, which resolved the name against the run's tools, is the only thing that decides it. `ToolCallOutcome::measured` is how the loop builds one: it converts the two durations and applies the output cap, `tools.max_output_bytes`, so every tool's output is cut the same way: text kept from the start up to the cap, at a character boundary, then the one line `[truncated: the first 100000 of 5242880 bytes]`, worded once in the model like the omitted-content line. `truncated_from_bytes` is the size before the cut, `None` when nothing was cut, and `output_bytes()` is the size the model was sent, that line included, which is why it can exceed the cap. Capping an output twice therefore cuts it twice; the loop applies the cap once, as each outcome is built. The cap is loop configuration (§5), not a model field.

**The run.** The loop doesn't assemble a `RunSummary` or a `Transcript` by hand. It starts a `Run` from a `RunSetup` and the two prompts, tells it each thing that happens, and `finish` turns it into the `FinishedRun`. It keeps the transcript and, beside it, only what a transcript doesn't hold: the input that waits for the next turn, which is the prompt until the first turn takes it, and the latencies of failed provider call attempts with how many the call in progress has had. Every total of the summary is computed from those when the run ends (usage, finish reasons, provider latency, retries, tool counts, bytes, per-tool shares, `result.text`, and the prompt sizes, where the task prompt's is the size of the first turn's input, or of the waiting input when no turn took it), so no quantity is held in two places and a summary always agrees with the transcript beside it. `progress` is computed the same way each time it's asked: a sum over the turns and a walk back from the last outcome to the last success, which is cheap next to a provider call. `turns` is the number of turns. A turn's `attempts` is the failed attempts since the turn before plus one, and the retries of a run are each turn's `attempts - 1` plus, for a call that never succeeded, its failed attempts less one: a retry is an attempt made beyond the first of its call, so a failure that nothing follows isn't one. Per-tool statistics exist only for a call whose status says a tool ran: the model can call any name, and a call to one the executor couldn't resolve counts in the totals and in `tool_calls_unknown` and creates no per-tool key. So the `lablet.tool.*.<name>` keys are bounded by what the executor resolves, not by `RunSetup::tools`. The two are the same set in a run, because a `ToolExecutor` resolves only the names it offered in `specs()` (§5) and `RunSetup::tools` is those names; the run reads the outcome rather than asking the tool list a second question it could answer differently. The intercepted `task_complete` call has no outcome, so it's in no total.

**Outcome.** `StopReason::class` sorts the stop reasons into `Completed`; `Failed`, a run that an error ended (`retries_exhausted`, `provider_error`, `context_exhausted`, `tool_errors_exhausted`); and `Stopped`, every other reason. A `RunOutcome` has an `error` exactly when its reason is `Failed` and may have a `result.structured` only when it's `Completed`. The three fields the rule ties together are private and read through methods: `Run::finish` is the one way to build an outcome, and it keeps of the `structured` and `error` it's given only what the class allows, giving a failure that came without text the reason's own sentence (for `context_exhausted`, that the response was cut short at the context window). So the loop passes what it has at hand, for example the argument of a `task_complete` call in a truncated response, and the outcome is still one a run can have. Reading an outcome from its serde form refuses a document that breaks the rules (`OutcomeError`). The class isn't written to the JSON or the wide event; the JSON is unchanged.

**Usage.** `input_tokens` includes the cached tokens; `cache_read_tokens` and `cache_write_tokens` are subsets of it. `total()` is `input_tokens + output_tokens` and is what `run.max_total_tokens` counts; `uncached_input_tokens()` is `input_tokens` less both cache fields. An adapter builds a `Usage` through the constructor named for its provider's convention, so the addition can't be forgotten: `from_uncached` for a provider that reports uncached input separately, as Anthropic does, and `from_inclusive` otherwise. Cache-related fields are zero for providers that don't report them.

**Cost.** `Cost::new` takes a finite `f64` of at least 0 and refuses anything else, on both the code and the serde path. JSON has no infinity or NaN, so serde writes either as `null`, which is also how a run with no pricing configured writes the field: a cost that overflowed would be indistinguishable from one never asked for. `Pricing::cost` (§4) is the only producer in a run and answers `Option<Cost>`, `None` when the rates and counts multiply out past an `f64`, which takes rates no real price list has.

**Finish reasons.** `FinishReason::from(String)` is how a provider's string becomes a reason, and serde reads through it. Every spelling either provider API uses for a known reason gives that reason (the table is in §6), so `Other("end_turn")` or `Other("stop")` can't come out of the conversion, and a fake script may use whichever spelling its author knows. A reason is written as its `as_str`: `end_turn`, `tool_use`, `max_tokens`, `context_window`, `refusal`, or the provider's own string for `Other`. `Other` carries an `UnknownReason`, whose string is private, so `From<String>` is the only way to build one and `Other("refusal")` isn't expressible: a refusal that skipped the conversion would read at point R as an ordinary end and complete the run.

**Serde form.** Derives and serde attributes only, with a private raw type where reading must check a rule (`ProviderResponse`, `Transcript`, `RunOutcome`). A `UserContent` is `{"text": "..."}`. Enums are externally tagged with `snake_case` names: a `ContentBlock` is `{"text": "..."}` or `{"tool_use": {"id": "...", "name": "...", "input": {}}}`, and the other variants follow the same pattern; a `ToolSource` is `"builtin"` or `{"mcp": {"server": "docs"}}`; a `ToolCallStatus` is `"unknown"` or `{"ran": {"source": "builtin", "ended": "ok"}}`, the two levels the type holds rather than the flattening `as_str` does for telemetry; a `Thinking` is `"provider_default"`, `"adaptive"`, `{"budget": 2048}`, or `"disabled"`, and a budget of 0 is refused. `Effort::XHigh` is `xhigh`. `Cost` is a bare number. An `Option` that's `None` is written as `null`, never left out; read, it may be left out, which is how a block of thinking without a signature is written by hand. A `Message` only serialises, as `{"user": {"tool_results": [{"call_id": "...", "content": [{"text": "..."}], "is_error": false}], "input": [{"text": "..."}]}}` or `{"assistant": [...]}`; that form is what the loop measures request bytes from (§5), and nothing reads it back. The forms a fake-provider script holds are strict where a slip would otherwise pass in silence: `Usage`, `ProviderResponse`, and `ToolUse` refuse a field they don't know, so `input_token: 12` is an error, not a zero. What may be left out reads as nothing: a `Usage` field as zero, a completion's `usage` as all zeros. The outcome document and a `ToolCallOutcome` refuse an unknown field for the same reason. `StopReason`, `CompletionMode`, `ToolSource` (the variant only), `ToolCallStatus`, `ToolCallEnd`, `FinishReason`, `ProviderKind`, and `Effort` each have an `as_str` that gives the serde spelling and a `Display` that prints it, except that a `ToolSource` prints an MCP tool's server too, as `mcp:docs`. A unit test in the model pins the wire spellings, and a test in `lablet-conformance`, which may depend on both crates, compares `StopReason`, `CompletionMode`, `ToolSource`, and `ToolCallStatus` with the generated registry enums through exhaustive matches, so a variant added on either side doesn't compile until the other has it.

**Tool results are text.** Lablet doesn't carry image, audio, or binary resource content from a tool. An executor replaces such an item with the one line `ToolResultContent::omitted` renders from its kind, MIME type, and byte length, for example `[image omitted: image/png, 48213 bytes]`, so the model and a reader of the transcript see one wording. `ToolUse::input_bytes`, `ToolCallOutcome::output_bytes`, and `Transcript::final_text` are each the one definition of the quantity they name: `lablet.tool.input.bytes`, `lablet.tool.output.bytes`, and `result.text`.

## 4. Domain policy (`lablet-policy`)

Pure functions over plain state, fully unit-testable without async:

```rust
pub struct StopPolicy { pub completion: CompletionMode, pub max_turns: NonZeroU32, pub timeout: Duration, pub max_total_tokens: Option<u64>, pub max_consecutive_tool_errors: NonZeroU32 }
impl StopPolicy {
    pub fn before_call(&self, progress: &Progress) -> Option<StopReason>;                        // point A of §1
    pub fn after_response(&self, finish: &FinishReason, calls: Calls) -> Option<StopReason>;     // point R
    pub fn after_tools(&self, progress: &Progress) -> Option<StopReason>;                        // point B
    pub fn allows_wait(&self, elapsed: Duration, wait: Duration) -> bool;                        // false when elapsed + wait reaches the timeout
}

pub struct RetryPolicy { max_retries: u32, base: Duration, max: Duration, factor: f64 }
impl RetryPolicy {
    pub fn new(max_retries: u32, base: Duration, max: Duration, factor: f64) -> Result<RetryPolicy, RetryPolicyError>;
    pub fn delay(&self, attempt: u32) -> Option<Duration>;   // the wait after one-based attempt `attempt` failed; None when the call has had all its retries
}
pub enum RetryPolicyError { BaseAboveMax { base: Duration, max: Duration }, Factor(f64) }

pub struct Pricing { input: f64, output: f64, cache_read: f64, cache_write: f64 }   // USD per million tokens
impl Pricing {
    pub fn new(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Result<Pricing, RateError>;
    pub fn rates(&self) -> Rates;                                     // what the run reports beside its cost
    pub fn cost(&self, usage: &Usage) -> Option<Cost>;   // None when the amount overflows an f64
}
```

**Stop policy.** The three methods hold the order and the boundaries §1 gives for each point. Each takes only what its point reads, so an input can't contradict itself: the two limit checks take the `Progress` the run produces (§3), and the token budget reads `Progress::usage.total()`, the billed tokens of every call so far (§1), while `after_response` takes the finish reason and a `Calls` and sees no limit at all. `Turn::calls(mode)` (§3) builds the `Calls`, and is the only thing that does, so the loop can't read a response differently from how the policy expects it: `TaskComplete` when the response calls `task_complete`, alone or among other tools, which only explicit mode offers; natural mode reads it as `Tools`. `Progress` lives in the model because the run produces it, and the policy depends on the model, never the reverse; the loop's call is `stop.before_call(&run.progress(elapsed))`. The policy decides nine of the stop reasons, `context_exhausted` among them when a response was cut short at the context window. The rest aren't its to decide: `cancelled` comes from the loop's poll of the `Cancellation` port, and `retries_exhausted`, `provider_error`, and `context_exhausted` for a rejected request from a failed provider call (`RetryPolicy::delay` returning `None`, and the `ProviderError` variant). `allows_wait` is here because stopping with `timeout` instead of sleeping is a reached-the-limit decision like the others. Every `StopPolicy` value is meaningful, so its fields are public and nothing is validated. The two counted caps are `NonZeroU32`, since a cap of zero has no reading other than one and two fields shouldn't reach that by two different routes; a zero `timeout` or token budget does stop the run at the first point A, which is a reading of its own.

**Retry policy.** `max_retries` is `run.max_retries`, and 0 is valid: it never retries. `new` refuses a `base` longer than `max` and a `factor` that isn't a finite number of at least 1. `delay(n)` is a wait when `n` is at most `max_retries` and `None` after that. It's total: attempt 0 is read as 1, and an attempt number or a factor large enough to overflow gives `max`, or zero when `base` is zero, never a panic or a NaN.

**Pricing.** `new` refuses a rate that isn't a finite number of at least 0. `cost` prices `Usage::uncached_input_tokens()` at the input rate and each cache field once, at its own rate, because `input_tokens` already includes both (§3); pricing `input_tokens` whole and adding the cache fields would bill the cached tokens twice. A cost is never negative and never NaN.

## 5. Application (`lablet-run`)

### Secondary ports

```rust
#[async_trait] pub trait ModelProvider: Send + Sync {
    fn model(&self) -> &ModelRef;
    fn endpoint(&self) -> Option<Endpoint>;                     // server.address and server.port; None for fake
    async fn complete(&self, req: ProviderRequest<'_>) -> Result<ProviderResponse, ProviderError>;
}
pub struct ProviderRequest<'a> { system: &'a str, messages: &'a [Message<'a>], tools: &'a [ToolSpec], max_tokens: u32, temperature: Option<f64>, thinking: Thinking, effort: Option<Effort>, seed: Option<u64>, deadline: Duration }   // messages is Run::messages(); Thinking and Effort are model types (§3), because RequestParams holds them
pub enum ProviderError { Retryable(String), ContextExhausted(String), Fatal(String), Malformed(String) }

#[async_trait] pub trait ToolExecutor: Send + Sync {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError>;
    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError>;
}
pub struct ToolCall { id: ToolCallId, name: ToolName, input: serde_json::Value, deadline: Duration, trace_context: Option<TraceContext> }
pub struct ToolOutput { content: Vec<ToolResultContent>, is_error: bool, mcp: Option<McpCallMeta> }   // content is text (§3); is_error is the tool's own report, as MCP's isError
pub struct ToolError { kind: ToolErrorKind, message: String, mcp: Option<McpCallMeta> }   // every kind becomes an error result for the model
pub enum ToolErrorKind { Unknown, Timeout, Failed }                  // Unknown is ToolCallStatus::Unknown; the other two name a ToolCallEnd, and the loop pairs it with the source the executor resolved

// Port and telemetry data. These are types of this crate, not of the model: nothing in the domain reads them.
pub struct TraceContext { traceparent: String, tracestate: Option<String> }   // W3C strings; no OpenTelemetry types below the adapters
pub struct McpCallMeta { method: String, session_id: Option<String>, protocol_version: Option<String>, jsonrpc_request_id: Option<String>, rpc_status_code: Option<String>, transport: NetworkTransport }
pub enum NetworkTransport { Pipe, Tcp }                              // stdio and HTTP; as_str gives the values of `network.transport`

#[async_trait] pub trait RunObserver: Send + Sync {
    async fn on(&self, event: RunEvent);
    fn trace_context(&self, call_id: &ToolCallId) -> Option<TraceContext> { None }   // the OTel observer answers with the tool span it opened on ToolCallStarted
}
pub enum RunEvent {                       // every variant carries run_id
    RunStarted { context: RunContext, model, endpoint: Option<Endpoint>, tools: Vec<ToolSpec>, system_prompt: Option<String>, prompt: Option<String> },
    TurnStarted { turn },
    ProviderCallStarted { turn, attempt, request_bytes: u64 },
    ProviderCallFinished { turn, attempt, record: TurnRecord, response: Option<Vec<ContentBlock>> },   // the turn as the transcript stored it
    ProviderCallFailed { turn, attempt, error, will_retry, backoff },
    ToolCallStarted { turn, call_id, name, source: Option<ToolSource>, input_bytes, input: Option<Value> },   // source is None for a name no tool has
    ToolCallFinished { turn, call_id, status: ToolCallStatus, latency_ms: u64, output_bytes: u64, truncated_from_bytes: Option<u64>, mcp: Option<McpCallMeta>, output: Option<Vec<ToolResultContent>> },   // read from the call's ToolCallOutcome
    RunFinished { context: RunContext, summary: RunSummary },
}

#[async_trait] pub trait Clock: Send + Sync { fn now(&self) -> Instant; async fn sleep(&self, d: Duration); }
pub trait Cancellation: Send + Sync { fn is_cancelled(&self) -> bool; }
```

The loop emits `ToolCallStarted`, then asks the observer for a `TraceContext` for that call id and places it on the `ToolCall`, so the MCP adapter can inject it into `params._meta`. The fan-out observer (kept so library users can register their own `RunObserver` beside the built-in one) returns the first `Some`; observers without spans return `None`. The OTel observer keeps open spans in a map keyed by call id, starts every child with an explicit parent context rather than the task-local current context (since `on` runs on arbitrary tasks), and uses an always-on sampler so the propagated flags are `01`; the span context is fixed at start and readable synchronously, and batch processors only see ended spans, so the handoff needs no coordination with export. `error.type` is defined per span: on the root it's the stop reason when the run didn't complete, on a chat span the `ProviderError` variant name (`retryable`, `context_exhausted`, `fatal`, `malformed`), and on a tool span the call's `ToolCallStatus` (§3) whenever it isn't `ok`: `tool_error`, the value the MCP conventions give, when the tool ran and returned an error result, and `unknown`, `timeout`, or `failed` when the executor did. The loop settles the status once for each call: `Ok(output)` is `ok`, or `tool_error` when `output.is_error`, and `Err(error)` is the namesake of `error.kind`. The same span carries `lablet.tool.output.truncated` and, when the output cap cut the output, `lablet.tool.output.original_bytes`; `lablet.tool.output.bytes` is the size the model was sent.

`ToolCallStarted.source` is `None` when the model calls a well-formed name that no tool has: there's no spec to read a source from. The `execute_tool` span of such a call carries neither `lablet.tool.source` nor `gen_ai.tool.type`, which the registry requires only for a tool the run offered, as it already does for `gen_ai.tool.description`; the span's `error.type` is `unknown`. Neither enum gains a member that names no source.

`request_bytes` is measured by the loop, not the adapter, so it's the same for every provider: the byte length of the system prompt, plus the length of each message in the request and of each tool spec as compact JSON in the model's serde form. The loop measures the system prompt and the tool specs once per run and each message once, the first time `Run::messages` renders it whole, which for a user message is when the results it holds are in, and keeps a running sum and a count of the messages it has measured, so a provider call costs the serialisation of what the last turn added, not of the conversation. It's a measure of context growth, not of the wire body.

Every per-step event carries `turn`, which observers emit as `lablet.turn` on the corresponding span. There is no turn span; the trace is `invoke_agent` with `chat` and `execute_tool` children directly beneath it, as the conventions describe, and turns are recovered by filtering on the index. A turn span can be added under the root later without changing anything else.

Content-bearing fields on events (`system_prompt`, `prompt`, `response`, `input`, `output`) are `None` unless `telemetry.capture_content` is on. The loop decides, so observers never see content they shouldn't. When content is off the attribute is omitted, not written as a placeholder; the byte-count attributes are always present. The intercepted `task_complete` argument appears in the outcome and transcript regardless, and in telemetry only when content is captured.

Observers never fail the run and never block it: `on` must return promptly, exporters buffer and batch, and export failures (an unreachable OTLP endpoint, an unwritable file) go to lablet's diagnostic log. The composition root flushes every observer before exit and reports flush failures on stderr without changing the exit code.

The `Clock` port is the only source of time for the loop, so `lablet-run` tests drive timeouts and backoff with a fake clock. Adapters enforce their own per-call deadlines with real time.

### Use case

```rust
pub struct RunService { provider: Arc<dyn ModelProvider>, tools: Arc<dyn ToolExecutor>, observer: Arc<dyn RunObserver>, clock: Arc<dyn Clock>, cancel: Arc<dyn Cancellation>, stop: StopPolicy, retry: RetryPolicy, request: RequestParams, pricing: Option<Pricing>, calls: CallLimits }
pub struct CallLimits { provider_timeout: Duration, tool_timeout: Duration, max_tool_output_bytes: Option<u64> }   // run.provider_timeout, run.tool_timeout, tools.max_output_bytes
impl RunService { pub async fn run(&self, context: RunContext, system: String, prompt: String) -> FinishedRun }
```

`run` never returns `Err`: every failure is a `RunOutcome` with a stop reason, so the caller always gets telemetry-consistent output. It returns one value, the model's `FinishedRun` (§3), which `Run::finish` builds: the `RunSummary`, whose `outcome` is the outcome document, and the `Transcript`, which is how the conversation leaves the loop. When `pricing` is set, the loop fills `RunSummary.cost` with `Pricing::cost` of the run's usage; otherwise it's `None`. The limits and request parameters in the summary come from the service's own `StopPolicy` and `RequestParams`, so they can't differ from what the run enforced. `CallLimits` holds what bounds one call and isn't a stop decision: the two per-call timeouts, which become the `deadline` of a request or a tool call, and the tool output cap, which the loop hands to `ToolCallOutcome::measured` for every call, so no executor cuts output itself.

The loop body, in the model's and the policy's terms. Every offset and latency is a `Duration` read from the `Clock`; the model turns them into milliseconds.

```
run = Run::start(RunSetup { run_id, model, endpoint, tools, completion, max_turns, timeout, request }, system, prompt)
loop:
    if cancelled: stop cancelled;  if let Some(r) = stop.before_call(&run.progress(elapsed)): stop r
    attempt = 1
    response = loop:                                                       # one provider call
        result = { messages = run.messages(); provider.complete(ProviderRequest { system: run.transcript().system(), &messages, .. }) }
        Ok(r)  -> break r
        Err(e) -> run.failed_attempt(latency); ContextExhausted -> stop context_exhausted, e; Fatal -> stop provider_error, e
                  retry.delay(attempt): Some(wait) if stop.allows_wait(elapsed, wait) -> sleep(wait), attempt += 1
                                        Some(_) -> stop timeout;  None -> stop retries_exhausted, e
    turn = run.responded(response, started, latency)?                      # the transcript's new turn; it takes the waiting input, and its record counts the attempts
    calls = turn.tool_uses().cloned();  structured = the input of the call named task_complete, if any
    if let Some(r) = stop.after_response(&turn.record().finish, turn.calls(stop.completion)): stop r
    outcomes = for call in calls:
        (status, content) = match tools.execute(ToolCall { id, name, input, deadline: tool_timeout, trace_context }):
            Ok(out) -> (ToolCallStatus::ran(source of call.name, if out.is_error { ToolError } else { Ok }), out.content)
            Err(e)  -> (Unknown for ToolErrorKind::Unknown, else ToolCallStatus::ran(source of call.name, e.kind), [Text(e.message)])
        ToolCallOutcome::measured(call.id, status, content, max_tool_output_bytes, started, latency)
    run.tool_calls(outcomes)?
    if cancelled: stop cancelled;  if let Some(r) = stop.after_tools(&run.progress(elapsed)): stop r
cost = pricing.and_then(|p| p.cost(&run.usage()))                          # None when there's no pricing, or the amount overflows
return run.finish(reason, elapsed, structured, error, cost)                 # FinishedRun { summary, transcript }
```

The messages are rendered inside the attempt because they borrow the run, which a failed attempt changes; rendering is one allocation and no copy of the conversation. The loop passes `finish` whatever `structured` and `error` it has, and the outcome keeps what the stop reason allows (§3), so the loop has no rule of its own about which stop reason carries which. The two calls marked `?` refuse only a sequence this loop can't produce: a response while the last turn's tool calls are unanswered or when it made none, which point R never lets through, and outcomes that aren't one for each call of the last turn, in order. `lablet-run`'s tests hold that neither is ever refused; if one were, `run` would report the defect on the diagnostic log and end the run with `provider_error` and the refusal's text, because a `FinishedRun` must still come back.

One `RunService` executes one run at a time; concurrent calls are a programming error and are rejected.

`ToolSet` is a composite `ToolExecutor` in this crate that routes by name across several executors, applies the config allow and deny lists, and rejects duplicate names at build time. The `task_complete` tool spec is defined in this crate, registered by `ToolSet` only in `explicit` mode regardless of `tools.builtin.enabled`, and never executed: the loop intercepts it. Its input schema is `run.completion_schema` when set, otherwise a free-form object. `ToolSet` is where "what happens when a tool is missing" is expressed: the tool is simply absent from `specs()`.

A `ToolExecutor` resolves only names it offered in `specs()`, and `ToolSet` fixes that set when the run is built, so no tool appears to a run part-way through. This is the port's obligation rather than a rule the model can hold, because a `ToolCallStatus` of `Ran` carries the source the executor resolved and the run believes it (§3). It's what keeps the per-tool attribute keys bounded, and it's the rule to hold an MCP executor to at phase 8, where a server may announce `notifications/tools/list_changed` mid-run: the new tool belongs to the next run, not this one.

Tool calls within one turn are executed sequentially. Parallel execution is a later option; the observer events already carry enough to distinguish it.

## 6. Adapters

### Finish reasons

Both provider adapters hand the provider's own string to `FinishReason::from` (§3), which holds this mapping, so neither adapter keeps a table of its own and a fake script may use either spelling:

| `FinishReason`  | Anthropic `stop_reason`         | OpenAI-compatible `finish_reason` | Stop policy at point R    |
| --------------- | ------------------------------- | --------------------------------- | ------------------------- |
| `EndTurn`       | `end_turn`, `stop_sequence`     | `stop`                            | decided by the tool calls |
| `ToolUse`       | `tool_use`                      | `tool_calls`                      | decided by the tool calls |
| `MaxTokens`     | `max_tokens`                    | `length`                          | `output_truncated`        |
| `ContextWindow` | `model_context_window_exceeded` | none                              | `context_exhausted`       |
| `Refusal`       | `refusal`                       | `content_filter`                  | `refused`                 |
| `Other(string)` | anything else, as `pause_turn`  | anything else, as `function_call` | decided by the tool calls |

### `provider-anthropic`

Messages API via `reqwest` with `rustls`. Maps `ContentBlock` both ways, including `thinking`, `redacted_thinking`, and cache-control (applied to the system prompt and tool specs when `model.cache: true`). Reads `cache_read_input_tokens` and `cache_creation_input_tokens` into `Usage`; the latter is reported as `gen_ai.usage.cache_write.input_tokens`, the semantic-convention name, not Anthropic's. Anthropic's own `input_tokens` leaves both out, so the adapter builds the `Usage` with `Usage::from_uncached`, which adds them in: `Usage.input_tokens` includes cached tokens (§3). Thinking: `ProviderDefault` sends nothing, `Adaptive` sends `thinking: {type: adaptive}`, `Budget(n)` sends `thinking: {type: enabled, budget_tokens: n}`, `Disabled` sends `type: disabled`; `effort` is a separate request parameter that maps to `output_config.effort` and is reported as `gen_ai.request.reasoning.level`. `temperature` is sent only when set; the build step warns that models from Opus 4.7 and Sonnet 5 onward reject a non-default value. Validates that a `Budget` is below `max_tokens` at build. Classifies 429, 529, 5xx, transport errors, and per-call timeouts as retryable, the `prompt is too long` invalid-request error as context exhausted, and 401 and 403 as fatal. Ignores `seed`.

### `provider-openai`

`/v1/chat/completions` with function calling. Covers OpenAI, Ollama, vLLM, and gateways via `base_url`. Sends `max_completion_tokens` and falls back to `max_tokens` if the server rejects it. One user message holding N `ToolResult` blocks becomes N `role: tool` messages on the way out. `Thinking` blocks are dropped on the way out; `reasoning_content` and `reasoning` fields are mapped to `Thinking` with no signature on the way in. Builds its `Usage` with `Usage::from_inclusive`, since `prompt_tokens` already includes the cached tokens. Ignores `thinking` and `effort`. Function `arguments` that aren't valid JSON are `Malformed`. Unknown fields go to `Opaque`. Passes `seed` when set. Classifies `context_length_exceeded` and equivalents as context exhausted. `api_key_env` is optional; no header is sent when it's unset.

### `provider-fake`

Plays a scripted sequence of `ProviderResponse`s from a YAML or JSON file (the domain model's serde form), with optional per-call latency (real time) and injected errors of each `ProviderError` class. Selected with `model.provider: fake` and `model.script: path`. Reports usage from the script so token accounting is exercised end to end. Used by lablet's smoke tests, doctests, and examples, and by users testing their own frameworks. `lablet init --provider fake` writes both a config and a script.

### `tools-builtin`

Each tool is a small struct; the executor holds only those enabled in config. Initial set: `bash` (working directory and timeout from config, captures stdout, stderr, exit code), `read_file`, `write_file`. All paths are resolved under `tools.builtin.root` and rejected if they escape it. `task_complete` isn't here; see §5.

Both executors report how a call ended in the port's terms, and the loop turns that into the call's `ToolCallStatus` (§5): a tool that ran and reports a failure of its own, such as a path outside the root or a file that can't be read, returns `ToolOutput` with `is_error: true` (`tool_error`); a name the executor doesn't have is `ToolErrorKind::Unknown`; a call that outlives its `deadline` is `Timeout`; and an executor that couldn't run the tool at all is `Failed`. Neither executor shortens output. Each returns what the tool produced, and the loop applies `tools.max_output_bytes` to every result the same way (§3).

### `tools-mcp`

One `rmcp` client per configured server (version pinned in the workspace `Cargo.toml`). Tool names are exposed exactly as the server reports them, so measurements reflect the server as-is. A reported name, or a prefixed one, that isn't a valid `ToolName` (1 to 64 of `[a-zA-Z0-9_-]`, which is what both provider APIs accept) is a build error with the `mcp:` prefix naming the server and the tool, since no provider would accept it. A name collision across servers is a build error; a server with `prefix_tools: true` has its tools renamed `<server>__<tool>`, which is the escape hatch. `execute` forwards the call and maps `isError` results to `is_error: true`, which the loop records as the status `tool_error`; a JSON-RPC error, a broken transport, or a dead server is a `ToolError` of kind `Failed`, a call that outlives its `deadline` one of kind `Timeout`. Tool results are text (§3): a `text` item is carried unchanged and a text resource as its text, and an `image`, `audio`, or binary resource item becomes the one line `ToolResultContent::omitted` renders from its kind, MIME type, and decoded byte length, for example `[image omitted: image/png, 48213 bytes]`. When a result carries `structuredContent`, the MCP specification has the server repeat it as JSON text in `content`, so the adapter sends `content` alone and the model doesn't see the same data twice; only when `content` is empty is the structured value sent, as one item of compact JSON text. It injects the `ToolCall`'s `trace_context` into the request's `params._meta` as unprefixed `traceparent` and `tracestate`, per MCP SEP-414, and fills `lablet-run`'s `McpCallMeta` (§5: method, session id, protocol version, JSON-RPC request id, RPC status code on error, transport `pipe` for stdio or `tcp` for HTTP) on the output or error so the OTel observer can put `mcp.*`, `jsonrpc.*`, `rpc.*`, and `network.transport` on the `execute_tool` span; the conventions want one span carrying both `gen_ai.tool.*` and `mcp.*`, not a nested MCP span. Servers are started when the `Lablet` is built, with `tools.mcp[].startup_timeout`, live for the lifetime of the `Lablet` across runs, and are shut down by `Lablet::shutdown`. A stdio server's stderr is forwarded line by line to the diagnostic log at `debug`. HTTP servers receive `headers` verbatim. If a server exits or its transport breaks mid-run, every call to its tools returns a tool error naming the server, so the consecutive error cap ends the run; its tools stay listed so the model's behaviour is observable.

### Telemetry contract (`lablet/telemetry/`)

Telemetry is contract-first. An OpenTelemetry Weaver registry under `lablet/telemetry/registry/` in the v2 syntax (`file_format: definition/2`) declares every attribute, span, and log record lablet emits. The approach follows `product/research/weaver/`, which verified it against Weaver v0.26.1:

- `manifest.yaml` (the `registry_manifest.yaml` name is deprecated) depends on the core semantic conventions (`v1.44.0`) and `semantic-conventions-genai` (pinned commit) through **relative paths** to `model/` trees vendored under `lablet/telemetry/deps/` by `cargo xtask weaver vendor`, each with a `SOURCES` file recording the repository, the ref, the commit and its date, and the paths copied. The paths are relative to the repository root, because Weaver resolves them against its working directory and `cargo xtask weaver` runs it there. Weaver has no dependency cache and clones every git dependency on every run, including the GenAI registry's own dependency on the core conventions, so the committed manifest never uses git URLs and the vendoring script points that one line of the vendored GenAI manifest at the vendored core tree. Every other vendored file is byte-identical to its source. The shared naming and stability policies and the Markdown doc templates from `opentelemetry-weaver-packages` are vendored the same way, without the test fixtures that sit beside them. The pinned commits live in `lablet/telemetry/vendor.sh` alone; `cargo xtask weaver vendor --check` fetches them again and fails on any difference, and a weekly CI job runs it. The gates need no network.
- The registry declares `lablet.*` attributes, each with a `note` beginning `Justification:`, and **lablet-owned** spans (`lablet.invoke_agent`, `lablet.chat`, `lablet.execute_tool`) and events (`lablet.run`, the wide event, and `lablet.retry`) that `ref` the `gen_ai.*`, `mcp.*`, `jsonrpc.*`, `rpc.*`, `network.*`, `server.*`, `session.*`, and `error.type` keys with explicit requirement levels. It doesn't refine or import the GenAI events, which core semconv's deprecated copies make unreachable. The provider-failure record and the content record are therefore declared in lablet's registry under the GenAI event names they're emitted with, `gen_ai.client.operation.exception` and `gen_ai.client.inference.operation.details`, so a consumer that knows the conventions finds them and live-check matches them by name. Resource entities are imported (`service`, `telemetry.sdk`).
- `cargo xtask weaver check` runs `weaver registry check --v2` with the lablet policies (Rego, after resolution) and the vendored naming and stability policies; the justification policy fails any `lablet.*` attribute without a note. The policy allows Development stability for imported `gen_ai.*`. It runs in pre-commit. Diagnostics render through lablet's templates under `lablet/telemetry/templates/diagnostics/`, as text or, when `CI=true`, as GitHub workflow commands: Weaver's own GitHub format prints policy violations only, so a reference that doesn't resolve would fail the check in silence, and its text format buries a finding under one warning for every v2 file.
- `cargo xtask weaver generate` runs `weaver registry generate --v2` with the MiniJinja templates under `lablet/telemetry/templates/registry/rust/` (filters are jq, not JMESPath), starting from the spike's templates, producing the sources of the `lablet-telemetry-registry` crate: a name constant for every attribute on a lablet signal or resource, an enum for each of lablet's closed value sets (the semantic-convention enums are open, so they get none), and for each span and event the list of its required keys and of all its keys. No span or event builders. It formats them with rustfmt and renders the vendored Markdown templates into `lablet/docs/telemetry/`. Both are rendered under `lablet/target/` and only then moved into place. `cargo xtask weaver generate --check` renders the same way and fails, naming the files, when the tree differs; it runs in pre-commit after `weaver check`.
- `cargo xtask weaver live-check` starts `weaver registry live-check` **without `--v2`** (the v2 index ignores dependency attributes and reports every `gen_ai.*` sample as missing; Weaver issue 1456) on a random free port pair, runs the fake-provider config against it over OTLP gRPC, stops it through the admin endpoint, and fails on violations. Live-check validates attributes (presence, type, enum) and matches log records to events by name; it doesn't match spans to definitions, so span names and required span attributes are held by unit tests in `telemetry-otel` against the generated key lists.
- Upgrading Weaver or bumping the GenAI commit is its own PR; `weaver registry diff` lists renamed `gen_ai.*` attributes and the generated-crate diff makes each one visible.

The tables in this document are the human summary; the registry is the source of truth. When they disagree, the registry wins and this document is corrected. Renames upstream in `gen_ai.*` are tracked as breaking changes to lablet's telemetry contract. Fallback if Weaver becomes unusable: the registry YAML and policies stay, codegen becomes a small xtask step over the committed resolved JSON, and live-check becomes an in-process observer test against the generated key lists.

### `telemetry-otel`

The only telemetry observer. It maps `RunEvent`s to OTel spans and log records exactly once, through `opentelemetry` and `opentelemetry_sdk`, and hands them to the SDK's pluggable `SpanExporter` and `LogExporter` implementations selected by config; there is no second rendering of the contract anywhere. Exporters: **OTLP network** via `opentelemetry-otlp` (gRPC via tonic or HTTP/protobuf, both on `rustls`), and the **OTLP/JSON file** exporter below. Both may be active at once. Batch span and log processors keep export off the loop's path. Every attribute name comes from `lablet-telemetry-registry`; string literals for attribute names are a lint failure in this crate. Unit tests assert each span's name and required attribute set against the generated per-signal key lists, since live-check doesn't. Follows the GenAI semantic conventions; anything the conventions lack uses the `lablet.` namespace. The wide event is a log record emitted through the logs API with the root span's trace context set on it. Shutdown runs from a blocking task with the export timeout lowered so an unreachable endpoint costs seconds, not the default ten.

| Event                   | Span                                                                                                                                        | Key attributes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| RunStarted..RunFinished | `invoke_agent lablet` (root, INTERNAL)                                                                                                      | `gen_ai.operation.name=invoke_agent`, `gen_ai.agent.name=lablet`, `gen_ai.agent.version`, `gen_ai.request.model`, the join keys (`gen_ai.conversation.id` and `session.id`, both the run id, and `lablet.config.digest`), `lablet.run.stop_reason`, `error.type` on failure, `lablet.run.turns`, `lablet.tool_calls.total`, aggregate `gen_ai.usage.*` (the conventions allow the aggregate on `invoke_agent`; a query summing `gen_ai.usage.*` over every span in a trace double counts and must filter on `gen_ai.operation.name`), `lablet.run.cost_usd` when pricing configured                                       |
| ProviderCall*           | `chat <model>` child of root (CLIENT)                                                                                                       | `gen_ai.operation.name=chat`, the join keys, `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.request.max_tokens`, `gen_ai.request.temperature` when set, `gen_ai.request.seed` when set, `gen_ai.request.reasoning.level` when set, `gen_ai.response.model`, `gen_ai.response.id`, `gen_ai.response.finish_reasons`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_write.input_tokens`, `server.address`, `server.port`, `error.type` on failure, `lablet.turn`, `lablet.attempt`, `lablet.request.bytes`                                |
| ToolCall*               | `execute_tool <name>` child of root (INTERNAL)                                                                                              | `gen_ai.operation.name=execute_tool`, the join keys, `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.tool.type` (`function` for builtin, `extension` for MCP), `gen_ai.tool.description`, `error.type` when the status isn't `ok`, `lablet.turn`, `lablet.tool.status`, `lablet.tool.source`, `lablet.tool.input.bytes`, `lablet.tool.output.bytes`, `lablet.tool.output.truncated`, `lablet.tool.output.original_bytes` when truncated, `lablet.tool.is_error`; for MCP tools also `mcp.method.name`, `mcp.session.id`, `mcp.protocol.version`, `jsonrpc.request.id`, `rpc.response.status_code`, `network.transport` |
| ProviderCallFailed      | `gen_ai.client.operation.exception` log record (severity WARN) in the chat span's context, plus a span event on the chat span for the retry | log record: `exception.type` (the `ProviderError` variant name, as `error.type` on the span), `exception.message`, the join keys, `gen_ai.operation.name`, `gen_ai.provider.name`, `gen_ai.request.model`, `lablet.turn`, `lablet.attempt`; span event `lablet.retry`: `lablet.attempt`, `lablet.retry.will_retry`, `lablet.retry.backoff_ms` when it will. The deprecated `exception` span event isn't used                                                                                                                                                                                                              |
| RunFinished             | one log record `lablet.run` (the wide event), trace context of the root span                                                                | the §1 wide-event attribute set                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |

Content, when captured, is emitted as `gen_ai.client.inference.operation.details` log records in the context of the span the content belongs to. Each carries the join keys, `gen_ai.operation.name`, and `lablet.turn`; a record for a chat span carries `gen_ai.system_instructions`, `gen_ai.input.messages`, and `gen_ai.output.messages`, and a record for a tool span carries `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.tool.call.arguments`, and `gen_ai.tool.call.result`. Content never appears on a span. Resource attributes: `service.name=lablet`, `service.version`, plus `telemetry.resource` from config. The exporter is flushed before the process exits.

### OTLP/JSON file exporter (in `telemetry-otel`)

A `SpanExporter` and `LogExporter` pair that writes each export batch as one line of OTLP/JSON, the protobuf JSON mapping of `ExportTraceServiceRequest` and `ExportLogsServiceRequest`, to `lablet-<run_id>.otlp.jsonl` in the working directory by default, any path, or stderr with `-`. This is the format the OpenTelemetry Collector's file exporter writes and its OTLP JSON file receiver reads, so a file written without a collector can be replayed into one later, or into any tool that speaks OTLP. Each line carries the full resource and instrumentation scope, so a file is byte-for-byte the data a collector would have received: spans with start and end times, status, events, and attributes; log records including the wide event and, when captured, the content records. No lablet-specific envelope exists; the registry describes the attributes and OTLP describes the structure. Always available, no endpoint needed, and the default when no OTLP endpoint is configured. Flattening is the consumer's first step, which DuckDB, DataFusion, jq, and the collector all do directly.

Lifecycle: the providers and exporters are built once per `Lablet`, before any run id exists. On `RunStarted` the observer hands the file exporter the run's path (the `lablet-<run_id>` default); a fixed `telemetry.file.path` receives every run of that `Lablet`, appended. After emitting the wide event, `Lablet::run` calls `force_flush` on the tracer and logger providers before returning, so the file is complete when `run` returns and a caller may read it immediately. Serialisation uses `opentelemetry-proto` with the `with-serde` feature and the same `group_spans_by_resource_and_scope` and `group_logs_by_resource_and_scope` transforms the OTLP HTTP/JSON path uses, written compact (`serde_json::to_string`, one request per line, newline terminated), which yields hex ids, stringified 64-bit integers, and camelCase names as the OTLP/JSON mapping requires. Readers dispatch on the top-level `resourceSpans` or `resourceLogs` key, as the Collector does, because the prost serde derives don't reject unknown fields.

## 7. Composition root (`apps/lablet`, package `lablet`)

Library and binary in one package. `lib.rs` exposes:

```rust
pub struct Config { ... }                              // serde, YAML or JSON, deny_unknown_fields
impl Config { pub fn from_path(p) -> Result<Config, ConfigError>; pub fn from_str(s, Format) -> ...; pub fn resolved(&self) -> ResolvedConfig; pub fn digest(&self) -> String; }
pub struct Lablet { service: RunService, mcp: Vec<McpHandle>, observers: Vec<Arc<dyn RunObserver>>, context_template: RunContext }
pub async fn build(config: Config) -> Result<Lablet, BuildError>;
impl Lablet { pub async fn run(&self, prompt: &str) -> RunOutcome; pub async fn shutdown(self); }
```

A `Lablet` may run many times; each `run` gets a fresh `RunId` and `RunContext`, MCP servers persist across runs, and `shutdown` is the only teardown. `run` takes the `FinishedRun` that `RunService::run` returns, writes its transcript to `run.transcript_path` when that's set, and returns its outcome. Runs on one `Lablet` are sequential.

While the build is in progress, a config that selects an adapter whose crate doesn't exist yet makes `build` return `BuildError::Unsupported { kind, phase }` naming the build-plan phase that delivers it; config validation, including the `api_key_env` check, runs before adapter selection and doesn't depend on the adapter. Two outcomes are "identical" when they're equal after removing `run_id` and `duration_ms`.

`main.rs` only does effects: parse args with `clap` derive, install a `tracing` subscriber on stderr filtered by `RUST_LOG` (default `warn`) for lablet's own diagnostics, load config, install Ctrl-C handling into the `Cancellation` port, `build`, `run` (which writes the transcript), print outcome, flush telemetry, print one human-readable summary line on stderr (stop reason, turns, tokens, tool calls, duration; suppressed by `--quiet`), exit. For a `max_total_tokens` stop the line gives both numbers and says what they count, as `token budget reached: 100,412 of 100,000 billed`. Diagnostics and telemetry are separate: the diagnostic log is about lablet, the telemetry is about the run. The outcome print is the one permitted `print_stdout`, marked with `#[expect]`.

```
lablet init [--provider anthropic|openai|fake] [path]   # write a working starter config (and a script for fake)
lablet run --config lablet.yaml [--prompt "..." | --prompt-file f | stdin] [--set key=value ...]
lablet check --config lablet.yaml [--resolved]          # validate, start MCP servers, list tools; --resolved prints the full config
lablet schema                                           # print the config JSON schema
```

`check` never contacts the model provider. It validates the config, checks that `api_key_env` names a set, non-empty variable when required, starts every MCP server and lists their tools, then exits. A provider rejecting the credentials is a startup failure of `run`, exit 1, before `RunStarted`. `run` with no prompt source reads stdin; an empty prompt is a config error.

Error messages name the config key, line, offending value, and accepted values, and distinguish config errors, MCP server startup failures, and provider rejections. Each class has a fixed prefix (`config:`, `mcp:`, `provider:`) so tests and scripts can match them.

`--set` applies dotted overrides to the raw config tree before deserialisation; values are parsed as YAML scalars. Environment variable substitution `${VAR}` is applied to string values after that; an unset variable is a config error. Durations use humantime syntax. No provenance tracking, retired-key roster, or inert-key warnings: unknown keys are errors and that's the whole config policy.

### Config

```yaml
run:
  completion: natural # natural | explicit
  max_turns: 30
  timeout: 10m
  max_total_tokens: null # billed tokens: input + output summed over every provider call, so context sent again counts again (ten turns growing to 20,000 tokens of context bill about 110,000); input already includes cached tokens; null means unlimited
  max_retries: 3 # retries per provider call, so four attempts; 0 never retries
  retry_backoff_base: 500ms # the wait after the first failed attempt; it doubles each time
  retry_backoff_max: 30s
  max_consecutive_tool_errors: 3
  tool_timeout: 60s
  provider_timeout: 120s
  transcript_path: null # write the full conversation at run end
  transcript_format: json # json | atif (ATIF v1.8, phase 10)
  completion_schema: null # explicit mode: JSON schema for the task_complete argument; null means any object

model:
  provider: anthropic # anthropic | openai | fake
  script: null # fake only: path to the scripted responses
  name: claude-sonnet-5
  api_key_env: ANTHROPIC_API_KEY # never the key itself; optional for openai (Ollama) and fake
  base_url: null # override for gateways, Ollama, vLLM
  max_tokens: 4096
  temperature: null # sent only when set; rejected by current Anthropic models
  thinking: provider_default # provider_default | adaptive | disabled | { budget: 2048 } (tokens, at least 1 and below max_tokens); anthropic only
  effort: null # low | medium | high | xhigh | max (anthropic)
  seed: null # passed through when the provider supports it
  cache: true # anthropic only
  pricing: null # { input, output, cache_read, cache_write } USD per million tokens

prompt:
  system: "You are ..." # exactly one of system or system_file
  system_file: null
  skills: [] # paths to SKILL.md files, appended to the system prompt in order

tools:
  builtin:
    root: . # sandbox root for file and bash tools
    enabled: [bash, read_file, write_file] # task_complete is implied by run.completion: explicit
  mcp:
    - name: docs
      transport: stdio
      command: npx
      args: ["-y", "@example/docs-mcp"]
      env: {}
      startup_timeout: 30s
      prefix_tools: false # true renames its tools <name>__<tool>
    - name: search
      transport: http
      url: http://localhost:8080/mcp
      headers: {}
  allow: null # list of tool names; null means all
  deny: [] # removed after allow is applied
  max_output_bytes: 100000 # a tool result longer than this is cut and ends with a line saying so; null means no cap

telemetry:
  capture_content: false
  otlp:
    endpoint: null # e.g. http://localhost:4317; null disables the network exporter and makes the file exporter the default
    protocol: grpc # grpc | http
    headers: {}
  file:
    path: null # OTLP/JSON lines; file path, "-" for stderr; null means lablet-<run_id>.otlp.jsonl when otlp.endpoint is also null
  resource: {} # extra resource attributes, e.g. experiment ids
```

Config digest (`lablet.config.digest`) is a SHA-256 of the canonical JSON form of the **resolved** config (defaults filled in) after `--set` overrides but before `${VAR}` substitution. Any value injected from the environment never enters the digest, two configs with the same effect share a digest, and a default change in a new lablet version changes the digest, which is why `gen_ai.agent.version` sits beside it in the wide event.

## 8. Versioning

- One workspace version, semver. The config schema, the outcome JSON, and the telemetry registry are the public contract; a breaking change to any of them is a major bump.
- `CHANGELOG.md` in keep-a-changelog format with an `Unreleased` section. `cargo xtask changelog` fails when `lablet/schema.json`, `lablet/telemetry/registry/`, or `lablet/tests/fixtures/outcome.json` differ from `main` and `Unreleased` has no entry. The base is the merge-base with `origin/main`; on `main` itself CI sets `LABLET_CHANGELOG_BASE` to the commit before the push. Uncommitted and untracked files count. "No entry" means the body of `## [Unreleased]` is unchanged since the base or has no list item. An unresolvable base warns and passes. CI checks out full history so the comparison works.
- Every third-party crate is pinned to an exact version in `[workspace.dependencies]`; a version bump is its own commit, never mixed with feature work. A member's manifest only inherits (`name.workspace = true`, with at most `features`, `optional`, or `default-features` beside it), so no version, path, git source, or rename exists outside that table, and each entry there has a comment on the line above it saying why. `xtask` is its own workspace and pins its own dependencies exactly; `lint-manifests` requires a crate pinned in both places to carry the same version. The lints refuse the Cargo features lablet doesn't use (member globs, `exclude`, a root package, `[patch]`, `[replace]`, cargo-config `patch`, `paths`, `source`) rather than model them.
- `rust-version` in the workspace `Cargo.toml` is the MSRV; policy is the pinned toolchain minus two minor versions, raised only in a minor release.

## 9. Testing

- Unit tests live beside the code in a sibling file declared `#[cfg(test)] mod tests;` (`src/foo.rs` with `src/foo/tests.rs`, or `src/tests.rs`), never as an inline `mod tests { ... }` body, so the coverage floor can exclude test code by file name instead of parsing Rust. The gate enforces this in the three floor crates (`lablet-model`, `lablet-policy`, `lablet-run`); elsewhere it's the convention. Policy and model crates are the bulk and need no async.
- `lablet-run` is tested end to end with hand-written fakes for every port, including a fake clock. No mocking framework.
- Provider adapters are tested against `wiremock` with recorded payloads, including error classification and content-block round trips.
- `lablet-conformance` holds case sets for `ToolExecutor` and `RunObserver`, pulled in as dev-dependencies by each adapter.
- `lablet-test-mcp-server` is an rmcp stdio binary with tools that echo, sleep for N seconds, return an image, return `structuredContent` beside its text, and make the server exit after N calls; cargo sets `CARGO_BIN_EXE_lablet-test-mcp-server` only for the server package's own tests, so how tests in other packages locate the binary is an open question (§10). A `--hang-startup` flag never completes initialisation.
- One integration test target per crate (`tests/it/main.rs`).
- A smoke test in `apps/lablet` runs a full loop with `provider-fake` and the OTLP/JSON file exporter and asserts on the spans and log records read back from the file. Cancellation scenarios drive the `Cancellation` port directly.

## 10. Open questions

Recorded here so they're not lost; none block the first build.

- Parallel tool execution within a turn.
- Interrupting a provider or tool call in flight on cancellation, rather than waiting for it.
- Loading skills through a tool rather than inlining.
- Streaming responses (not needed for measurement, may be needed for very long outputs).
- A `lablet.run` metric set alongside traces, once there is a consumer for it.
- Validating the `task_complete` argument against `run.completion_schema` rather than only advertising it.
- A `check --probe` flag that makes one minimal provider call.
- Which MCP server served a tool call isn't on the tool span. `ToolSource::Mcp` knows the server, and the wide event lists the servers, but no attribute joins a call to its server. The conventions have none, so it would be a `lablet.*` extension.
- How `lablet-tools-mcp`'s tests locate the `lablet-test-mcp-server` binary: cargo sets `CARGO_BIN_EXE_<name>` only for tests of the package that defines the binary, so a cross-package mechanism (a build through `escargot`, or hosting the MCP scenarios in the server's own package) must be chosen in phase 8. An artifact dependency isn't an option on the pinned stable toolchain: cargo rejects it without nightly `-Z bindeps`. Nor is a plain dev-dependency on the server package, which cargo ignores because the package has no library.
- A `turn` span under `invoke_agent`, if per-turn grouping in trace viewers proves worth an extra span level.
- The deferred `lablet.*` extensions in the research catalogue (working time, failed-attempt tokens, cache hit ratio, time split, event sequence).
- Exporting the transcript as an ATIF v1.8 trajectory, planned for phase 10.
- A schema version field on the outcome JSON and on the transcript document. Deferred until both schemas settle; until then a reader tells the forms apart by their keys.
- A Parquet exporter for developers without a collector: a third `SpanExporter` and `LogExporter` pair beside OTLP and the OTLP/JSON file, probably in the OTel-Arrow (OTAP) layout. Deferred because choosing the file layout is a contract decision; the OTLP/JSON file covers the lightweight case today.
