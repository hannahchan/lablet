# Instrumentation catalogue

Researched 2026-09-18. A deduplicated inventory of what agent SDKs, observability platforms, eval frameworks, coding agents, and the OpenTelemetry GenAI conventions instrument, merged from the five source-group documents in this folder. The inventory is fact; the **Rec** column and §9 are opinion.

**Rec** values: `adopt` (emit as named), `adapt` (emit the idea under a lablet name or with a change), `later` (worth having, not for the first build), `skip` (not relevant to a benchmarking loop). Source tags: **std** semconv-genai, **sdk** agent SDKs, **plat** observability platforms, **eval** eval frameworks, **code** coding agents. A name in the Name column is the canonical (semconv) spelling where one exists.

## 1. Span structure

| Concept | Canonical | Who has it | Notes | Rec |
| --- | --- | --- | --- | --- |
| Run root | `invoke_agent {agent}` INTERNAL, `gen_ai.operation.name=invoke_agent` | std; 6 of 8 sdk; plat (Honeycomb, Datadog kind inference); code (Copilot) | Datadog rejects tool/task as root; Braintrust drops rootless traces; Langfuse names the trace after it | adopt |
| Turn | none in std | sdk (OpenAI `turn`, Strands `execute_event_loop_cycle`, Mastra `MODEL_STEP`); code (Claude Code `interaction`, Codex `turn.id`) | Most SDKs flatten. A loop tool is the natural owner of a turn span | adapt: `lablet.turn {n}` span or `lablet.turn.index` on children |
| Inference | `chat {model}` CLIENT, covers the whole call **including retries** | std; sdk; plat; code | One span per logical call; retries are span events, not child spans | adopt |
| Tool execution | `execute_tool {tool}` INTERNAL | std; sdk; plat; code | For MCP tools the same span carries `mcp.*`; std says do not nest a second MCP span | adopt |
| Retry attempt | span event on the chat span (std `gen_ai.client.operation.exception` log record) | std; code (Claude Code `api_error` with `attempt`) | | adopt |
| Compaction | ADK `gen_ai.compaction.*`, Inspect `CompactionEvent{tokens_before, tokens_after}`, Gemini `chat_compression` | sdk; eval; code | Lablet has no compaction yet | later |
| Sub-agent | `invoke_agent` child, ATIF `subagent_trajectory_ref`, Claude `parent_tool_use_id` | std; sdk; eval | Out of scope for one loop | skip |
| Approval / permission | Claude `tool_decision`, Gemini `decision`, Codex `tool_decision`, Inspect `ApprovalEvent` | code; eval | Human-in-the-loop only | skip |

## 2. Attributes

### 2.1 Identity and correlation

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `gen_ai.conversation.id` | string | all spans | std; sdk; plat (Honeycomb, Datadog, Opik session key); code | Must be a real id, never derived from trace id | adopt, = run id |
| `session.id` | string | all spans | std (opt-in); plat (Langfuse, Phoenix session key); code (Claude, Gemini, Copilot) | Fragmented session keys across platforms; emit both | adopt, = run id |
| `gen_ai.agent.name` | string | all spans | std; sdk; plat (Honeycomb Agent Timeline requires it) | `lablet` or a configured name | adopt |
| `gen_ai.agent.version` | string | root | std; sdk (ADK metric dim) | lablet version | adopt |
| `gen_ai.tool.call.id` | string | tool span | std; every sdk; plat; code | The join key between request, execution, and result | adopt |
| `gen_ai.response.id` | string | chat span | std; code (Claude `request_id`) | Provider response id | adopt |
| response id on tool spans | string | tool span | code (OpenHands `llm_response_id`, Claude `message.uuid`) | Groups the N tool calls of one response | adapt: `lablet.response.id` |
| per-process sequence | int | every event | code (Claude `event.sequence`, Codex `tool_result_seq`) | Total order independent of clocks | adapt: `lablet.event.sequence` |
| `gen_ai.agent.call.id` | string | baggage | sdk (Pydantic run id) | Same purpose as run id | skip |
| user, org, terminal, entrypoint ids | string | all | code (Claude, Codex, Cline, Gemini) | Human-tool identity | skip |
| task id, sample index, epoch | string, int | root | eval (Inspect `id`/`epoch`, Harbor `task_id`, tau2 `task_id`/`trial`) | Composers need these to join runs to tasks | adapt: `lablet.task.id`, `lablet.sample.index` from `telemetry.resource` |

