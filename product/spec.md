# Lablet specification

Status: draft for build. Companion to [brief.md](brief.md), [quality-bar.md](quality-bar.md), [build-plan.md](build-plan.md), and [acceptance.md](acceptance.md).

## 1. What a run is

A run takes a config and a task prompt and executes one agent loop:

```
turn = 1
messages = [user(prompt)]
loop:
    if cancelled or stop policy says stop (timeout, tokens): break          # point A
    response = provider.complete(system, messages, tool specs)              # retried per call on transient error
    messages += response.message
    if stop policy says stop (completed, ended_without_completion, output_truncated): break   # point R; a task_complete call is recorded here, never executed
    results = []
    for each tool_use block in response: results += tools.execute(call)     # errors become error results
    messages += user(results)                                                # all results of a turn in ONE user message
    if cancelled or stop policy says stop (tool errors, turns, timeout, tokens): break   # point B
    turn += 1
emit outcome
```

The stop policy is evaluated at three points: before each provider call (point A), after each provider response and before any tool runs (point R), and after each tool phase (point B). Cancellation, the run timeout, and the token budget are checked at A and B; the turn cap and the consecutive tool error cap at point B only. A run therefore never makes a provider call after the condition that should have stopped it. Point R reads only the response, so a response that finishes the task completes the run even when it also used up the timeout or the token budget.

When several conditions hold at one point, the first in this order is the stop reason, so one state always gives one reason:

| Point | Order                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A     | `cancelled`, `timeout`, `max_total_tokens`                                                                                                                                                                                                                                                                                                                                                                                                            |
| R     | explicit mode and `task_complete` called: `output_truncated` when the finish reason is `max_tokens`, because the call's arguments may be cut short, and `completed` otherwise, whatever else the response holds. Otherwise any tool call: the run goes on, whatever the finish reason. Otherwise finish reason `max_tokens`: `output_truncated`, in both modes. Otherwise `completed` in natural mode and `ended_without_completion` in explicit mode |
| B     | `cancelled`, `tool_errors_exhausted`, `max_turns`, `timeout`, `max_total_tokens`                                                                                                                                                                                                                                                                                                                                                                      |

Cancellation leads because the loop polls its port before it asks the policy, which doesn't see cancellation at all. The tool error cap comes next at B because it alone says the run was failing, not merely long.

Every limit is met when the run reaches it, not when it passes it. With `max_turns: 2` the run stops after the tool phase of turn 2, having made two provider calls; a response on turn 2 that finishes the task still completes the run, because point R comes first. The run stops with `timeout` once the elapsed time equals `run.timeout`, with `max_total_tokens` once input plus output tokens equal the budget, and with `tool_errors_exhausted` on the third consecutive error result when the cap is 3.

Every provider call, tool call, retry, and stop decision is reported to an observer, which is how telemetry leaves the process.

### Completion modes

| Mode                | Completes when                                                                                                                                                                                                                                | Ends without completing when                                                         |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `natural` (default) | The model returns a turn with no tool calls and a finish reason other than `max_tokens`.                                                                                                                                                      | Never. Other stop reasons still apply.                                               |
| `explicit`          | The model calls the built-in `task_complete` tool. The call is intercepted by the loop, never executed, not counted in `tool_calls`, and its JSON argument is the run's structured result. Other tool calls in the same response are ignored. | The model returns a turn with no tool calls. Stop reason `ended_without_completion`. |

### Stop reasons

`completed`, `ended_without_completion`, `max_turns`, `timeout`, `max_total_tokens`, `output_truncated` (finish reason `max_tokens` and no tool calls; with tool calls the tools run and the loop continues), `context_exhausted` (the provider rejected the request as too long), `retries_exhausted`, `tool_errors_exhausted`, `cancelled`, `provider_error` (any other non-retryable error).

### Retries

