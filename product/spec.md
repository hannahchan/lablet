# Lablet specification

Status: draft for build. Companion to [brief.md](brief.md), [quality-bar.md](quality-bar.md), and [build-plan.md](build-plan.md).

## 1. What a run is

A run takes a config and a task prompt and executes one agent loop:

```
turn = 1
messages = [user(prompt)]
loop:
    response = provider.complete(system, messages, tool specs)      # retried on transient error
    messages += response.message
    if stop policy says stop (completion, turns, timeout, retries, tool errors): break
    for each tool_use block in response: result = tools.execute(call); messages += tool_result
    turn += 1
emit outcome
```

Every provider call, tool call, retry, and stop decision is reported to an observer, which is how telemetry leaves the process.

### Completion modes

| Mode | Completes when | Ends without completing when |
| --- | --- | --- |
| `natural` (default) | The model returns a turn with no tool calls. | Never. Other stop reasons still apply. |
| `explicit` | The model calls the built-in `task_complete` tool. Its JSON argument is the run's structured result. | The model returns a turn with no tool calls. Stop reason `ended_without_completion`. |

### Stop reasons

`completed`, `ended_without_completion`, `max_turns`, `timeout`, `max_total_tokens`, `output_truncated` (the model hit `max_tokens` without a tool call), `context_exhausted` (the provider rejected the request as too long), `retries_exhausted`, `tool_errors_exhausted`, `cancelled`, `provider_error` (any other non-retryable error).

### Retries