### 2.2 Model request

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `gen_ai.provider.name` | string enum | chat, root | std (Req); sdk; plat; code | `anthropic`, `openai`, or a custom value for fake | adopt |
| `gen_ai.request.model` | string | chat | std; all | | adopt |
| `gen_ai.request.max_tokens` | int | chat | std (Rec); sdk; plat | | adopt |
| `gen_ai.request.temperature`, `top_p`, `top_k` | double, int | chat | std (Rec); sdk; plat | Only when set | adopt |
| `gen_ai.request.seed` | int | chat | std; sdk (Mastra) | | adopt |
| `gen_ai.request.reasoning.level` | string | chat | std | Anthropic `effort` maps here | adopt |
| thinking budget | int | chat | none in std | | adapt: `lablet.request.thinking_budget` |
| `gen_ai.request.stream` | bool | chat | std | Always false for now | adopt |
| `gen_ai.request.choice.count`, `stop_sequences`, `frequency_penalty`, `presence_penalty` | mixed | chat | std | Not in lablet's config | skip |
| `gen_ai.tool.definitions` | JSON | chat, root | std (OptIn); sdk (Strands, Pydantic); code (Gemini, Copilot) | Description and schema not recommended by default | adopt, gated |
| request size | int | chat | code (Codex, Claude sizes) | Bytes of the serialised request | adapt: `lablet.request.bytes` (already in spec) |
| `server.address`, `server.port` | string, int | chat | std (Rec); sdk | Real backend, e.g. Ollama host | adopt |
| cache enabled / cache-control present | bool | chat | none | Lablet-specific experiment variable | adapt: `lablet.request.cache_control` |

### 2.3 Model response

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `gen_ai.response.model` | string | chat | std; plat (Datadog primary model key) | | adopt |
| `gen_ai.response.finish_reasons` | string[] | chat | std; sdk; plat; code | Report `error` when none | adopt |
| `gen_ai.response.status` | enum | chat | std registry | `completed`, `failed`, `cancelled`, … | later |
| `gen_ai.response.time_to_first_chunk` | double s | chat | std (streaming) | Also `ttft_ms` in Claude, Codex, Cline, Copilot | later, needs streaming |
| `error.type` | string | chat, tool, root | std (Stable, CondReq) | Provider error code or class | adopt |
| attempt number | int | span event | code (Claude `attempt`, Codex `attempt`) | | adapt: `lablet.attempt` (already in spec) |
| response size | int | chat | code | | adapt: `lablet.response.bytes` |
| `gen_ai.response.model` mismatch, `model_fallbacks` | | | eval (Inspect) | Not applicable | skip |

### 2.4 Usage

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `gen_ai.usage.input_tokens` | int | chat, root aggregate | std; all | std: **includes** cached tokens. Inspect `input_tokens` **excludes** them; Harbor/ATIF includes. State the convention | adopt, inclusive |
| `gen_ai.usage.output_tokens` | int | chat, root | std; all | Includes reasoning tokens | adopt |
| `gen_ai.usage.cache_read.input_tokens` | int | chat, root | std; sdk; plat; code | | adopt |
| `gen_ai.usage.cache_write.input_tokens` | int | chat, root | std | **Spec currently says `cache_creation`.** Semconv name is `cache_write`; ADK, Strands, Copilot emit `cache_creation`; Codex emits `cache_write` | adopt `cache_write`; fix spec |
| `gen_ai.usage.reasoning.output_tokens` | int | chat | std; sdk (ADK); code (Codex, Gemini `thoughts`) | Anthropic does not report separately; zero | adopt |
| total tokens | int | root | plat (Braintrust, Opik); code (Codex, Cline) | Only if equals the sum | adapt: `lablet.usage.total_tokens` (in spec) |
| root aggregate under a distinct namespace | int | root | sdk (Pydantic `gen_ai.aggregated_usage.*`); Strands double counts by reusing `gen_ai.usage.*` | Platforms sum `gen_ai.usage.*` across spans | **decide**: reuse `gen_ai.usage.*` on root (Honeycomb, Datadog expect) or `lablet.usage.*` |
| tokens billed on failed attempts | int | root | eval (recommendation; no framework has it) | Lets evaluators subtract retry cost | adapt: `lablet.usage.failed_attempt_tokens` |
| per-model usage map | map | root | sdk (Claude `model_usage`); eval (Inspect `model_usage`, Harbor) | One model per lablet run | skip |
| `gen_ai.usage.{text,image,audio}.*` | int | chat | std | Modality splits | skip |
| context window size | int | chat | sdk (Claude `contextWindow`); code (Codex `context_window`, OpenHands) | Lets analysis compute context fill ratio | adapt: `lablet.model.context_window` from config |