- **Provider errors** are classified by the adapter as retryable (transport failure, rate limit, 5xx, overloaded, per-call timeout), context exhausted (the provider's context-length error), or fatal (auth, bad request, unknown model). Retryable errors are retried with exponential backoff up to `run.max_retries` times **per provider call**; the counter resets on success. `max_retries` counts retries, not attempts: `3` allows four attempts, and `0` never retries. The loop builds its `RetryPolicy` with `max_attempts = max_retries + 1`. Attempts are numbered from 1, as `lablet.attempt` reports them. When attempt `n` fails and `n` is at most `run.max_retries`, the loop waits and tries again; otherwise the run stops with `retries_exhausted`. So with `max_retries: 3`, two failures and a success is a completed call with two retries, and four failures exhaust it. Before each wait the loop checks the run timeout, so backoff can't carry a run far past it: if the elapsed time plus the wait reaches `run.timeout`, the run stops with `timeout` instead of sleeping. The wait after attempt `n` is `base * factor^(n - 1)` capped at `max`, with no jitter, so the same attempt always waits the same time. `base` and `max` come from `run.retry_backoff_base` and `run.retry_backoff_max`; `factor` is 2.
- **Malformed provider responses** (the adapter can't map the payload to the domain model, including a tool call whose arguments aren't valid JSON, or whose name or id isn't a valid `ToolName` or `ToolCallId`) count as retryable. That differs from a well-formed name that no tool has, which is an error result for the model. An OpenAI-compatible server that sends an empty or repeated call id gets a synthesised unique one from the adapter, not a `Malformed`.
- **Tool errors** aren't retried by lablet. The error is returned to the model as an error tool result. A call to a tool name that's not in `specs()` is also an error result. After `run.max_consecutive_tool_errors` consecutive error results the run stops with `tool_errors_exhausted` at point B. A successful tool call resets the counter.
- **Tool timeouts** (`run.tool_timeout`) are tool errors.
- **Provider timeouts** (`run.provider_timeout`) are enforced per call by the adapter's HTTP client via the deadline on the request. The run-level `run.timeout` is checked at points A and B only, so a run may overrun it by at most one provider or tool call.

### Cancellation

Cancellation is polled at points A and B via the `Cancellation` port; the CLI wires it to Ctrl-C. A run in the middle of a long tool call stops after that call returns or times out. Interrupting a call in flight is an open question.

### Transcript

When `run.transcript_path` is set, the full message list (system prompt, every message, every tool result) is written there as JSON when the run ends, whatever the stop reason. The document is the serde form of `Transcript` (§3): `system`, `messages`, and `turns`, which holds the usage and response model of the completion behind each assistant message, in order. This is independent of telemetry and `capture_content`; it's how a grader lablet or an outer framework gets the conversation.

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

`result.text` is the concatenated `Text` blocks of the last assistant message, or `""` if there is none. `result.structured` is always present: the `task_complete` argument when the run completed in explicit mode, `null` otherwise, natural mode included. `error` is `null` or a string, and is likewise always present. `duration_ms` is a whole number of milliseconds. The document is the derived serde form of `RunOutcome` (§3), and `lablet/tests/fixtures/outcome.json` holds one example that a `lablet-model` test reads, compares with a value built in code, and writes back unchanged. `usage` totals are summed over every successful provider call; a failed attempt reports no usage (`ProviderError` carries none), and `lablet.provider.calls` counts successful completions while `lablet.provider.retries` counts failed attempts, so chat spans number `calls + retries`. `input_tokens` **includes** cached tokens, as the GenAI semantic conventions require; `cache_read_tokens` and `cache_write_tokens` are subsets of it, so consumers that want uncached input subtract them.

Exit codes: `0` completed, `2` ended with any other stop reason, `1` the run never started (config error, MCP server failed to start, provider rejected the credentials on the first call before any `RunStarted` event is emitted).

### Wide event

When a run ends, every observer emits exactly one wide event: a single record carrying everything worth knowing about the run, so an analyst can answer most questions from one row without joining spans. It's emitted after `RunFinished`, whatever the stop reason, and is the last thing the run produces. Its content is `RunContext` plus `RunSummary` (§3), flattened to the attribute names below.

| Group    | Attributes                                                                                                                                                                                                                                                                                                                                                                                   |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| identity | `gen_ai.conversation.id` and `session.id` (both the run id), `gen_ai.agent.name` (always `lablet`), `gen_ai.agent.version`, `lablet.config.digest`                                                                                                                                                                                                                                           |
| setup    | `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.request.max_tokens`, `gen_ai.request.seed`, `gen_ai.request.reasoning.level`, `lablet.run.completion_mode`, `lablet.run.max_turns`, `lablet.run.timeout_ms`, `lablet.tools.names` (string[]), `lablet.tools.count`, `lablet.mcp.servers` (string[]), `lablet.prompt.system_bytes`, `lablet.prompt.user_bytes`, `lablet.skills.count` |
| outcome  | `lablet.run.stop_reason`, `error.type`, `lablet.run.error`, `lablet.run.duration_ms`, `lablet.run.turns`, `lablet.result.text_bytes`, `lablet.result.has_structured`, `lablet.run.transcript_path`                                                                                                                                                                                           |
| provider | `lablet.provider.calls`, `lablet.provider.retries`, `lablet.provider.latency_ms.total`, `lablet.provider.latency_ms.max`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_write.input_tokens`, `gen_ai.response.finish_reasons` (string[], one per call), `lablet.run.cost_usd` (when pricing configured)             |
| tools    | `lablet.tool_calls.total`, `lablet.tool_calls.errors`, `lablet.tool_calls.latency_ms.total`, `lablet.tool_calls.input_bytes.total`, `lablet.tool_calls.output_bytes.total`, `lablet.tool.calls.<name>`, `lablet.tool.errors.<name>`, `lablet.tool.latency_ms.<name>` (templates, int)                                                                                                        |
| content  | `lablet.result.text` and `lablet.result.structured` (JSON string) only when `telemetry.capture_content` is on                                                                                                                                                                                                                                                                                |

Naming rule: a GenAI or core semantic-convention attribute is used wherever one exists. A `lablet.*` attribute is added only when the run can't be described without it, and every one carries a one-line justification in the registry. Per-tool values use Weaver `template[int]` attributes, which allow a dynamic suffix only; there are no map-typed attributes. `gen_ai.usage.input_tokens` includes cached tokens; `run.max_total_tokens` counts `input + output`. Extensions considered and deferred are listed in the research catalogue, not here.

The wide event is an OTel log record and reaches every configured exporter (OTLP over the network, the OTLP/JSON file) identically. Spans remain the per-step detail; the wide event is the per-run row.

Aggregatability rules: the wide event has a fixed flat shape (no nested maps; template attributes with a bounded key set for per-tool values); the join keys `gen_ai.conversation.id`, `session.id`, and `lablet.config.digest` appear on the wide event, on every span, and on every other log record so any consumer can group by them without a join; composer-supplied `telemetry.resource` attributes (task id, experiment id, and the like) live on the OTel Resource only, which travels with every export batch as OTLP defines, because their keys are chosen by the composer and can't be declared in the registry, and live-check reports undeclared span attributes as violations; only raw counts, bytes, tokens, and durations are emitted, never ratios or averages, since those belong to the aggregation.

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

Pure types with serde derives. `serde_json::Value` is the JSON data currency. Every field is public except the one inside an identifier or `Cost`, which a constructor guards.

```rust
// Identifiers: validated by `new` and, through serde `try_from`, on deserialisation. Each serialises as a bare string.
pub struct RunId(String);                 // non-empty, no surrounding whitespace; by convention a ULID, generated by the composition root
pub struct ToolCallId(String);            // non-empty, no surrounding whitespace; the provider's id for the call
pub struct ToolName(String);              // the above, and 1 to 64 of [a-zA-Z0-9_-], which is what both provider APIs accept
pub enum IdError { Empty { kind: &'static str }, SurroundingWhitespace { kind: &'static str, value: String }, ToolNameCharacter { value: String }, ToolNameTooLong { value: String } }

// Conversation
pub enum Role { User, Assistant }
pub struct Message { pub role: Role, pub content: Vec<ContentBlock> }
impl Message {
    pub fn new(role: Role, content: Vec<ContentBlock>) -> Result<Message, MessageError>;   // validates
    pub fn validate(&self) -> Result<(), MessageError>;                                    // first violation in block order
    pub fn text(&self) -> String;                                                          // the Text blocks, concatenated
    pub fn tool_result(&self, call_id: &ToolCallId) -> Option<&ContentBlock>;
}
pub enum MessageError { DuplicateToolUse { id: String }, DuplicateToolResult { id: String }, ToolUseFromUser { id: String }, ToolResultFromAssistant { id: String } }
pub enum ContentBlock {
    Text(String),
    Thinking { text: String, signature: String },                     // Anthropic: text may be empty (display omitted); signature always present
    RedactedThinking { data: String },                                // Anthropic: replayed unchanged
    ToolUse { id: ToolCallId, name: ToolName, input: serde_json::Value },
    ToolResult { call_id: ToolCallId, content: Vec<ToolResultContent>, is_error: bool },
    Opaque { provider: ProviderKind, payload: serde_json::Value },   // any other provider-specific block, replayed unchanged
}
pub enum ToolResultContent { Text(String), Json(serde_json::Value) }
pub struct Transcript { pub system: String, pub messages: Vec<Message>, pub turns: Vec<TurnRecord> }   // turns[n] belongs to the n-th assistant message
pub struct TurnRecord { pub usage: Usage, pub response_model: Option<String> }
pub struct Step<'a> { pub message: &'a Message, pub record: Option<&'a TurnRecord>, pub observation: Option<&'a Message> }
impl Transcript {
    pub fn new(system: String) -> Transcript;
    pub fn push_user(&mut self, content: Vec<ContentBlock>);
    pub fn push_assistant(&mut self, content: Vec<ContentBlock>, record: TurnRecord);
    pub fn steps(&self) -> Vec<Step<'_>>;                             // one per assistant message
}
impl Step<'_> { pub fn result_of(&self, call_id: &ToolCallId) -> Option<&ContentBlock>; }

// Provider
pub enum ProviderKind { Anthropic, Openai, Fake }
pub struct ModelRef { pub provider: ProviderKind, pub name: String }
pub struct Endpoint { pub host: String, pub port: u16 }
pub struct Usage { pub input_tokens: u64, pub output_tokens: u64, pub cache_read_tokens: u64, pub cache_write_tokens: u64 }
impl Usage { pub fn total(&self) -> u64; pub fn uncached_input_tokens(&self) -> u64; }   // and Add, AddAssign, Default; all saturating
pub struct Completion { pub message: Message, pub usage: Usage, pub finish: FinishReason, pub response_id: Option<String>, pub response_model: Option<String> }
pub enum FinishReason { EndTurn, ToolUse, MaxTokens, Other(String) }
pub struct Cost(f64);                     // USD; Cost::new(usd), usd()
pub struct RequestDefaults { pub max_tokens: u32, pub temperature: Option<f32>, pub thinking: Thinking, pub seed: Option<u64> }
pub struct Thinking { pub mode: ThinkingMode, pub budget: Option<u32>, pub effort: Option<Effort> }
pub enum ThinkingMode { Default, Enabled, Disabled }
pub enum Effort { Low, Medium, High, XHigh, Max }

// Tools
pub struct ToolSpec { pub name: ToolName, pub description: String, pub input_schema: serde_json::Value, pub source: ToolSource }
pub enum ToolSource { Builtin, Mcp { server: String } }
pub struct TraceContext { pub traceparent: String, pub tracestate: Option<String> }   // W3C strings; no OpenTelemetry types in the domain
pub struct McpCallMeta { pub method: String, pub session_id: Option<String>, pub protocol_version: Option<String>, pub jsonrpc_request_id: Option<String>, pub rpc_status_code: Option<String>, pub transport: NetworkTransport }
pub enum NetworkTransport { Pipe, Tcp }   // stdio and HTTP; the values of `network.transport`

// Run
pub enum CompletionMode { Natural, Explicit }
pub enum StopReason { Completed, EndedWithoutCompletion, MaxTurns, Timeout, MaxTotalTokens, OutputTruncated, ContextExhausted, RetriesExhausted, ToolErrorsExhausted, Cancelled, ProviderError }
pub struct RunOutcome { pub run_id: RunId, pub stop_reason: StopReason, pub turns: u32, pub usage: Usage, pub tool_calls: u64, pub duration_ms: u64, pub result: RunResult, pub error: Option<String> }
pub struct RunResult { pub text: String, pub structured: Option<serde_json::Value> }
pub struct RunContext { pub run_id: RunId, pub config_digest: String, pub agent_version: String, pub resource: Vec<(String, String)>, pub transcript_path: Option<PathBuf>, pub skills_count: u32, pub mcp_servers: Vec<String>, pub completion: CompletionMode, pub max_turns: u32, pub timeout_ms: u64, pub request: RequestDefaults, pub capture_content: bool }
pub struct ToolStats { pub calls: u64, pub errors: u64, pub latency_ms: u64 }   // one tool's share of the run; the `lablet.tool.*.<name>` templates
pub struct RunSummary { pub model: ModelRef, pub tools: Vec<ToolName>, pub prompt_system_bytes: u64, pub prompt_user_bytes: u64, pub provider_calls: u64, pub provider_retries: u64, pub provider_latency_total_ms: u64, pub provider_latency_max_ms: u64, pub finish_reasons: Vec<FinishReason>, pub tool_calls_errors: u64, pub tool_latency_total_ms: u64, pub tool_input_bytes: u64, pub tool_output_bytes: u64, pub per_tool: BTreeMap<ToolName, ToolStats>, pub cost: Option<Cost>, pub outcome: RunOutcome }
```

`RunContext` is composition-root knowledge handed to the loop. `RunSummary` is accumulated by the loop, not reconstructed by observers, so every observer reports identical numbers. The wide event is the two flattened together. `RunSummary` doesn't repeat what its `outcome` already holds: `gen_ai.usage.*` comes from `outcome.usage`, `lablet.tool_calls.total` from `outcome.tool_calls`, `lablet.run.turns` from `outcome.turns`, `lablet.run.duration_ms` from `outcome.duration_ms`, and `lablet.run.stop_reason` and `lablet.run.error` from the fields of the same name, so no two fields can disagree.

**Usage.** `input_tokens` includes the cached tokens; `cache_read_tokens` and `cache_write_tokens` are subsets of it. `total()` is `input_tokens + output_tokens` and is what `run.max_total_tokens` counts; `uncached_input_tokens()` is `input_tokens` less both cache fields. An adapter whose provider reports uncached input separately, as Anthropic does, adds the cache counts in before it builds a `Usage`. Cache-related fields are zero for providers that don't report them.

**Durations.** A duration that reaches telemetry as a `*_ms` attribute is stored as whole milliseconds in a `u64` field named `*_ms` (`duration_ms`, `timeout_ms`, the latency fields). The loop converts once, where it reads the clock, so no observer rounds for itself. The model holds no `Duration`.

**Serde form.** Derives only. Enums are externally tagged with `snake_case` names: a `ContentBlock` is `{"text": "..."}`, `{"tool_use": {"id": "...", "name": "...", "input": {}}}`, or `{"tool_result": {"call_id": "...", "content": [{"text": "..."}], "is_error": false}}`, and the other variants follow the same pattern; a `ToolSource` is `"builtin"` or `{"mcp": {"server": "docs"}}`. `FinishReason::Other` is untagged, so it's written as the provider's own string, and a string that spells a known reason always reads back as that reason. `Effort::XHigh` is `xhigh`. `Cost` is a bare number. An `Option` that's `None` is written as `null`, never left out. A `Usage` field left out of the input reads as zero. `StopReason`, `CompletionMode`, `ToolSource` (the variant only), `FinishReason`, `ProviderKind`, `Effort`, and `NetworkTransport` each have an `as_str` and a `Display` that print the serde spelling; a unit test in the model pins the wire spellings, and a test in `lablet-conformance`, which may depend on both crates, compares `StopReason`, `CompletionMode`, and `ToolSource` with the generated registry enums through exhaustive matches, so a variant added on either side doesn't compile until the other has it.

**Message validation.** `Message::new` and `Message::validate` hold the rules both provider APIs impose: tool-use blocks only in assistant messages with ids unique within the message, tool-result blocks only in user messages with one result for each call id. The first violation in block order is reported. The fields are public and deserialisation doesn't validate, so an adapter validates the message it builds from a response and reports a failure as `Malformed`.

**Transcript.** `Transcript` is the conversation of one run and the loop's working state: `messages` is the slice a provider call sends, and `push_assistant` stores the completion's `Usage` and response model beside the message it produced. It maps onto an ATIF v1.8 trajectory without a model change: `steps()` yields one `Step` for each assistant message, its `observation` is the following user message when that message holds tool results, `result_of` joins a result to its call by call id within that step (so an id a fake script repeats across turns still joins correctly), and a step's metrics are `prompt_tokens = input_tokens`, `completion_tokens = output_tokens`, `cached_tokens = cache_read_tokens`, which agrees with ATIF's rule that cached tokens are a subset of prompt tokens. The push methods take content rather than a `Message` so the role, and with it the pairing of `turns` with assistant messages, can't be wrong.

Replaying `Thinking`, `RedactedThinking`, and `Opaque` blocks unchanged and in order is required for Anthropic, which rejects modified thinking blocks in the latest assistant turn.

## 4. Domain policy (`lablet-policy`)

Pure functions over plain state, fully unit-testable without async:

```rust
pub struct StopPolicy { pub completion: CompletionMode, pub max_turns: u32, pub timeout: Duration, pub max_total_tokens: Option<u64>, pub max_consecutive_tool_errors: u32 }
pub struct RunState { pub turn: u32, pub elapsed: Duration, pub total_tokens: u64, pub consecutive_tool_errors: u32, pub last_finish: Option<FinishReason>, pub last_had_tool_use: bool, pub task_complete_called: bool }   // turn is one-based; Default
pub enum StopPoint { BeforeProviderCall, AfterProviderResponse, AfterToolPhase }   // points A, R, and B of §1
impl StopPolicy { pub fn evaluate(&self, at: StopPoint, state: &RunState) -> Option<StopReason> }

pub struct RetryPolicy { max_attempts: u32, base: Duration, max: Duration, factor: f64 }
impl RetryPolicy {
    pub fn new(max_attempts: u32, base: Duration, max: Duration, factor: f64) -> Result<RetryPolicy, RetryPolicyError>;
    pub fn delay(&self, attempt: u32) -> Option<Duration>;   // the wait after one-based attempt `attempt` failed; None when it was the last one allowed
}
pub enum RetryPolicyError { NoAttempts, BaseAboveMax { base: Duration, max: Duration }, Factor(f64) }

pub struct Pricing { input: f64, output: f64, cache_read: f64, cache_write: f64 }   // USD per million tokens
impl Pricing {
    pub fn new(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Result<Pricing, PricingError>;
    pub fn cost(&self, usage: &Usage) -> Cost;
}
pub enum PricingError { Rate { name: &'static str, value: f64 } }
```

**Stop policy.** `evaluate` holds the order and the boundaries §1 gives for each point, and decides seven of the stop reasons. The other four aren't its to decide: `cancelled` comes from the loop's poll of the `Cancellation` port, and `retries_exhausted`, `context_exhausted`, and `provider_error` from a failed provider call (`RetryPolicy::delay` returning `None`, and the `ProviderError` variant). Completion is a `StopPoint` and not a function of its own because `RunState` already carries what it reads (`last_finish`, `last_had_tool_use`, `task_complete_called`), so the loop has one shape at all three points: update the state, then ask. Natural mode doesn't read `task_complete_called`. Every `StopPolicy` value is meaningful, so its fields are public and nothing is validated: `max_turns: 0` and `max_consecutive_tool_errors: 0` act as 1, and a zero `timeout` or token budget stops the run at the first point A.

**Retry policy.** `max_attempts` is `run.max_retries`. `new` refuses a `max_attempts` of 0 (1 means a failed call isn't retried), a `base` longer than `max`, and a `factor` that isn't a finite number of at least 1. `delay` is total: attempt 0 is read as 1, and an attempt number or a factor large enough to overflow gives `max`, never a panic or a NaN.

**Pricing.** `new` refuses a rate that isn't a finite number of at least 0. `cost` prices `Usage::uncached_input_tokens()` at the input rate and each cache field once, at its own rate, because `input_tokens` already includes both (§3); pricing `input_tokens` whole and adding the cache fields would bill the cached tokens twice. A cost is never negative and never NaN.

## 5. Application (`lablet-run`)

### Secondary ports

```rust
#[async_trait] pub trait ModelProvider: Send + Sync {
    fn model(&self) -> &ModelRef;
    fn endpoint(&self) -> Option<Endpoint>;                     // server.address and server.port; None for fake
    async fn complete(&self, req: CompletionRequest<'_>) -> Result<Completion, ProviderError>;
}
pub struct CompletionRequest<'a> { system: &'a str, messages: &'a [Message], tools: &'a [ToolSpec], max_tokens: u32, temperature: Option<f32>, thinking: Thinking, seed: Option<u64>, deadline: Duration }   // Thinking is a model type (§3), because RequestDefaults holds one
pub enum ProviderError { Retryable(String), ContextExhausted(String), Fatal(String), Malformed(String) }

#[async_trait] pub trait ToolExecutor: Send + Sync {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError>;
    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError>;
}
pub struct ToolCall { id: ToolCallId, name: ToolName, input: serde_json::Value, deadline: Duration, trace_context: Option<TraceContext> }
pub struct ToolOutput { content: Vec<ToolResultContent>, is_error: bool, mcp: Option<McpCallMeta> }
pub struct ToolError { kind: ToolErrorKind, message: String, mcp: Option<McpCallMeta> }   // every kind becomes an error result for the model
pub enum ToolErrorKind { Unknown, Timeout, Failed }

#[async_trait] pub trait RunObserver: Send + Sync {
    async fn on(&self, event: RunEvent);
    fn trace_context(&self, call_id: &ToolCallId) -> Option<TraceContext> { None }   // the OTel observer answers with the tool span it opened on ToolCallStarted
}
pub enum RunEvent {                       // every variant carries run_id
    RunStarted { context: RunContext, model, endpoint: Option<Endpoint>, tools: Vec<ToolSpec>, system_prompt: Option<String>, prompt: Option<String> },
    TurnStarted { turn },
    ProviderCallStarted { turn, attempt, request_bytes },
    ProviderCallFinished { turn, attempt, usage, finish, latency, response_id, response_model, response: Option<Message> },
    ProviderCallFailed { turn, attempt, error, will_retry, backoff },
    ToolCallStarted { turn, call_id, name, source, input_bytes, input: Option<Value> },
    ToolCallFinished { turn, call_id, is_error, error_kind: Option<ToolErrorKind>, output_bytes, latency, mcp: Option<McpCallMeta>, output: Option<ToolOutput> },
    RunFinished { context: RunContext, summary: RunSummary },
}

#[async_trait] pub trait Clock: Send + Sync { fn now(&self) -> Instant; async fn sleep(&self, d: Duration); }
pub trait Cancellation: Send + Sync { fn is_cancelled(&self) -> bool; }
```

The loop emits `ToolCallStarted`, then asks the observer for a `TraceContext` for that call id and places it on the `ToolCall`, so the MCP adapter can inject it into `params._meta`. The fan-out observer (kept so library users can register their own `RunObserver` beside the built-in one) returns the first `Some`; observers without spans return `None`. The OTel observer keeps open spans in a map keyed by call id, starts every child with an explicit parent context rather than the task-local current context (since `on` runs on arbitrary tasks), and uses an always-on sampler so the propagated flags are `01`; the span context is fixed at start and readable synchronously, and batch processors only see ended spans, so the handoff needs no coordination with export. `error.type` is defined per span: on the root it's the stop reason when the run didn't complete, on a chat span the `ProviderError` variant name (`retryable`, `context_exhausted`, `fatal`, `malformed`), on a tool span the `ToolErrorKind` name when the executor failed, and `tool_error`, the value the MCP conventions give, when the tool ran and returned an error result.

Every per-step event carries `turn`, which observers emit as `lablet.turn` on the corresponding span. There is no turn span; the trace is `invoke_agent` with `chat` and `execute_tool` children directly beneath it, as the conventions describe, and turns are recovered by filtering on the index. A turn span can be added under the root later without changing anything else.

Content-bearing fields on events (`system_prompt`, `prompt`, `response`, `input`, `output`) are `None` unless `telemetry.capture_content` is on. The loop decides, so observers never see content they shouldn't. When content is off the attribute is omitted, not written as a placeholder; the byte-count attributes are always present. The intercepted `task_complete` argument appears in the outcome and transcript regardless, and in telemetry only when content is captured.

Observers never fail the run and never block it: `on` must return promptly, exporters buffer and batch, and export failures (an unreachable OTLP endpoint, an unwritable file) go to lablet's diagnostic log. The composition root flushes every observer before exit and reports flush failures on stderr without changing the exit code.

The `Clock` port is the only source of time for the loop, so `lablet-run` tests drive timeouts and backoff with a fake clock. Adapters enforce their own per-call deadlines with real time.

### Use case

```rust
pub struct RunService { provider: Arc<dyn ModelProvider>, tools: Arc<dyn ToolExecutor>, observer: Arc<dyn RunObserver>, clock: Arc<dyn Clock>, cancel: Arc<dyn Cancellation>, stop: StopPolicy, retry: RetryPolicy, request: RequestDefaults }
impl RunService { pub async fn run(&self, context: RunContext, system: String, prompt: String) -> (RunOutcome, RunSummary) }
```

`run` never returns `Err`: every failure is a `RunOutcome` with a stop reason, so the caller always gets telemetry-consistent output. One `RunService` executes one run at a time; concurrent calls are a programming error and are rejected.

`ToolSet` is a composite `ToolExecutor` in this crate that routes by name across several executors, applies the config allow and deny lists, and rejects duplicate names at build time. The `task_complete` tool spec is defined in this crate, registered by `ToolSet` only in `explicit` mode regardless of `tools.builtin.enabled`, and never executed: the loop intercepts it. Its input schema is `run.completion_schema` when set, otherwise a free-form object. `ToolSet` is where "what happens when a tool is missing" is expressed: the tool is simply absent from `specs()`.

Tool calls within one turn are executed sequentially. Parallel execution is a later option; the observer events already carry enough to distinguish it.

## 6. Adapters

### `provider-anthropic`

Messages API via `reqwest` with `rustls`. Maps `ContentBlock` both ways, including `thinking`, `redacted_thinking`, and cache-control (applied to the system prompt and tool specs when `model.cache: true`). Reads `cache_read_input_tokens` and `cache_creation_input_tokens` into `Usage`; the latter is reported as `gen_ai.usage.cache_write.input_tokens`, the semantic-convention name, not Anthropic's. Anthropic's own `input_tokens` leaves both out, so the adapter adds them to it: `Usage.input_tokens` includes cached tokens (§3). Thinking: `Default` sends nothing (current models think adaptively by default), `Enabled` sends `thinking: {type: enabled, budget_tokens}`, `Disabled` sends `type: disabled`; `effort` maps to `output_config.effort` and is reported as `gen_ai.request.reasoning.level`. `temperature` is sent only when set; the build step warns that models from Opus 4.7 and Sonnet 5 onward reject a non-default value. Validates `budget < max_tokens` at build. Classifies 429, 529, 5xx, transport errors, and per-call timeouts as retryable, the `prompt is too long` invalid-request error as context exhausted, and 401 and 403 as fatal. Ignores `seed`.

### `provider-openai`

`/v1/chat/completions` with function calling. Covers OpenAI, Ollama, vLLM, and gateways via `base_url`. Sends `max_completion_tokens` and falls back to `max_tokens` if the server rejects it. One user message holding N `ToolResult` blocks becomes N `role: tool` messages on the way out. `Thinking` blocks are dropped on the way out; `reasoning_content` and `reasoning` fields are mapped to `Thinking` with an empty signature on the way in. Function `arguments` that aren't valid JSON are `Malformed`. Unknown fields go to `Opaque`. Passes `seed` when set. Classifies `context_length_exceeded` and equivalents as context exhausted. `api_key_env` is optional; no header is sent when it's unset.

### `provider-fake`

Plays a scripted sequence of `Completion`s from a YAML or JSON file (the domain model's serde form), with optional per-call latency (real time) and injected errors of each `ProviderError` class. Selected with `model.provider: fake` and `model.script: path`. Reports usage from the script so token accounting is exercised end to end. Used by lablet's smoke tests, doctests, and examples, and by users testing their own frameworks. `lablet init --provider fake` writes both a config and a script.

### `tools-builtin`

Each tool is a small struct; the executor holds only those enabled in config. Initial set: `bash` (working directory and timeout from config, captures stdout, stderr, exit code), `read_file`, `write_file`. All paths are resolved under `tools.builtin.root` and rejected if they escape it. `task_complete` isn't here; see §5.

### `tools-mcp`

One `rmcp` client per configured server (version pinned in the workspace `Cargo.toml`). Tool names are exposed exactly as the server reports them, so measurements reflect the server as-is. A reported name, or a prefixed one, that isn't a valid `ToolName` (1 to 64 of `[a-zA-Z0-9_-]`, which is what both provider APIs accept) is a build error with the `mcp:` prefix naming the server and the tool, since no provider would accept it. A name collision across servers is a build error; a server with `prefix_tools: true` has its tools renamed `<server>__<tool>`, which is the escape hatch. `execute` forwards the call and maps `isError` results to `is_error: true`. It injects the `ToolCall`'s `trace_context` into the request's `params._meta` as unprefixed `traceparent` and `tracestate`, per MCP SEP-414, and fills `McpCallMeta` (method, session id, protocol version, JSON-RPC request id, RPC status code on error, transport `pipe` for stdio or `tcp` for HTTP) on the output or error so the OTel observer can put `mcp.*`, `jsonrpc.*`, `rpc.*`, and `network.transport` on the `execute_tool` span; the conventions want one span carrying both `gen_ai.tool.*` and `mcp.*`, not a nested MCP span. Servers are started when the `Lablet` is built, with `tools.mcp[].startup_timeout`, live for the lifetime of the `Lablet` across runs, and are shut down by `Lablet::shutdown`. A stdio server's stderr is forwarded line by line to the diagnostic log at `debug`. HTTP servers receive `headers` verbatim. If a server exits or its transport breaks mid-run, every call to its tools returns a tool error naming the server, so the consecutive error cap ends the run; its tools stay listed so the model's behaviour is observable.

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

| Event                   | Span                                                                                                                                        | Key attributes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| RunStarted..RunFinished | `invoke_agent lablet` (root, INTERNAL)                                                                                                      | `gen_ai.operation.name=invoke_agent`, `gen_ai.agent.name=lablet`, `gen_ai.agent.version`, `gen_ai.request.model`, the join keys (`gen_ai.conversation.id` and `session.id`, both the run id, and `lablet.config.digest`), `lablet.run.stop_reason`, `error.type` on failure, `lablet.run.turns`, `lablet.tool_calls.total`, aggregate `gen_ai.usage.*` (the conventions allow the aggregate on `invoke_agent`; a query summing `gen_ai.usage.*` over every span in a trace double counts and must filter on `gen_ai.operation.name`), `lablet.run.cost_usd` when pricing configured        |
| ProviderCall*           | `chat <model>` child of root (CLIENT)                                                                                                       | `gen_ai.operation.name=chat`, the join keys, `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.request.max_tokens`, `gen_ai.request.temperature` when set, `gen_ai.request.seed` when set, `gen_ai.request.reasoning.level` when set, `gen_ai.response.model`, `gen_ai.response.id`, `gen_ai.response.finish_reasons`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_write.input_tokens`, `server.address`, `server.port`, `error.type` on failure, `lablet.turn`, `lablet.attempt`, `lablet.request.bytes` |
| ToolCall*               | `execute_tool <name>` child of root (INTERNAL)                                                                                              | `gen_ai.operation.name=execute_tool`, the join keys, `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.tool.type` (`function` for builtin, `extension` for MCP), `gen_ai.tool.description`, `error.type` on failure, `lablet.turn`, `lablet.tool.source`, `lablet.tool.input.bytes`, `lablet.tool.output.bytes`, `lablet.tool.is_error`; for MCP tools also `mcp.method.name`, `mcp.session.id`, `mcp.protocol.version`, `jsonrpc.request.id`, `rpc.response.status_code`, `network.transport`                                                                                            |
| ProviderCallFailed      | `gen_ai.client.operation.exception` log record (severity WARN) in the chat span's context, plus a span event on the chat span for the retry | log record: `exception.type` (the `ProviderError` variant name, as `error.type` on the span), `exception.message`, the join keys, `gen_ai.operation.name`, `gen_ai.provider.name`, `gen_ai.request.model`, `lablet.turn`, `lablet.attempt`; span event `lablet.retry`: `lablet.attempt`, `lablet.retry.will_retry`, `lablet.retry.backoff_ms` when it will. The deprecated `exception` span event isn't used                                                                                                                                                                               |
| RunFinished             | one log record `lablet.run` (the wide event), trace context of the root span                                                                | the §1 wide-event attribute set                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |

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

A `Lablet` may run many times; each `run` gets a fresh `RunId` and `RunContext`, MCP servers persist across runs, and `shutdown` is the only teardown. Runs on one `Lablet` are sequential.

While the build is in progress, a config that selects an adapter whose crate doesn't exist yet makes `build` return `BuildError::Unsupported { kind, phase }` naming the build-plan phase that delivers it; config validation, including the `api_key_env` check, runs before adapter selection and doesn't depend on the adapter. Two outcomes are "identical" when they're equal after removing `run_id` and `duration_ms`.

`main.rs` only does effects: parse args with `clap` derive, install a `tracing` subscriber on stderr filtered by `RUST_LOG` (default `warn`) for lablet's own diagnostics, load config, install Ctrl-C handling into the `Cancellation` port, `build`, `run`, print outcome, write the transcript, flush telemetry, print one human-readable summary line on stderr (stop reason, turns, tokens, tool calls, duration; suppressed by `--quiet`), exit. Diagnostics and telemetry are separate: the diagnostic log is about lablet, the telemetry is about the run. The outcome print is the one permitted `print_stdout`, marked with `#[expect]`.

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
  max_total_tokens: null # input + output across the run (input already includes cached tokens); null means unlimited
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
  thinking:
    mode: default # default | enabled | disabled
    budget: null # tokens, enabled mode only, must be below max_tokens
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
- `lablet-test-mcp-server` is an rmcp stdio binary with tools that echo, sleep for N seconds, and make the server exit after N calls; cargo sets `CARGO_BIN_EXE_lablet-test-mcp-server` only for the server package's own tests, so how tests in other packages locate the binary is an open question (§10). A `--hang-startup` flag never completes initialisation.
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
- A Parquet exporter for developers without a collector: a third `SpanExporter` and `LogExporter` pair beside OTLP and the OTLP/JSON file, probably in the OTel-Arrow (OTAP) layout. Deferred because choosing the file layout is a contract decision; the OTLP/JSON file covers the lightweight case today.