- **Provider errors** are classified by the adapter as retryable (transport failure, rate limit, 5xx, overloaded), context exhausted (the provider's context-length error), or fatal (auth, bad request, unknown model). Retryable errors are retried with exponential backoff up to `run.max_retries` per run, then the run stops with `retries_exhausted`.
- **Malformed provider responses** (the adapter cannot map the payload to the domain model) count as retryable.
- **Tool errors** are not retried by lablet. The error is returned to the model as an error tool result. After `run.max_consecutive_tool_errors` consecutive error results the run stops with `tool_errors_exhausted`. A successful tool call resets the counter.
- **Tool timeouts** (`run.tool_timeout`) are tool errors.
- **Provider timeouts** (`run.provider_timeout`) are retryable errors, enforced per call by the adapter's HTTP client via the deadline on the request. The run-level `run.timeout` is checked between steps only, so a run may overrun it by at most one provider or tool call.

### Timeouts and cancellation

Cancellation (Ctrl-C in the CLI) is polled between steps. A run in the middle of a long tool call stops after that call returns or times out. Interrupting a call in flight is an open question.

### Transcript

When `run.transcript_path` is set, the full message list (system prompt, every message, every tool result) is written there as JSON when the run ends, whatever the stop reason. This is independent of telemetry and `capture_content`; it is how a grader lablet or an outer framework gets the conversation.

### Outcome

Written to stdout as one JSON document when the run ends, whatever the stop reason:

```json
{
  "run_id": "01J...",
  "stop_reason": "completed",
  "turns": 7,
  "usage": { "input_tokens": 0, "output_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0 },
  "tool_calls": 5,
  "duration_ms": 12345,
  "result": { "text": "final assistant text", "structured": { "...": "task_complete argument, explicit mode only" } },
  "error": null
}
```

Exit codes: `0` completed, `2` ended with any other stop reason, `1` config or startup failure (nothing ran).

### Wide event

When a run ends, every observer emits exactly one wide event: a single record carrying everything worth knowing about the run, so an analyst can answer most questions from one row without joining spans. It is emitted after `RunFinished`, whatever the stop reason, and is the last thing the run produces. Its content is the `RunSummary` type in §3, flattened to attributes with dotted names:

| Group | Fields |
| --- | --- |
| identity | `run.id`, `config.digest`, `lablet.version`, every `telemetry.resource` attribute |
| setup | `model.provider`, `model.name`, `run.completion_mode`, `run.max_turns`, `run.timeout_ms`, `tools.names` (list), `tools.count`, `tools.mcp_servers` (list), `prompt.system_bytes`, `prompt.user_bytes`, `skills.count` |
| outcome | `stop_reason`, `error`, `duration_ms`, `turns`, `result.text_bytes`, `result.has_structured`, `transcript_path` |
| provider | `provider.calls`, `provider.retries`, `provider.latency_ms.total`, `provider.latency_ms.max`, `usage.input_tokens`, `usage.output_tokens`, `usage.cache_read_tokens`, `usage.cache_write_tokens`, `usage.total_tokens`, `cost_usd` (when pricing configured), `finish_reasons` (counts by reason) |
| tools | `tool_calls.total`, `tool_calls.errors`, `tool_calls.latency_ms.total`, `tool_calls.input_bytes.total`, `tool_calls.output_bytes.total`, and per tool `tool.<name>.calls`, `tool.<name>.errors`, `tool.<name>.latency_ms.total` |
| content | `result.text` and `result.structured` only when `telemetry.capture_content` is on |

The same record goes to every configured observer: an OTel log record for OTLP, the final line for JSONL. Spans remain the per-step detail; the wide event is the per-run row.

## 2. Architecture

Explicit architecture, in the shape of the UsefulBytes repository, sized down. Rings are directory prefixes inside the `lablet/` workspace; directory `foo/bar/` is package `lablet-bar`.

```
lablet/
  crates/domain/model            conversation, tools, usage, stop reasons, run identity
  crates/domain/policy           pure stop and retry decisions
  crates/application/run         RunService (the loop) and its secondary ports
  crates/adapters/secondary/
    provider-anthropic           ModelProvider over the Anthropic Messages API
    provider-openai              ModelProvider over OpenAI-compatible chat completions (Ollama, vLLM, gateways)
    provider-fake                ModelProvider that plays scripted responses; a product feature, not test scaffolding
    tools-builtin                ToolExecutor for task_complete, bash, read_file, write_file
    tools-mcp                    ToolExecutor over rmcp (stdio and streamable HTTP clients)
    telemetry-otel               RunObserver emitting OTLP traces and logs
    telemetry-jsonl              RunObserver writing one JSON event per line to a file or stderr
    shared/telemetry-registry    generated from the Weaver registry: attribute names, span and event builders
  apps/lablet                    composition root: config, build(), CLI. Library and binary.
  telemetry/                     Weaver registry (YAML), policies, and codegen templates
  tests/conformance              shared ToolExecutor and RunObserver conformance cases
  xtask/                         (repo root, not a member) lint-layers, gates
```

Dependency direction, enforced by `cargo xtask lint-layers`:

| Ring | May depend on | May not use |
| --- | --- | --- |
| domain | domain | tokio, serde derive, reqwest, tracing, opentelemetry, rmcp |
| application | domain | tokio, serde derive, reqwest, opentelemetry, rmcp (`tracing` allowed) |
| adapters | application, domain | (no restriction) |
| composition root | everything | (no restriction) |

There are no primary adapters. The composition root calls `RunService` directly, which is also the library surface.

Ports are object-safe `async_trait` traits, `Send + Sync`, injected as `Arc<dyn Trait>` through constructors. Ports live in the application crate that consumes them. Errors are `thiserror` enums owned by the layer that defines the contract; adapters map their own errors into the port's error at the `impl` boundary and never leak their types inward.

## 3. Domain model (`lablet-model`)

Pure types. `serde_json::Value` is allowed as the JSON data currency (tool inputs and outputs are JSON by nature); serde derives are not.

```rust
pub struct RunId(String);                 // ULID, generated by the composition root
pub struct ModelRef { pub provider: ProviderKind, pub name: String }
pub enum Role { User, Assistant }
pub struct Message { pub role: Role, pub content: Vec<ContentBlock> }
pub enum ContentBlock {
    Text(String),
    Thinking { text: String, signature: Option<String> },
    ToolUse { id: ToolCallId, name: ToolName, input: serde_json::Value },
    ToolResult { call_id: ToolCallId, content: Vec<ToolResultContent>, is_error: bool },
    Opaque { provider: ProviderKind, payload: serde_json::Value },   // round-trips provider-specific blocks
}
pub enum ToolResultContent { Text(String), Json(serde_json::Value) }
pub struct ToolSpec { pub name: ToolName, pub description: String, pub input_schema: serde_json::Value, pub source: ToolSource }
pub enum ToolSource { Builtin, Mcp { server: String } }
pub struct Usage { input_tokens, output_tokens, cache_read_tokens, cache_write_tokens }   // u64, Add impl
pub struct Completion { pub message: Message, pub usage: Usage, pub finish: FinishReason }
pub enum FinishReason { EndTurn, ToolUse, MaxTokens, Other(String) }
pub enum StopReason { Completed, EndedWithoutCompletion, MaxTurns, Timeout, MaxTotalTokens, OutputTruncated, ContextExhausted, RetriesExhausted, ToolErrorsExhausted, Cancelled, ProviderError }
pub struct RunOutcome { ... as in §1 }
pub struct RunSummary { ... every field in the §1 wide-event table, typed; per-tool aggregates as a map keyed by ToolName }
```

`RunSummary` is accumulated by the loop, not reconstructed by observers, so every observer reports identical numbers.

Cache-related fields are zero for providers that do not report them. `Opaque` exists so Anthropic cache-control markers, thinking signatures, and similar survive a round trip without the domain knowing their shape.

## 4. Domain policy (`lablet-policy`)

Pure functions over plain state, fully unit-testable without async:

```rust
pub struct StopPolicy { pub completion: CompletionMode, pub max_turns: u32, pub timeout: Duration, pub max_total_tokens: Option<u64>, pub max_retries: u32, pub max_consecutive_tool_errors: u32 }
pub struct RunState { pub turn: u32, pub elapsed: Duration, pub total_tokens: u64, pub retries: u32, pub consecutive_tool_errors: u32, pub last_finish: Option<FinishReason>, pub last_had_tool_use: bool, pub task_complete_called: bool }
impl StopPolicy { pub fn evaluate(&self, state: &RunState) -> Option<StopReason> }
pub struct RetryPolicy { pub base: Duration, pub max: Duration, pub factor: f64 }
impl RetryPolicy { pub fn delay(&self, attempt: u32) -> Duration }
pub struct Pricing { per-million input, output, cache read, cache write }
impl Pricing { pub fn cost(&self, usage: &Usage) -> Cost }
```

## 5. Application (`lablet-run`)

### Secondary ports

```rust
#[async_trait] pub trait ModelProvider: Send + Sync {
    fn model(&self) -> &ModelRef;
    async fn complete(&self, req: CompletionRequest) -> Result<Completion, ProviderError>;
}
pub struct CompletionRequest<'a> { system: &'a str, messages: &'a [Message], tools: &'a [ToolSpec], max_tokens: u32, temperature: Option<f32>, thinking_budget: Option<u32>, seed: Option<u64>, deadline: Duration }
pub enum ProviderError { Retryable(String), ContextExhausted(String), Fatal(String), Malformed(String) }

#[async_trait] pub trait ToolExecutor: Send + Sync {
    async fn specs(&self) -> Result<Vec<ToolSpec>, ToolError>;
    async fn execute(&self, call: ToolCall) -> Result<ToolOutput, ToolError>;
}
pub struct ToolCall { id: ToolCallId, name: ToolName, input: serde_json::Value, deadline: Duration }
pub struct ToolOutput { content: Vec<ToolResultContent>, is_error: bool }
pub enum ToolError { Unknown(ToolName), Timeout, Failed(String) }

#[async_trait] pub trait RunObserver: Send + Sync {
    async fn on(&self, event: RunEvent);
}
pub enum RunEvent {
    RunStarted { run_id, model, tools: Vec<ToolSpec>, config_digest, system_prompt: Option<String>, prompt: Option<String> },
    TurnStarted { turn },
    ProviderCallStarted { turn, attempt, request_bytes },
    ProviderCallFinished { turn, attempt, usage, finish, latency, response: Option<Message> },
    ProviderCallFailed { turn, attempt, error, will_retry, backoff },
    ToolCallStarted { turn, call_id, name, source, input_bytes, input: Option<Value> },
    ToolCallFinished { turn, call_id, is_error, output_bytes, latency, output: Option<ToolOutput> },
    RunFinished { outcome: RunOutcome, summary: RunSummary },
}

#[async_trait] pub trait Clock: Send + Sync { fn now(&self) -> Instant; async fn sleep(&self, d: Duration); }
pub trait Cancellation: Send + Sync { fn is_cancelled(&self) -> bool; }
```

Content-bearing fields on events (`system_prompt`, `prompt`, `response`, `input`, `output`) are `None` unless `telemetry.capture_content` is on. The loop decides, so observers never see content they should not.

Observers never fail the run and never block it: `on` must return promptly, exporters buffer and batch, and export failures (an unreachable OTLP endpoint, an unwritable file) go to lablet's diagnostic log. The composition root flushes every observer before exit and reports flush failures on stderr without changing the exit code.

### Use case

```rust
pub struct RunService { provider: Arc<dyn ModelProvider>, tools: Arc<dyn ToolExecutor>, observer: Arc<dyn RunObserver>, clock: Arc<dyn Clock>, cancel: Arc<dyn Cancellation>, stop: StopPolicy, retry: RetryPolicy, request: RequestDefaults }
impl RunService { pub async fn run(&self, run_id: RunId, system: String, prompt: String) -> RunOutcome }
```

`run` never returns `Err`: every failure is a `RunOutcome` with a stop reason, so the caller always gets telemetry-consistent output.

`ToolSet` is a composite `ToolExecutor` in this crate that routes by name across several executors, applies the config allow and deny lists, and rejects duplicate names at build time. Built-in `task_complete` is only registered in `explicit` mode. `ToolSet` is where "what happens when a tool is missing" is expressed: the tool is simply absent from `specs()`.

Tool calls within one turn are executed sequentially. Parallel execution is a later option; the observer events already carry enough to distinguish it.

## 6. Adapters

### `provider-anthropic`
Messages API via `reqwest`. Maps `ContentBlock` both ways, including `thinking` and cache-control (applied to the system prompt and tool specs when `model.cache: true`). Reads `cache_read_input_tokens` and `cache_creation_input_tokens` into `Usage`. Classifies 429, 529, 5xx, transport errors, and per-call timeouts as retryable, and the `prompt is too long` invalid-request error as context exhausted. Ignores `seed`.

### `provider-openai`
`/v1/chat/completions` with function calling. Covers OpenAI, Ollama, vLLM, and gateways via `base_url`. `Thinking` blocks are dropped on the way out and reasoning content, when present, is mapped on the way in. Unknown fields go to `Opaque`. Passes `seed` when set. Classifies `context_length_exceeded` and equivalents as context exhausted.

### `tools-builtin`
Each tool is a small struct; the executor holds only those enabled in config. Initial set: `task_complete` (JSON schema from config or free-form object), `bash` (working directory and timeout from config, captures stdout, stderr, exit code), `read_file`, `write_file`. All paths are resolved under `tools.builtin.root` and rejected if they escape it.

### `tools-mcp`
One `rmcp` client per configured server. `specs()` lists tools from every server, prefixing names with `<server>__` only when two servers collide. `execute` forwards the call and maps `isError` results to `is_error: true`. Servers are started at build time with `tools.mcp[].startup_timeout` and shut down when the run ends. A stdio server's stderr is forwarded line by line to the diagnostic log at `debug`. If a server exits or its transport breaks mid-run, every call to its tools returns a tool error naming the server, so the consecutive error cap ends the run; its tools stay listed so the model's behaviour is observable.

### `provider-fake`
Plays a scripted sequence of `Completion`s from a YAML or JSON file, with optional per-call latency and injected errors of each `ProviderError` class. Selected with `model.provider: fake` and `model.script: path`. Reports usage from the script so token accounting is exercised end to end. Used by lablet's smoke tests, doctests, and examples, and by users testing their own frameworks.

### Telemetry contract (`lablet/telemetry/`)
Telemetry is contract-first. An OpenTelemetry Weaver semantic-convention registry under `lablet/telemetry/registry/` declares every attribute, span, event, and the wide event lablet emits. It imports the upstream semantic-conventions registry for `gen_ai.*` and defines `lablet.*`. From that registry:

- `weaver registry check` with the policies under `lablet/telemetry/policies/` runs as a gate (naming, stability, required fields).
- `weaver registry generate` with the Jinja templates under `lablet/telemetry/templates/rust/` produces the `lablet-telemetry-registry` crate: attribute name constants, typed attribute values, and span and event builders. The generated code is checked in and a gate fails if regeneration produces a diff.
- The same templates produce `lablet/docs/telemetry.md`.
- `weaver registry live-check` receives OTLP from a fake-provider run in CI and fails on any attribute, span, or event not in the registry, or with the wrong type.

The tables in this document are the human summary; the registry is the source of truth. When they disagree, the registry wins and this document is corrected.

### `telemetry-otel`
Spans and logs via `opentelemetry` and `opentelemetry-otlp` (gRPC or HTTP/protobuf), using the batch span and log processors so export never sits on the loop's path. Every attribute name and builder comes from `lablet-telemetry-registry`; string literals for attribute names are a lint failure in this crate. Follows the GenAI semantic conventions; anything the conventions lack uses the `lablet.` namespace.

| Event | Span | Key attributes |
| --- | --- | --- |
| RunStarted..RunFinished | `invoke_agent lablet` (root) | `gen_ai.operation.name=invoke_agent`, `gen_ai.agent.name=lablet`, `gen_ai.conversation.id=<run_id>`, `lablet.config.digest`, `lablet.run.stop_reason`, `lablet.run.turns`, `lablet.run.tool_calls`, total `gen_ai.usage.*`, `lablet.run.cost_usd` when pricing configured |
| ProviderCall* | `chat <model>` child of root | `gen_ai.operation.name=chat`, `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.response.finish_reasons`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens`, `lablet.turn`, `lablet.attempt`, `lablet.request.bytes` |
| ToolCall* | `execute_tool <name>` child of root | `gen_ai.operation.name=execute_tool`, `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.tool.type`, `lablet.tool.source`, `lablet.tool.input.bytes`, `lablet.tool.output.bytes`, `lablet.tool.is_error` |
| ProviderCallFailed | span event on the chat span | `exception.message`, `lablet.retry.will_retry`, `lablet.retry.backoff_ms` |
| RunFinished | one log record `lablet.run` (the wide event), correlated to the root span by trace and span id | every `RunSummary` field under the `lablet.` prefix, plus the `gen_ai.usage.*` totals under their conventional names |

Content, when captured, is emitted as `gen_ai.client.inference.operation.details` log records carrying `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.system_instructions`, and tool inputs and outputs. Resource attributes: `service.name=lablet`, `service.version`, plus `telemetry.resource` from config. The exporter is flushed before the process exits.

### `telemetry-jsonl`
One JSON object per `RunEvent`, to a file path or stderr, with the wide event as the final line (`{"event": "run_summary", ...}`). Always available, no endpoint needed. The default observer when no OTLP endpoint is configured. Both observers can run at once through a fan-out observer in the composition root.

## 7. Composition root (`apps/lablet`, package `lablet`)

Library and binary in one package. `lib.rs` exposes:

```rust
pub struct Config { ... }                              // serde, YAML or JSON, deny_unknown_fields
impl Config { pub fn from_path(p) -> Result<Config, ConfigError>; pub fn from_str(s, Format) -> ...; }
pub struct Lablet { service: RunService, shutdown: Vec<Box<dyn Shutdown>> }
pub async fn build(config: Config) -> Result<Lablet, BuildError>;
impl Lablet { pub async fn run(&self, prompt: &str) -> RunOutcome; pub async fn shutdown(self); }
```

`main.rs` only does effects: parse args with `clap` derive, install a `tracing` subscriber on stderr filtered by `RUST_LOG` (default `warn`) for lablet's own diagnostics, load config, install Ctrl-C handling into the `Cancellation` port, `build`, `run`, print outcome, write the transcript, flush telemetry, exit. Diagnostics and telemetry are separate: the diagnostic log is about lablet, the telemetry is about the run.

```
lablet init [--provider anthropic|openai|fake] [path]   # write a working starter config
lablet run --config lablet.yaml [--prompt "..." | --prompt-file f | stdin] [--set key=value ...]
lablet check --config lablet.yaml [--resolved]          # validate, start MCP servers, list tools; --resolved prints the full config
lablet schema                                           # print the config JSON schema
```

Error messages from `check` and startup name the config key, line, offending value, and accepted values, and distinguish config errors, MCP server startup failures, and provider rejections.

`--set` applies dotted overrides (`--set run.max_turns=5`). Environment variable substitution `${VAR}` is applied to string values. No provenance tracking, retired-key roster, or inert-key warnings: unknown keys are errors and that is the whole config policy.

### Config

```yaml
run:
  completion: natural            # natural | explicit
  max_turns: 30
  timeout: 10m
  max_total_tokens: null         # input + output across the run; null means unlimited
  max_retries: 3
  max_consecutive_tool_errors: 3
  tool_timeout: 60s
  provider_timeout: 120s
  transcript_path: null          # write the full conversation as JSON at run end

model:
  provider: anthropic            # anthropic | openai | fake
  script: null                   # fake only: path to the scripted responses
  name: claude-sonnet-5
  api_key_env: ANTHROPIC_API_KEY # never the key itself
  base_url: null                 # override for gateways, Ollama, vLLM
  max_tokens: 4096
  temperature: null
  thinking_budget: null
  seed: null                     # passed through when the provider supports it
  cache: true                    # anthropic only
  pricing: null                  # { input, output, cache_read, cache_write } USD per million tokens

prompt:
  system: "You are ..."          # or system_file: path
  skills: []                     # paths to SKILL.md files, appended to the system prompt in order

tools:
  builtin:
    root: .                      # sandbox root for file and bash tools
    enabled: [bash, read_file, write_file]
  mcp:
    - name: docs
      transport: stdio
      command: npx
      args: ["-y", "@example/docs-mcp"]
      env: { }
      startup_timeout: 30s
    - name: search
      transport: http
      url: http://localhost:8080/mcp
      headers: { }
  allow: null                    # list of tool names; null means all
  deny: []                       # removed after allow is applied

telemetry:
  capture_content: false
  otlp:
    endpoint: null               # e.g. http://localhost:4317; null disables the OTel observer
    protocol: grpc               # grpc | http
    headers: { }
  jsonl:
    path: null                   # file path, "-" for stderr, null disables
  resource: { }                  # extra resource attributes, e.g. experiment ids
```

Config digest (`lablet.config.digest`) is a SHA-256 of the canonical JSON form of the config after `--set` overrides but before `${VAR}` substitution, so any value injected from the environment (API keys, auth headers) never enters the digest and runs can be grouped by configuration in analysis.

## 8. Testing

- Unit tests inline. Policy and model crates are the bulk and need no async.
- `lablet-run` is tested end to end with hand-written fakes for every port. No mocking framework.
- Provider adapters are tested against `wiremock` with recorded payloads, including error classification and content-block round trips.
- `tests/conformance` holds case sets for `ToolExecutor` and `RunObserver`, pulled in as dev-dependencies by each adapter.
- One integration test target per crate (`tests/it/main.rs`).
- A smoke test in `apps/lablet` runs a full loop with a fake provider and the JSONL observer and asserts on the event stream.

## 9. Open questions

Recorded here so they are not lost; none block the first build.

- Parallel tool execution within a turn.
- Interrupting a provider or tool call in flight on cancellation, rather than waiting for it.
- Loading skills through a tool rather than inlining.
- Streaming responses (not needed for measurement, may be needed for very long outputs).
- A `lablet.run` metric set alongside traces, once there is a consumer for it.
- Structured `task_complete` schema validation.