### 2.5 Cost

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `gen_ai.usage.cost` | double USD | chat, root | plat (Langfuse, Opik read it) | Not in std, but the de facto override key | adopt alongside `lablet.run.cost_usd` |
| integer micro-USD | int | root | code (Codex `cost_microusd`) | Avoids float counters | later |
| pricing table version | string | root | eval (tau2 `pricing_version`) | Makes cost reproducible | adapt: `lablet.pricing.version` |
| cost from gateway header | double | chat | code (OpenHands `x-litellm-response-cost`) | | skip |

### 2.6 Tool execution

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `gen_ai.tool.name` | string | tool | std (Req); all | Raw name | adopt |
| `gen_ai.tool.type` | string | tool | std (Req); code (Copilot) | `function` for builtin, `extension` for MCP | adopt |
| `gen_ai.tool.description` | string | tool | std (Rec) | | adopt |
| `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result` | JSON | tool | std (OptIn); sdk; plat (Datadog input/output); code | Gated | adopt, gated |
| tool source | enum | tool | code (Codex `tool_origin`, Gemini `tool_type`, Claude `mcp_server_scope`) | `builtin`, `mcp` | adapt: `lablet.tool.source` (in spec) |
| MCP server name | string | tool | code (Codex `mcp_server`, Gemini `mcp_server_name`) | | adapt: `lablet.mcp.server` |
| success / is_error | bool | tool | every code agent; sdk (Strands `tool.status`) | | adapt: `lablet.tool.is_error` (in spec) |
| input and output sizes | int | tool | code (Claude `tool_input_size_bytes`, `tool_result_size_bytes`; Codex `arguments_length`, `output_length`, `output_line_count`) | Emitted even when content is off | adapt: `lablet.tool.input.bytes`, `output.bytes`, `output.lines` |
| output truncated flag | bool, int | tool | code (Gemini `tool_output_truncated`, Claude `_truncated`, `_original_length`) | | adapt: `lablet.tool.output.truncated`, `original_bytes` |
| tool phase timing | histogram | tool | code (Gemini `phase`: validation, preparation, execution, result_processing) | | later |
| exit code, cmd, stdout, stderr for shell | int, str | tool | eval (Inspect `SandboxEvent{action, cmd, result, output}`); code (Claude `bash_command_class`, `bash_argv0`) | ATIF has no slot; put in extra | adapt: `lablet.bash.exit_code`, `lablet.bash.argv0`, `lablet.bash.command_class` |
| subprocess CPU time | double | tool | none | Gap all sources share | later |
| bounded-vocabulary companions | string | metric dims | code (Claude `tool_name_safe`, `error_class`) | Only these on metric dimensions | adapt if metrics are added |

### 2.7 MCP

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `mcp.method.name` | enum | tool span | std (Req) | `tools/call` | adopt |
| `mcp.session.id` | string | tool span | std (Rec); sdk (ADK) | | adopt |
| `mcp.protocol.version` | string | tool span | std (Rec); sdk (ADK) | | adopt |
| `jsonrpc.request.id` | string | tool span | std (CondReq) | | adopt |
| `rpc.response.status_code` | string | tool span | std (CondReq on error) | | adopt |
| `network.transport` | string | tool span | std (Rec) | `pipe` for stdio, `tcp` for HTTP | adopt |
| `network.protocol.name`, `.version` | string | tool span | std | `http` `2` | adopt |
| trace context in `params._meta` | | request | std (SEP-414 Final) | Unprefixed `traceparent`, `tracestate`, `baggage` | adopt |
| `mcp.client.operation.duration`, `mcp.client.session.duration` | histogram | metric | std | | later, with metrics |
| `tools/list` span at startup | span | check/build | std (method enum) | Lablet lists tools at build | adopt |

