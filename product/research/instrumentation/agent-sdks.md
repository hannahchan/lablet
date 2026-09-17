# Agent SDKs

Researched: 2026-09-18. Sources are linked inline. Everything here is an inventory of what the source emits or expects; recommendations for lablet are confined to the final section and marked as opinion.

## Sources covered

- **Claude Agent SDK** (Python 0.2.155 / TS bundling Claude Code 2.1.x; repo head `aab2797`, 2026-09-17). [Python reference](https://code.claude.com/docs/en/agent-sdk/python), [TypeScript reference](https://code.claude.com/docs/en/agent-sdk/typescript), [Hooks](https://code.claude.com/docs/en/agent-sdk/hooks), [`types.py`](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/types.py). No built-in OTel; exposes structured messages and hooks.
- **OpenAI Agents SDK** (Python; repo head `fdf21db`, 2026-09-17). [Tracing guide](https://openai.github.io/openai-agents-python/tracing/), [`span_data.py`](https://github.com/openai/openai-agents-python/blob/main/src/agents/tracing/span_data.py), [`spans.py`](https://github.com/openai/openai-agents-python/blob/main/src/agents/tracing/spans.py), [`processors.py`](https://github.com/openai/openai-agents-python/blob/main/src/agents/tracing/processors.py). OTel bridge is external: [`opentelemetry-instrumentation-openai-agents-v2`](https://github.com/open-telemetry/opentelemetry-python-contrib/blob/main/instrumentation-genai/opentelemetry-instrumentation-openai-agents-v2/README.rst) (marked deprecated in favour of `opentelemetry-instrumentation-genai-openai-agents`).
- **Google ADK** (Python; repo head `4ff3cdc`, 2026-09-17). [`telemetry/tracing.py`](https://github.com/google/adk-python/blob/main/src/google/adk/telemetry/tracing.py), [`_token_usage.py`](https://github.com/google/adk-python/blob/main/src/google/adk/telemetry/_token_usage.py), [`_metrics.py`](https://github.com/google/adk-python/blob/main/src/google/adk/telemetry/_metrics.py), [`_stable_semconv.py`](https://github.com/google/adk-python/blob/main/src/google/adk/telemetry/_stable_semconv.py), [`_adk_attributes.py`](https://github.com/google/adk-python/blob/main/src/google/adk/telemetry/_adk_attributes.py), [Cloud Trace guide](https://adk.dev/integrations/cloud-trace/).
- **Vercel AI SDK** (v7 docs; repo head `1284569`, 2026-09-17). [Telemetry](https://ai-sdk.dev/docs/ai-sdk-core/telemetry). Two layers: legacy `ai.*` spans and the `@ai-sdk/otel` `OpenTelemetry` integration emitting `gen_ai.*`.
- **LangSmith / LangGraph** (langsmith-sdk head `e4370d6`, 2026-09-17). [Run data format](https://docs.langchain.com/langsmith/run-data-format), [Log LLM trace](https://docs.langchain.com/langsmith/log-llm-trace), [Cost tracking](https://docs.langchain.com/langsmith/cost-tracking), [Concepts](https://docs.langchain.com/langsmith/observability-concepts), [`env/_runtime_env.py`](https://github.com/langchain-ai/langsmith-sdk/blob/main/python/langsmith/env/_runtime_env.py). Not OTel-native; a run-tree JSON model.
- **Pydantic AI + Logfire** (repo head `450fdd7`, 2026-09-17; instrumentation `version` 2–6, default 5; GenAI semconv v1.37.0). [Logfire integration](https://pydantic.dev/docs/ai/integrations/logfire/), [`models/instrumented.py`](https://github.com/pydantic/pydantic-ai/blob/main/pydantic_ai_slim/pydantic_ai/models/instrumented.py), [`_instrumentation.py`](https://github.com/pydantic/pydantic-ai/blob/main/pydantic_ai_slim/pydantic_ai/_instrumentation.py).
- **Mastra** (repo head `95f1ad9`, 2026-09-17; OTel exporter targets GenAI semconv v1.38.0). [Tracing overview](https://mastra.ai/docs/observability/tracing/overview), [OTel exporter](https://mastra.ai/docs/observability/tracing/exporters/otel), [`observability/types/tracing.ts`](https://github.com/mastra-ai/mastra/blob/main/packages/core/src/observability/types/tracing.ts).
- **Strands Agents** (AWS; repo head `98b4977`, 2026-09-17). [Traces](https://strandsagents.com/docs/user-guide/observability-evaluation/traces/), [Metrics](https://strandsagents.com/docs/user-guide/observability-evaluation/metrics/), [`telemetry/tracer.py`](https://github.com/strands-agents/sdk-python/blob/main/strands-py/src/strands/telemetry/tracer.py), [`metrics.py`](https://github.com/strands-agents/sdk-python/blob/main/strands-py/src/strands/telemetry/metrics.py), [`metrics_constants.py`](https://github.com/strands-agents/sdk-python/blob/main/strands-py/src/strands/telemetry/metrics_constants.py).

## Span structure

| SDK | Root | Turn / step | LLM call | Tool call | Other |
|---|---|---|---|---|---|
| Claude Agent SDK | No spans. One `session_id` per `query()`; a `ResultMessage` closes the run. | `AssistantMessage` (one per model response, `num_turns` counts tool-use round trips) | Same as turn; `usage` on `AssistantMessage` | `ToolUseBlock`/`ToolResultBlock` inside messages; `PreToolUse`/`PostToolUse`/`PostToolUseFailure` hooks keyed by `tool_use_id` | `SubagentStart`/`SubagentStop` with `agent_id`, `agent_type`; `PreCompact`; `SDKCompactBoundaryMessage` |
| OpenAI Agents | `Trace` (`workflow_name`, `trace_id`, `group_id`, `metadata`) | `turn` span (`type: "custom"`, `data.sdk_span_type="turn"`, `turn`, `agent_name`, `usage`) inside `agent` span; `task` span wraps a runner invocation | `generation` (`input`, `output`, `model`, `model_config`, `usage`) or `response` (`response_id`, `usage`) | `function` (`name`, `input`, `output`, `mcp_data`) | `handoff` (`from_agent`, `to_agent`), `guardrail` (`name`, `triggered`), `mcp_tools` (`server`, `result`), `custom`, speech/transcription |
| Google ADK | `invoke_agent` span per agent (`gen_ai.operation.name=invoke_agent`); nested per sub-agent. Runner-level `invocation`/`agent_run` names in the legacy layer | No explicit turn span; one `generate_content {model}` per model call | `generate_content {model}` (a.k.a. `call_llm` in older docs) | `execute_tool {name}` (`gen_ai.operation.name=execute_tool`); merged parallel tool responses become one `execute_tool` with `gen_ai.tool.name="(merged tools)"` | `send_data` for injected content; compaction attributes `gen_ai.compaction.*`; MCP HTTP exchange log records |
| Vercel AI SDK (legacy) | `ai.generateText` / `ai.streamText` (whole multi-step call) | `ai.generateText.doGenerate` / `ai.streamText.doStream` (one per provider call, i.e. per step) | Same as step | `ai.toolCall` | `ai.embed*`, deprecated `ai.generateObject*` |
| Vercel AI SDK (`@ai-sdk/otel`) | `invoke_agent {modelId}` (INTERNAL) | `chat {modelId}` (CLIENT) per step | Same as step | `execute_tool {toolName}` (INTERNAL) | — |
| LangSmith | Root run (`parent_run_id=null`); `trace_id` = root run id. LangGraph graph is a `chain` run; each node a child `chain` | Node runs (`chain`) | `run_type="llm"` run | `run_type="tool"` run | `retriever`, `embedding`, `prompt`, `parser`; ordering via `dotted_order` |
| Pydantic AI | `agent run` (v≤2) / `invoke_agent {agent}` (v≥3); `gen_ai.operation.name=invoke_agent` | No turn span; model spans and tool spans are direct children | `chat {model}` (span name `f'{operation} {model_name}'`) | `running tool` / `running N tools` (v≤2) or `execute_tool {tool}` (v≥3) | `running output function`; `gen_ai.agent.call.id` and `gen_ai.conversation.id` propagated as OTel baggage |
| Mastra | `AGENT_RUN` (`isRootSpan`) → OTel `invoke_agent {agent_id}` | `MODEL_STEP` (`stepIndex`) → `agent_step {agent_id}`; `MODEL_GENERATION` → `model_generation {model}` | `MODEL_INFERENCE` → `chat {model}` (the only span carrying model/usage in OTel export) | `TOOL_CALL`, `MCP_TOOL_CALL`, `CLIENT_TOOL_CALL`, `PROVIDER_TOOL_CALL` → `execute_tool {tool}` | `WORKFLOW_*`, `PROCESSOR_RUN`, `MEMORY_OPERATION`, `RAG_*`, `MODEL_CHUNK` (event) |
| Strands | `invoke_agent {agent_name}` (INTERNAL) | `execute_event_loop_cycle` (`event_loop.cycle_id`, `event_loop.parent_cycle_id`) | `chat` (INTERNAL) | `execute_tool {name}` | `memory.search`/`memory.add`/`memory.inject`/`memory.extract`; multi-agent spans |

Span events vs spans: Strands, ADK, Vercel and Pydantic AI put message content on span events (or log records) and metadata on span attributes; OpenAI Agents and Mastra put content in `span_data`/`input`/`output` fields on the span itself; LangSmith stores `inputs`/`outputs` on the run and uses `events` only for streaming markers (`new_token`).

## Attributes

Grouped by namespace. "Set on" uses the span kinds from the table above.

### `gen_ai.*` (OTel GenAI semconv, as emitted)

| name | type | set on | req | description | source(s) |
|---|---|---|---|---|---|
| `gen_ai.operation.name` | string | all spans | opt | `invoke_agent`, `chat`, `execute_tool`, `execute_event_loop_cycle` (Strands-specific value) | ADK tracing.py; Pydantic; Mastra OTel; Strands tracer.py; Vercel otel |
| `gen_ai.system` | string | all | opt | Legacy provider id: ADK `"gcp.vertex.agent"`, Strands `"strands-agents"`, Vercel/Pydantic provider | ADK; Strands; Vercel; Pydantic |
| `gen_ai.provider.name` | string | all | opt | Replaces `gen_ai.system` under `OTEL_SEMCONV_STABILITY_OPT_IN=gen_ai_latest_experimental` (Strands) or v≥3 (Pydantic); Mastra always | Strands `_get_common_attributes`; Pydantic; Mastra |
| `gen_ai.agent.name` / `.description` / `.version` | string | root | opt | ADK sets name+description on `invoke_agent`; `gen_ai.agent.version` is a metric dim | ADK tracing.py, _metrics.py; Pydantic; Strands; Vercel otel |
| `gen_ai.agent.call.id` | string | baggage/root | opt | Pydantic `run_id` (UUID7) propagated as baggage | Pydantic `_instrumentation.py` |
| `gen_ai.conversation.id` | string | root | opt | ADK `session.id`; Pydantic `conversation_id`; Mastra `conversationId` | ADK; Pydantic; Mastra |
| `gen_ai.request.model` | string | llm | opt | | all except Claude/LangSmith |
| `gen_ai.request.max_tokens` / `.temperature` / `.top_p` / `.top_k` / `.frequency_penalty` / `.presence_penalty` / `.stop_sequences` | number/array | llm | opt | Vercel lists all; ADK sets `top_p`, `max_tokens`; Mastra `parameters` incl. `seed`, `maxRetries` | Vercel; ADK; Mastra |
| `gen_ai.response.model` / `.id` / `.finish_reasons` | string / string[] | llm | opt | `finish_reasons` is an array | Vercel; ADK; Pydantic; Mastra |
| `gen_ai.usage.input_tokens` / `.output_tokens` | int | llm (and root, see summary) | opt | Universal | all OTel SDKs |
| `gen_ai.usage.cache_read.input_tokens` / `.cache_creation.input_tokens` | int | llm, root | opt | Current names in ADK and Strands; Strands also dual-emits deprecated `gen_ai.usage.cache_read_input_tokens` / `cache_write_input_tokens` unless opted into latest | ADK `_token_usage.py`; Strands tracer.py |
| `gen_ai.usage.reasoning.output_tokens` | int | llm | opt | ADK | ADK `_token_usage.py` |
| `gen_ai.usage.experimental.system_instruction_tokens` / `.reasoning_tokens_limit` | int | llm | opt | ADK experimental | ADK |
| `gen_ai.usage.prompt_tokens` / `.completion_tokens` / `.total_tokens` | int | root, llm | opt | Strands legacy names, dual-emitted with `input/output_tokens` | Strands tracer.py |
| `gen_ai.aggregated_usage.*` | int | root | opt | Pydantic AI remaps root-span usage to this namespace (`use_aggregated_usage_attribute_names=True`) so backends don't double count | Pydantic instrumented.py |
| `gen_ai.input.messages` / `.output.messages` / `.system_instructions` | JSON string | llm, root | opt | Content; on span (Pydantic, Mastra) or in `gen_ai.client.inference.operation.details` event (Strands) | Pydantic; Mastra; Strands |
| `gen_ai.tool.name` / `.call.id` / `.description` / `.type` | string | tool | opt | ADK sets `type` = tool class name | ADK; Pydantic; Strands; Vercel otel; Mastra |
| `gen_ai.tool.call.arguments` / `.call.result` | JSON string | tool | opt | Pydantic v≥3, Strands latest, Mastra | Pydantic; Strands; Mastra |
| `gen_ai.tool.definitions` / `gen_ai.agent.tools` | JSON string | root, llm | opt | Strands (`gen_ai_tool_definitions` opt-in); Pydantic | Strands; Pydantic |
| `gen_ai.tool.status` | string | tool | opt | Strands latest; legacy `tool.status` | Strands |
| `gen_ai.event.start_time` / `.end_time` | string | root, tool, llm | opt | Strands legacy | Strands docs |
| `gen_ai.server.request.duration` / `.time_to_first_token` | number | llm | opt | Strands | Strands tracer.py |
| `gen_ai.compaction.trigger` / `.summarizer_type` / `.event_count` / `.token_threshold` / `.event_retention_size` / `.compaction_interval` / `.overlap_size` / `.result_event_id` / `.start_timestamp` / `.end_timestamp` | mixed | compaction span | opt | ADK context compaction | ADK tracing.py |
| `gen_ai.workflow.name` / `.nested` | string/bool | metric dims | opt | ADK workflow agents | ADK `_metrics.py` |

### Vendor namespaces

| name | type | set on | req | description | source(s) |
|---|---|---|---|---|---|
| `ai.operationId`, `ai.model.id`, `ai.model.provider`, `ai.settings.maxRetries`, `ai.settings.maxOutputTokens`, `ai.settings.maxSteps`, `ai.telemetry.functionId`, `ai.telemetry.metadata.*`, `ai.request.headers.*`, `ai.response.providerMetadata`, `operation.name`, `resource.name` | mixed | all `ai.*` spans | opt | Vercel legacy common set | Vercel |
| `ai.prompt`, `ai.prompt.messages`, `ai.prompt.tools`, `ai.prompt.toolChoice`, `ai.response.text`, `ai.response.toolCalls`, `ai.response.finishReason`, `ai.response.id`, `ai.response.model`, `ai.response.timestamp` | string | root / step | opt | Content and outcome | Vercel |
| `ai.usage.promptTokens`, `ai.usage.completionTokens` (also `inputTokens`, `outputTokens`, `cachedInputTokens`, `reasoningTokens` in v5+) | int | root / step | opt | Root span sums steps | Vercel |
| `ai.response.msToFirstChunk`, `ai.response.msToFinish`, `ai.response.avgCompletionTokensPerSecond` | number | `doStream` | opt | Streaming latency | Vercel |
| `ai.toolCall.name`, `ai.toolCall.id`, `ai.toolCall.args`, `ai.toolCall.result` | string | `ai.toolCall` | opt | | Vercel |
| `gcp.vertex.agent.invocation_id`, `.session_id`, `.event_id`, `.llm_request`, `.llm_response`, `.tool_call_args`, `.tool_response`, `.data`, `gcp.mcp.server.destination.id` | string | llm, tool, send_data | opt | ADK correlation ids and JSON payloads; payload attrs become `"{}"` when capture off | ADK tracing.py |
| `adk.experimental.context_cache.hit` / `.fingerprint` / `.contents_count` / `.invocations_used`, `adk.experimental.skill.*`, `adk.experimental.root_agent.name` | mixed | llm / skill | opt | No compatibility guarantee | ADK `_adk_attributes.py` |
| `error.type`, `http.request.method`, `http.response.status_code`, `server.address`, `server.port`, `url.full`, `mcp.protocol.version`, `mcp.session.id` | string/int | tool, MCP log | opt | ADK uses stable OTel HTTP/MCP attrs on MCP exchanges | ADK tracing.py |
| `event_loop.cycle_id`, `event_loop.parent_cycle_id`, `system_prompt`, `agent.name`, `tool.status`, `strands.cancellation.type`, `tags`, plus any `trace_attributes` passed to `Agent()` | mixed | Strands spans | opt | Legacy names remain unless opted in | Strands tracer.py |
| `agent_name`, `model_name`, `final_result`, `pydantic_ai.all_messages`, `all_messages_events`, `model_request_parameters`, `metadata`, `logfire.msg`, `logfire.json_schema`, `pydantic_ai.tool.failure_stage`, `pydantic_ai.tool.deferral.name` | mixed | root / llm / tool | opt | Pydantic/Logfire extras; `logfire.json_schema` types JSON attrs for the UI | Pydantic docs, `_instrumentation.py` |
| `agentId`, `instructions`, `prompt`, `availableTools`, `maxSteps`, `conversationId`, `tripwireAbort`, `model`, `provider`, `resultType`, `streaming`, `finishReason`, `responseModel`, `responseId`, `completionStartTime`, `serverAddress`, `serverPort`, `costContext`, `toolType`, `toolDescription`, `toolCallId`, `success`, `mcpServer`, `serverVersion`, `stepIndex`, `isContinued`, `warnings` | mixed | Mastra span `attributes` | opt | Native (pre-export) shape | Mastra `tracing.ts` |
| `ls_provider`, `ls_model_name`, `ls_model_type`, `ls_temperature`, `ls_max_tokens`, `thread_id`/`session_id`/`conversation_id`, `revision_id` | string | run `extra.metadata` | `ls_provider`+`ls_model_name` required for cost | LangSmith metadata keys | LangSmith log-llm-trace, cost-tracking |

### Claude Agent SDK message fields (not span attributes)

| field | type | on | description | source |
|---|---|---|---|---|
| `session_id`, `uuid`, `message_id`, `parent_tool_use_id` | string | all messages | Correlation; `parent_tool_use_id` links subagent output to the spawning tool call | types.py |
| `model`, `stop_reason`, `usage` (`input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`) | | `AssistantMessage` | Per model response | types.py |
| `ToolUseBlock.id/name/input`, `ToolResultBlock.tool_use_id/content/is_error` | | content blocks | Tool call and result | types.py |
| Hook inputs: `hook_event_name`, `session_id`, `cwd`, `transcript_path`, `tool_name`, `tool_input`, `tool_response`, `tool_use_id`, `error`, `is_interrupt`, `agent_id`, `agent_type`, `agent_transcript_path`, `prompt`, `trigger`, `permission_suggestions` | | hooks | Events: `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `UserPromptSubmit`, `Stop`, `SubagentStop`, `SubagentStart`, `PreCompact`, `Notification`, `PermissionRequest`; TS adds `SessionStart`, `SessionEnd`, `CwdChanged` and others | types.py; hooks doc |
| Hook outputs: `permissionDecision` (`allow`/`deny`/`ask`/`defer`), `permissionDecisionReason`, `updatedInput`, `updatedToolOutput`, `additionalContext`, `continue_`, `stopReason`, `systemMessage` | | hooks | Hooks can block, rewrite, or end the run | types.py |

## Metrics

| name | instrument | unit | dimensions | description | source(s) |
|---|---|---|---|---|---|
| `gen_ai.client.token.usage` | histogram | `{token}` | `gen_ai.token.type` (`input`/`output`), `gen_ai.provider.name`, `gen_ai.operation.name`, `gen_ai.request.model` | Pydantic AI; also OTel contrib instrumentation for OpenAI Agents | Pydantic instrumented.py; OTel contrib README |
| `operation.cost` | histogram | `{USD}` | same | Pydantic AI, from its price table | Pydantic |
| `gen_ai.client.operation.time_to_first_chunk` | histogram | `s` | same | Pydantic AI, streaming only | Pydantic |
| `gen_ai.client.operation.duration` | histogram | `s` | gen_ai.* | OTel contrib instrumentation for OpenAI Agents (`OTEL_INSTRUMENTATION_OPENAI_AGENTS_CAPTURE_METRICS`) | OTel contrib README |
| `gen_ai.invoke_agent.duration`, `gen_ai.invoke_workflow.duration`, `gen_ai.execute_tool.duration` | histogram | `s` | `gen_ai.agent.name`, `gen_ai.agent.version`, `gen_ai.tool.name`, `gen_ai.tool.version`, `gen_ai.workflow.name`, `gen_ai.workflow.nested`, `error.type` | ADK; meter name `gcp.vertex.agent` | ADK `_metrics.py` |
| `gen_ai.invoke_agent.inference_calls`, `gen_ai.invoke_agent.tool_calls` | histogram | `1` | agent dims | ADK: per-invocation counts, recorded once per run | ADK `_metrics.py` |
| `adk.experimental.invoke_agent.{input,output,total}_tokens`, `.cache_read.input_tokens`, `.reasoning.output_tokens`, `.tool.input_tokens`, same under `invoke_workflow.*`, `.skill.loads`, `adk.experimental.skill.script.executions` | histogram / counter | `{token}` / `1` | agent dims | ADK per-run token histograms | ADK `_metrics.py` |
| `strands.event_loop.cycle_count`, `.start_cycle`, `.end_cycle` | counter | `Count` | — | Strands | `metrics_constants.py`, metrics.py |
| `strands.event_loop.cycle_duration` | histogram | `s` | — | | Strands |
| `strands.event_loop.latency` | histogram | `ms` | — | Model latency | Strands |
| `strands.event_loop.input.tokens`, `.output.tokens`, `.cache_read.input.tokens`, `.cache_write.input.tokens` | histogram | `token` | — | | Strands |
| `strands.tool.call_count`, `.success_count`, `.error_count` | counter | `Count` | `tool_name`, `tool_use_id` | | Strands metrics.py |
| `strands.tool.duration` | histogram | `s` | `tool_name`, `tool_use_id` | | Strands |
| `strands.model.time_to_first_token` | histogram | `ms` | — | | Strands |

Claude Agent SDK, OpenAI Agents SDK (native), Vercel AI SDK, LangSmith and Mastra emit no OTel metrics.

## Events and log records

| name | carried on | fields | when emitted | source(s) |
|---|---|---|---|---|
| `gen_ai.system.message`, `gen_ai.user.message`, `gen_ai.assistant.message`, `gen_ai.tool.message`, `gen_ai.choice` | span event (Strands, legacy mode) / log record (ADK) | `content`/`message`, `finish_reason`, `id`, `role`; Strands cycle spans also `gen_ai.choice.message`, `gen_ai.choice.tool.result` | On span start (inputs) and end (outputs). ADK elides body to `"<elided>"` unless `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true` | Strands tracer.py; ADK `_stable_semconv.py` |
| `gen_ai.client.inference.operation.details` | span event | `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.system_instructions`, `gen_ai.tool.definitions` | Strands latest mode (`gen_ai_latest_experimental`); `gen_ai_span_attributes_only` puts the same keys on the span instead; ADK experimental path also uses this name | Strands tracer.py; ADK `_experimental_semconv.py` |
| `ai.stream.firstChunk`, `ai.stream.finish` | span event | `ai.response.msToFirstChunk` | Vercel `doStream` | Vercel |
| `new_token` | run `events[]` | `name`, `time` | LangSmith first-token marker for streaming `llm` runs; feeds `first_token_time` | LangSmith log-llm-trace |
| `memory.query`, `memory.content` | span event | `content` | Strands memory spans | Strands tracer.py |
| MCP HTTP exchange | log record | `http.request.body.content`, `http.response.body.content`, `http.request.header.{}`, `http.response.header.{}`, `mcp.*` | ADK, gated by `ADK_CAPTURE_MCP_HTTP_BODIES` and `OTEL_INSTRUMENTATION_HTTP_CAPTURE_HEADERS_CLIENT_{REQUEST,RESPONSE}` | ADK tracing.py |
| `HookEventMessage` | message stream | hook input/output | Claude SDK when `include_hook_events` set | Claude changelog 0.1.74 |

Content-capture switches:

| SDK | switch | default | effect |
|---|---|---|---|
| Claude Agent SDK | none (messages always carry content) | on | — |
| OpenAI Agents | `RunConfig.trace_include_sensitive_data`, env `OPENAI_AGENTS_TRACE_INCLUDE_SENSITIVE_DATA`; `OPENAI_AGENTS_DISABLE_TRACING=1` | on | Drops `input`/`output` on generation and function spans |
| ADK | `ADK_CAPTURE_MESSAGE_CONTENT_IN_SPANS` (span payloads → `"{}"`), `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` (log bodies), `ADK_EXPERIMENTAL_TELEMETRY` | on / off / off | Structure kept, payload blanked; binary parts summarised as `<inline_data: mime, N bytes>`; thought signatures stripped |
| Vercel | `telemetry.recordInputs` / `recordOutputs`, `includeRuntimeContext`, `includeToolsContext` | on | Telemetry itself is opt-in per call (`isEnabled`) |
| LangSmith | `LANGSMITH_HIDE_INPUTS` / `hide_inputs` callables (not covered above; standard SDK feature) | on | |
| Pydantic AI | `InstrumentationSettings(include_content, include_binary_content, include_model_request_parameters)` | on | |
| Mastra | `SensitiveDataFilter` processor (on by default), `excludeSpanTypes`, `spanFilter`, `serializationOptions` (`maxStringLength` 131072, `maxDepth` 8, `maxArrayLength` 50, `maxObjectKeys` 50) | on | Redaction plus truncation at export |
| Strands | `OTEL_SEMCONV_STABILITY_OPT_IN` flags `gen_ai_span_attributes_only`, `gen_ai_unredacted_attributes=<list>`; everything else is redacted through `_redact()` | redacted in latest mode | |

## Per-run summary fields

| SDK | where | fields |
|---|---|---|
| Claude Agent SDK | `ResultMessage` (final message) | `subtype` (`success`, `error_max_turns`, `error_during_execution`, …), `duration_ms`, `duration_api_ms`, `is_error`, `num_turns`, `session_id`, `stop_reason`, `terminal_reason` (`completed`, `max_turns`, `aborted_streaming`, `aborted_tools`, …), `total_cost_usd`, `usage` (`input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`, `web_search_requests`, `cost_usd`), `model_usage` per model (`inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`, `webSearchRequests`, `costUSD`, `contextWindow`, `canonicalModel`, `provider`), `permission_denials`, `errors`, `api_error_status`, `structured_output`, `deferred_tool_use` |
| OpenAI Agents | `task` and `turn` spans `data.usage`; trace has only `workflow_name`, `group_id`, `metadata` | No cost, no turn count on the trace object; `turn` index on turn spans |
| ADK | Metrics only (`gen_ai.invoke_agent.inference_calls`, `.tool_calls`, `adk.experimental.invoke_agent.*_tokens`, `gen_ai.invoke_agent.duration`) | Root span has no usage roll-up attributes; no cost |
| Vercel | Root `ai.generateText` / `invoke_agent` span | `ai.usage.*` summed over steps, `ai.response.finishReason`, `ai.response.toolCalls`; no cost, no step count attribute (`ai.settings.maxSteps` only) |
| LangSmith | Root run | `total_tokens`, `prompt_tokens`, `completion_tokens`, `total_cost`, `prompt_cost`, `completion_cost` rolled up from children; `status` (`success`/`error`/`pending`), `error`, `first_token_time`, `feedback_stats`; `usage_metadata` details (`input_token_details.cache_read/cache_creation/audio/text/image`, `output_token_details.reasoning/audio/text/image`, `input_cost_details`, `output_cost_details`) |
| Pydantic AI | Root `invoke_agent` span | `gen_ai.aggregated_usage.input_tokens/output_tokens` (+ `details.*`), `final_result`, `pydantic_ai.all_messages`; cost via `operation.cost` metric only; `RunUsage` object on the result has `requests`, `tool_calls`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `details` |
| Mastra | `AGENT_RUN` span `output`/`attributes` | Native `usage` (`inputTokens`, `outputTokens`, `inputDetails.{text,cacheRead,cacheWrite,cacheWrite5m,cacheWrite1h,audio,image}`, `outputDetails.{text,reasoning,audio,image}`), `costContext`, `tripwireAbort`; the OTel exporter deliberately does not put usage on `invoke_agent` |
| Strands | `invoke_agent` span end + `EventLoopMetrics.get_summary()` | Span: `gen_ai.usage.{prompt,completion,input,output,total}_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens`, `finish_reason` in `gen_ai.choice`. Summary: `total_cycles`, `total_duration`, `average_cycle_time`, `tool_usage[tool].execution_stats.{call_count,success_count,error_count,total_time,average_time,success_rate}`, `accumulated_usage.{inputTokens,outputTokens,totalTokens,cacheReadInputTokens,cacheWriteInputTokens}`, `accumulated_metrics.latencyMs`, `traces`; `gen_ai_use_latest_invocation_tokens` switches root usage from lifetime-accumulated to last-invocation |

## Execution environment

| SDK | captured |
|---|---|
| Claude Agent SDK | `SystemMessage(subtype="init")`: `cwd`, `model`, `tools`, `mcp_servers`, `permissionMode`, `claude_code_version`, `apiKeySource`, `slash_commands`, `agents`, `skills`, `git_branch`/`git_root` (some versions); hooks get `cwd`, `transcript_path`; options `env`, `sandbox`, `CLAUDE_AGENT_SDK_CLIENT_APP` in User-Agent |
| OpenAI Agents | Nothing beyond `workflow_name`/`metadata` |
| ADK | Nothing on spans; resource attributes come from the OTel SDK setup |
| Vercel | `ai.request.headers.*`, opt-in `includeRuntimeContext` keys |
| LangSmith | `extra.runtime`: `sdk`, `sdk_version`, `library`, `platform`, `runtime`, `py_implementation`, `runtime_version`, `langchain_version`, `langchain_core_version`, `revision_id` (from `LANGCHAIN_REVISION_ID` or `git describe`), CI commit SHAs (`GITHUB_SHA`, `CI_COMMIT_SHA`, `VERCEL_GIT_COMMIT_SHA`, `BUILDKITE_COMMIT`, …); `extra.metrics`: `thread_count`, `mem.rss`, `cpu.time.{sys,user}`, `cpu.ctx_switches`, `cpu.percent`; any non-secret `LANGCHAIN_*`/`LANGSMITH_*` env vars |
| Pydantic AI | `server.address`, `server.port` on model spans; nothing about host |
| Mastra | `serviceName` in config; `requestContextKeys` promote request context to root-span metadata; `serverAddress`/`serverPort` on model spans |
| Strands | Only what you pass in `trace_attributes` |

LangSmith is the only SDK that records git revision and process resource usage by default.

## Notable design choices

- **Two naming eras coexist.** Strands, ADK and Pydantic AI all dual-emit or version-gate legacy names (`gen_ai.system`, `gen_ai.usage.prompt_tokens`, `cache_read_input_tokens`) against the v1.37/1.38 semconv names (`gen_ai.provider.name`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.client.inference.operation.details`). `OTEL_SEMCONV_STABILITY_OPT_IN=gen_ai_latest_experimental` is the shared switch.
- **Root-span usage double counting** is handled three ways: Pydantic renames to `gen_ai.aggregated_usage.*`; Mastra omits usage from `invoke_agent` in OTel export; Strands puts totals on the root under the same `gen_ai.usage.*` keys (and so double counts in naive sums).
- **Turn as a span** exists only in OpenAI Agents (`turn`, with `turn` index) and Strands (`execute_event_loop_cycle` with `cycle_id`/`parent_cycle_id`); Mastra's `MODEL_STEP` has `stepIndex`. Everyone else flattens model calls under the root.
- **Cost** is computed client-side only by Claude Agent SDK (`total_cost_usd`, per-model `costUSD`) and Pydantic AI (`operation.cost` histogram); LangSmith computes server-side from `ls_model_name`/`ls_provider` greedily by token type; Mastra carries `costContext` for the backend.
- **Stop reason** at run level: Claude has both `stop_reason` (model) and `terminal_reason` (loop: `completed`, `max_turns`, `aborted_*`) plus `subtype`; Strands `finish_reason` on the `gen_ai.choice` event; Vercel `ai.response.finishReason`; ADK `gen_ai.response.finish_reasons` per call only.
- **Correlation ids**: ADK `gcp.vertex.agent.{invocation_id,session_id,event_id}` on every span; Pydantic baggage `gen_ai.agent.call.id` + `gen_ai.conversation.id`; OpenAI `group_id`; LangSmith `thread_id` metadata + `dotted_order`; Claude `session_id` + `uuid` + `parent_tool_use_id`.
- **Content off = structure kept.** ADK writes `"{}"`/`"<elided>"` placeholders rather than dropping attributes, so dashboards keep working.
- **Truncation at export** (Mastra `serializationOptions`) rather than at capture.
- **Cardinality**: Strands puts `tool_use_id` as a metric attribute, which is unbounded; ADK keys metrics by `gen_ai.agent.name`/`gen_ai.tool.name` only.
- **Tool-call linkage**: every SDK carries the provider's tool call id (`tool_use_id`, `gen_ai.tool.call.id`, `toolCallId`, `ai.toolCall.id`) so results can be joined to requests.

## Recommendation for lablet (opinion)

- **`gen_ai.operation.name` = `invoke_agent` / `chat` / `execute_tool` with span names `invoke_agent {agent}`, `chat {model}`, `execute_tool {tool}`** — adopt. Six of eight SDKs (ADK, Vercel otel, Pydantic, Mastra, Strands, OTel contrib for OpenAI) converge on exactly this; it is the de facto standard.
- **`gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.response.model`, `gen_ai.response.finish_reasons`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens`, `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.conversation.id`** — adopt as the core attribute set; emit only the v1.37+ names, never the legacy dual set.
- **A turn span between root and `chat`** — adapt from OpenAI `turn` / Strands `execute_event_loop_cycle`: name it `turn {n}` or similar with an integer turn index attribute; most SDKs lack it and it is what an agent-loop tool should own.
- **Root-span totals under a distinct namespace** (Pydantic's `gen_ai.aggregated_usage.*` pattern) — adopt; avoids the Strands double-count problem while still giving one-span run summaries.
- **Run summary fields modelled on Claude `ResultMessage`**: `num_turns`, `duration_ms`, `duration_api_ms`, `total_cost_usd`, `is_error`, loop-level `terminal_reason` distinct from model `stop_reason`, per-model usage map — adopt; it is the richest per-run summary of the eight and maps cleanly to root-span attributes.
- **Content in `gen_ai.input.messages` / `gen_ai.output.messages` / `gen_ai.tool.call.arguments` / `gen_ai.tool.call.result`, gated by one switch, with placeholders (ADK style) rather than absent attributes when off** — adopt.
- **Metrics: `gen_ai.client.token.usage` (`gen_ai.token.type` dim), `gen_ai.client.operation.duration`, plus per-run histograms like ADK `gen_ai.invoke_agent.inference_calls` / `.tool_calls`** — adopt; keep dimensions to agent/model/tool name and `error.type`; skip Strands' `tool_use_id` metric attribute.
- **Environment capture like LangSmith `extra.runtime`** (SDK version, git revision from `git describe` or CI SHA env vars, platform, `mem.rss`, cpu time) — adapt as OTel resource attributes plus a few run attributes; no other SDK does this and lablet's purpose is reproducible instrumented runs.
- **Client-side cost** — adapt: Claude and Pydantic prove a bundled price table is workable; expose as `operation.cost`-style metric and a root attribute, mark the price-table version.
- **Vendor namespaces (`gcp.vertex.agent.*`, `ai.*`, `adk.experimental.*`) and Strands' legacy `gen_ai.usage.prompt_tokens`/`total_tokens`** — skip; where lablet needs its own attributes, use one `lablet.*` prefix and keep it small.