### 2.8 Run outcome

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| loop stop reason, distinct from model finish reason | enum | root | sdk (Claude `terminal_reason` vs `stop_reason`); code (Gemini `terminate_reason`); eval (tau2 `termination_reason`, Inspect `limit.type`) | Closed enum; SWE-agent leaks exception names, do not copy | adapt: `lablet.run.stop_reason` (in spec) |
| limit split from error | struct | root | eval (Inspect `EvalSample.limit{type, limit, usage}` vs `error`) | A limit is a normal exit, still scored; an error is not | adapt: `lablet.run.limit.type`, `.limit`, `.usage` beside `stop_reason` |
| error type, message, traceback | string | root | eval (Inspect `EvalError`, Harbor `ExceptionInfo{type, message, traceback, occurred_at}`) | | adapt: `lablet.run.error.type`, `.message` |
| interrupted, and what was in flight | enum | root | eval (Inspect `InterruptEvent{source, interrupted: generate/tool_call/between_turns}`); code (Codex `turn.interrupted`) | | adapt: `lablet.run.interrupted_during` |
| turns | int | root | sdk (Claude `num_turns`); code (Gemini `turn_count`, Codex `turn.count`); eval (Inspect `turn_count`) | | adopt as `lablet.run.turns` (in spec) |
| message count | int | root | eval (Inspect `message_count`) | Distinct from turns | adapt: `lablet.run.messages` |
| provider calls, retries | int | root | eval (Inspect `ModelEvent.retries`, SWE-agent `api_calls`); std metric `invoke_agent.inference_calls` | | in spec |
| tool calls, errors | int | root | std metric `invoke_agent.tool_calls`; sdk (Strands per-tool stats); code | | in spec |
| total time vs working time | double s | root | eval (Inspect `total_time`, `working_time`; every event has `working_start`) | Working time excludes retries and waits. Only Inspect models it | adapt: `lablet.run.working_ms` beside `duration_ms` |
| time split: LLM wait, tool exec, loop overhead | double | root | code (recommendation); sdk (Claude `duration_api_ms` vs `duration_ms`) | | adapt: `lablet.run.provider_ms`, `.tool_ms`, `.overhead_ms` |
| phase timings | struct | root | eval (Harbor `environment_setup`, `agent_setup`, `agent_execution`, `verifier`) | Lablet only has agent execution; expose build vs run | adapt: `lablet.run.setup_ms` |
| submission / final answer | string | outcome | eval (SWE-agent `submission`, Vivaria `submission`, GAIA `model_answer`) | `result.text` and `result.structured` | in spec |
| scores map | map | outcome | eval (Inspect `scores{name: Score}`, Harbor `rewards`, tau2 `reward_breakdown`); std `gen_ai.evaluation.result` | Lablet does not grade; leave a nullable slot the composer fills | adapt: `scores: null` in outcome |
| `gen_ai.evaluation.result` log record | log | | std; plat (Honeycomb) | Grader lablets emit this | later, for grader mode |
| context fill at end | double | root | derived | tokens of last request over context window | adapt: `lablet.run.context_fill_ratio` |
| cache hit rate | double | root | code (OpenHands derived) | cache_read over input | adapt: `lablet.run.cache_hit_ratio` |
| tokens per turn, tool calls per turn, tool failure rate | double | root | code (recommendation) | Cheap to derive in analysis; do not emit | skip |
| productivity: lines, commits, PRs, acceptance | counters | | code (all) | Human acceptance measures | skip |

### 2.9 Environment and resource

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `service.name`, `service.version`, `service.instance.id` | string | resource | std (Req); plat (Datadog `ml_app`) | | adopt |
| `deployment.environment.name` | string | resource | std; plat (Langfuse environment) | | adopt from config |
| `telemetry.sdk.*` | string | resource | std (Req) | Set by the SDK | adopt |
| `host.name`, `host.arch` | string | resource | std | | adopt |
| `os.type`, `os.version` | string | resource | std | | adopt |
| `container.id` | string | resource | std (Stable) | Detected from cgroup when present | adopt |
| `process.pid`, `process.command_args`, `process.working_directory`, `process.runtime.*` | mixed | resource | std | | adopt |
| git revision, branch, dirty flag of the target repo | string, bool | resource | std registry `vcs.ref.head.{revision,name,type}` (no resource group); eval (Inspect `EvalRevision{origin, commit, dirty}`, tau2 `git_commit`); sdk (LangSmith `revision_id`); code (Copilot `git.commit_sha`) | Only Inspect has dirty flag | adapt: `vcs.ref.head.revision`, `vcs.ref.head.name` as resource plus `lablet.vcs.dirty` |
| `vcs.repository.url.full` | string | resource | std registry; code (Claude opt-in) | | adopt, opt-in |
| sandbox / container spec | struct | root | eval (Inspect `SandboxEnvironmentSpec`, Harbor `EnvironmentConfig{cpus, memory_mb, …}`) | Declared, not measured; the composer knows it | adapt: passthrough via `telemetry.resource` |
| lablet config digest | string | root | eval (Harbor `task_checksum`, `config` dump); code (OpenHands `conversation_ref`) | | in spec |
| tool and dependency versions | map | resource | eval (Inspect `packages`, SWE-agent `swe_agent_hash`); code (OpenHands `RuntimeProperties`) | rmcp version, MCP server versions from `initialize` | adapt: `lablet.mcp.server.version.<name>` |
| MCP server config summary | | root | code (Gemini `mcp_servers_count`, `mcp_tools_count`; Codex `mcp_server_count`) | | in spec (`lablet.tools.count`, `lablet.mcp.servers`) |
| process memory and CPU | gauge | metric | code (Gemini `memory.usage`, `cpu.usage`; sdk LangSmith `mem.rss`, `cpu.time`) | Not comparable across hosts; every eval framework skips it | skip |
| seed | int | chat, root | std `gen_ai.request.seed`; eval (tau2 `seed`) | | adopt |
| model version string | string | root | code (recommendation) | Anthropic model ids are versioned; response model covers it | skip |

### 2.10 Content and privacy

| Name | Type | On | Who | Notes | Rec |
| --- | --- | --- | --- | --- | --- |
| `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.system_instructions` | JSON (published schema) | chat span or `gen_ai.client.inference.operation.details` log record | std (OptIn); sdk; plat (Datadog, Weave, Braintrust read as attributes; Honeycomb prefers events); code (Gemini, Copilot) | Roles `system`, `user`, `assistant`, `tool`; parts `text`, `tool_call`, `tool_call_response`, `reasoning` | adopt, gated |
| content switch | env | | std example `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT`; code (Claude five switches, Codex, Copilot) | One switch | adopt, `telemetry.capture_content` plus honour the env name |
| placeholder when off | string | | sdk (ADK writes `{}` or `<elided>`) | Dashboards keep working | adapt: emit `[redacted]` or omit; **decide** |
| length always, content gated | int | all | code (Claude, Codex) | | adopt (bytes fields above) |
| content length cap with sidecars | int | | code (Claude `CLAUDE_CODE_OTEL_CONTENT_MAX_LENGTH`, `_truncated`, `_original_length`; Mastra export truncation) | | adapt: `telemetry.content_max_bytes` |
| raw API bodies | file | | code (Claude `OTEL_LOG_RAW_API_BODIES`); eval (Inspect `log_model_api`) | Debugging provider adapters | later |
| secret redaction on config export | | | eval (Harbor `templatize_sensitive_env`) | Digest before env substitution covers it | in spec |
| `openinference.span.kind`, `input.value`, `output.value` | string | all spans | plat (Phoenix requires; Langfuse, Weave read) | Small duplicate set for Phoenix rendering | later, optional |
| `langfuse.observation.type` | string | all spans | plat (Langfuse) | One attribute, no conflicts | later, optional |

## 3. Metrics

Lablet's spec emits traces and logs only. These are what a metric set would contain if one is added; every platform surveyed derives its LLM views from spans, so metrics only matter for generic dashboards.

| Name | Instrument | Unit | Dimensions | Who | Rec |
| --- | --- | --- | --- | --- | --- |
| `gen_ai.client.token.usage` | histogram | `{token}` | `gen_ai.token.type`, provider, operation, model | std; sdk (Pydantic); plat (Traceloop); code (Gemini, Copilot) | later |
| `gen_ai.client.operation.duration` | histogram | s | operation, provider, model, `error.type` | std; sdk; code | later |
| `gen_ai.invoke_agent.duration`, `.inference_calls`, `.tool_calls` | histogram | s, count | `gen_ai.agent.name`, `error.type` | std; sdk (ADK) | later |
| `gen_ai.execute_tool.duration` | histogram | s | `gen_ai.tool.name`, `gen_ai.tool.type`, `error.type` | std | later |
| `mcp.client.operation.duration` | histogram | s | `mcp.method.name`, `error.type` | std | later |
| cost | counter | USD or µUSD | model | sdk (Pydantic `operation.cost`); code (Claude `cost.usage`, Codex `cost_microusd`) | later |
| retries, API errors | counter | 1 | status, attempt | code (Codex `api_request`, Gemini `api.request.count`) | later |
| `gen_ai.server.*`, streaming TTFT/TBT | histogram | s | | std; code (Codex) | skip |
| session count, active time, process memory, productivity | various | | | code | skip |

## 4. Events and log records

| Name | Carried on | Fields | Who | Rec |
| --- | --- | --- | --- | --- |
| `gen_ai.client.inference.operation.details` | log record in the chat span context | full inference attribute set plus content | std; sdk (Strands, ADK); code (Gemini, Copilot) | adopt, gated (in spec) |
| `gen_ai.client.operation.exception` | log record, WARN | `exception.type`, `exception.message`, `exception.stacktrace`, gen_ai attrs | std | adopt for provider failures |
| retry event | span event on chat | `attempt`, `will_retry`, `backoff` | code (Claude `api_error`) | adapt: keep as span event plus the exception log record |
| `exception` span event | span event | | std, **deprecated** in favour of log records | skip |
| wide event / run summary record | log record | everything in §2.8 | code (Gemini `agent.finish`, Cline `task.completed`, OpenHands outcome event); Honeycomb canonical log line | in spec (`lablet.run`) |
| run started / config record | log record | model, tools, MCP servers, sandbox, switches | code (Gemini `config`, Codex `conversation_starts`) | adapt: `RunStarted` already carries it; consider a `lablet.run.started` log record |
| tool result record | log record | name, id, success, duration, sizes, error, gated args and output | code (Claude `tool_result`, Codex `tool_result`, Gemini `tool_call`) | adapt: the tool span covers it; a log record only if a consumer needs logs without traces |
| tool output truncated | log record | original and truncated length, threshold | code (Gemini) | adapt as attributes on the tool span |
| `gen_ai.evaluation.result` | log record | name, score value, label, explanation | std; plat | later, grader mode |
| `session.start`, `session.end` | log record | `session.id` | std | skip, run root covers it |
| Inspect transcript events | typed events | `ModelEvent`, `ToolEvent`, `SandboxEvent`, `SampleLimitEvent`, `InterruptEvent`, `ErrorEvent`, `CompactionEvent` | eval | adapt: lablet's `RunEvent` maps onto these; see §9 |
| ATIF `Step` | trajectory document | one per inference: `tool_calls[]`, `observation.results[]` with `source_call_id`, `metrics`, `llm_call_count` | eval (Harbor, Terminal-bench) | adapt: transcript export to ATIF v1.8 |

## 5. Per-run summary shapes worth copying

| Source | Shape | Why it matters |
| --- | --- | --- |
| Claude Agent SDK `ResultMessage` | `num_turns`, `duration_ms`, `duration_api_ms`, `total_cost_usd`, `is_error`, `stop_reason` vs `terminal_reason`, `usage` with cache buckets, `structured_output` | Richest single run summary; distinguishes model stop from loop stop, and API time from wall time |
| Inspect `EvalSample` | `total_time`, `working_time`, `turn_count`, `message_count`, `limit{type, limit, usage}`, `error`, `error_retries[]`, `model_usage`, `scores` | Two clocks, limit versus error, retries kept with their events |
| Harbor `TrialResult` and `AgentContext` | `n_input_tokens` (incl. cache), `n_cache_tokens`, `n_output_tokens`, `cost_usd`, `exception_info`, phase `TimingInfo`, `verifier_result.rewards` | Outcome separated from trajectory; the composer's contract |
| OpenHands `Metrics` | `accumulated_cost`, per-call `costs[]`, `response_latencies[]`, `token_usages[]`, derived `cache_hit_rate`, `diff(baseline)` | Per-call ledgers plus derived ratios |
| tau2 `SimulationRun` | `termination_reason` enum, `seed`, `pricing_version`, `agent_usage` with `cost_breakdown` | Reproducibility fields on the run |
| Vivaria `TraceEntry` | cumulative `usageTokens`, `usageActions`, `usageTotalSeconds`, `usageCost` on every entry | Running totals on every event make limits observable mid-run |

## 6. Cross-cutting patterns

- **Naming eras.** `gen_ai.system` and `prompt_tokens`/`completion_tokens` are deprecated but still emitted by Strands, ADK, Traceloop and accepted by Langfuse, Braintrust, Weave, Opik. Datadog requires the new names. Emit only the new names.
- **Cache accounting has three conventions.** Semconv, Harbor and ATIF: `input_tokens` includes cached. Inspect: excludes. Langfuse: mutually exclusive buckets. Braintrust: `prompt_tokens` must include cached. Whatever lablet picks must be written down on the outcome and the wide event.
- **Root-span usage double counting.** Platforms sum `gen_ai.usage.*` across all spans. Pydantic renames root totals to `gen_ai.aggregated_usage.*`; Mastra omits them; Strands double counts. Honeycomb and Datadog read totals from the root.
- **Span kind inference diverges.** Datadog, Braintrust, Honeycomb key on `gen_ai.operation.name`; Phoenix needs `openinference.span.kind`; Langfuse defaults to `span` unless `langfuse.observation.type` or a model attribute is present.
- **Cost.** No cost attribute in semconv. Every platform computes it from model plus tokens, and every one accepts an override (`gen_ai.usage.cost` in Langfuse and Opik, `llm.cost.*`, `estimated_cost`). Client-side cost exists in Claude SDK, Pydantic, Claude Code, Codex, OpenHands, Cline, Aider.
- **Content.** Opt-in everywhere except Gemini. Best practice: one switch, lengths always, a size cap with truncation sidecars, structure kept when redacted.
- **Cardinality.** Claude Code's `*_safe` companions and `OTEL_METRICS_INCLUDE_*` flags, Codex's bounded `originator`, ADK keying metrics by agent and tool name only. Strands' `tool_use_id` on metrics is the anti-pattern.
- **Correlation layering.** Claude Code `session.id > prompt.id > message.uuid > tool_use_id` plus `event.sequence`; OpenHands `llm_response_id`, `action_id`, `parent_id`; Inspect `run_id > sample uuid > event uuid / span_id`.
- **Exceptions are moving to log records.** The `exception` span event is deprecated in core semconv; GenAI defines `gen_ai.client.operation.exception`.
- **Retries live inside one span.** Semconv, Inspect (`ModelEvent.retries`), Claude Code (`attempt` on `api_error`).
- **Two clocks.** Only Inspect separates working time from wall time, and it stamps `working_start` on every event.
- **Outcome separated from trajectory.** Harbor `result.json` versus `trajectory.json`; OpenHands events versus `Metrics`. Lablet's outcome JSON and transcript already follow this.
- **Nobody measures** subprocess CPU, lockfile hashes, or sandbox ids. Git revision plus dirty flag appears only in Inspect and, without dirty, in tau2, LangSmith, Copilot.

## 7. Semconv facts that correct the current spec

| Spec says | Research says | Where |
| --- | --- | --- |
| `gen_ai.usage.cache_creation.input_tokens` | Semconv name is `gen_ai.usage.cache_write.input_tokens`; `cache_creation` is Anthropic's field name, copied by ADK, Strands, Copilot | standards.md §Attributes |
| `gen_ai.usage.input_tokens` unstated whether cache is included | Semconv: includes cached tokens; report billed counts | standards.md |
| ProviderCallFailed as a span event with `exception.message` | Core semconv deprecates the `exception` span event; GenAI defines `gen_ai.client.operation.exception` log record | standards.md §Events |
| `gen_ai.request.reasoning.level` absent | Exists; Anthropic `effort` maps to it | standards.md |
| Registry imports "upstream semconv" | GenAI moved to `semantic-conventions-genai`, Weaver v2 syntax, everything Development; core stays at `v1.44.0` | standards.md (already in spec) |
| No MCP attributes on tool spans | Semconv wants `mcp.*`, `jsonrpc.*`, `network.transport` on the same `execute_tool` span, and `traceparent` in `params._meta` | standards.md §MCP |
| `gen_ai.tool.type` present | Values are `function`, `extension`, `datastore` | standards.md |
| `lablet.seed` in the wide event | `gen_ai.request.seed` exists | standards.md |
| `gen_ai.conversation.id` = run id | Allowed if it is a real id lablet mints; also emit `session.id` for Langfuse and Phoenix | standards.md, observability-platforms.md |
| Weaver templates under `templates/rust/` | Weaver expects `templates/registry/<target>/` | standards.md (already in spec) |

## 8. Gaps no source fills, where lablet can lead

- Working time versus wall time on a per-event basis outside Inspect.
- Tool subprocess CPU and wall time, and the LLM-wait / tool-exec / loop-overhead split of a run.
- Tokens billed on failed attempts as a separate counter.
- Git dirty flag and lockfile hash of the target repository as resource attributes.
- A closed-enum stop reason that separates limits from errors and is stable across providers.
- Transcript that round-trips to ATIF v1.8 and to Inspect events from one source of truth.

## 9. Recommendation summary (opinion)

1. **Attribute names.** Follow semconv-genai verbatim for everything it names, including `cache_write`, `gen_ai.request.seed`, `gen_ai.request.reasoning.level`, `gen_ai.tool.type`, and the `mcp.*` set on tool spans. Emit only the current names, never the deprecated dual set.
2. **Correlation.** `gen_ai.conversation.id` and `session.id` both equal the run id on every span; `gen_ai.agent.name` on every span; `gen_ai.tool.call.id` and a response id on every tool span; a monotonic `lablet.event.sequence` on every event.
3. **Root totals.** Put `gen_ai.usage.*` totals on the root span as Honeycomb and Datadog expect, and note in the docs that a naive sum across all spans double counts. Reserve `lablet.usage.total_tokens` for the inclusive total.
4. **Outcome shape.** Extend the outcome and wide event with Inspect's two clocks (`working_ms`), a `limit` structure beside `stop_reason`, `error.type` and `error.message`, `interrupted_during`, `messages`, and a nullable `scores` map. Add `failed_attempt_tokens`, `cache_hit_ratio`, `context_fill_ratio`, and the provider / tool / overhead time split.
5. **Sizes always, content gated.** Bytes and line counts for prompt, response, tool input, and tool output unconditionally. Content behind one switch with a byte cap and `truncated` / `original_bytes` sidecars.
6. **Exceptions as log records.** Keep the retry span event for the trace view but emit `gen_ai.client.operation.exception` log records for provider failures.
7. **Environment.** Resource attributes for service, host, os, process, container, plus `vcs.ref.head.revision`, `vcs.ref.head.name`, and a `lablet.vcs.dirty` flag detected from the working directory. Skip measured CPU and memory.
8. **Transcript formats.** Make the transcript exportable to ATIF v1.8 (one turn is one `Step`) and keep the event model mappable to Inspect's `ModelEvent`, `ToolEvent`, `SandboxEvent`, `SampleLimitEvent`, `ErrorEvent`.
9. **Metrics.** Not for the first build. If added, only the semconv set with bounded dimensions.
10. **Platform duplicates.** Offer `openinference.span.kind` and `langfuse.observation.type` as an opt-in `telemetry.compat` list rather than always emitting them.

## 10. Decisions taken (2026-09-18)

Rule applied throughout: follow the semantic conventions where they speak, then whatever is idiomatic, and start lean.

- Root-span totals reuse `gen_ai.usage.*`; the double-count caveat is documented.
- `input_tokens` in the outcome JSON includes cached tokens, matching semconv.
- No turn span; `lablet.turn` index on chat and tool spans. A turn span can be added later.
- Redacted content is omitted, not written as a placeholder; byte counts are always present.
- ATIF export lands in the features phase; the transcript model is designed for it in the domain phase.

## 11. Extensions deferred

Kept here so they are not lost. None is in the spec or the registry yet; each needs a justification when added.

`lablet.usage.failed_attempt_tokens`, `lablet.run.working_ms` and per-event working time, `lablet.run.provider_ms` / `tool_ms` / `overhead_ms`, `lablet.run.cache_hit_ratio`, `lablet.run.context_fill_ratio`, `lablet.model.context_window`, `lablet.run.limit.*` split from error, `lablet.run.interrupted_during`, `lablet.run.messages`, `lablet.event.sequence`, `lablet.response.id` on tool spans, `lablet.tool.output.lines`, `lablet.tool.output.truncated`, `lablet.bash.*`, `lablet.mcp.server.version.<name>`, `lablet.vcs.dirty`, `lablet.pricing.version`, `gen_ai.usage.cost`, `openinference.span.kind` and `langfuse.observation.type` compat, metrics.
