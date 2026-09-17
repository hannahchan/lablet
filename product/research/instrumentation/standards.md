# Standards: OpenTelemetry GenAI semantic conventions

Researched: 2026-09-18. Sources are linked inline. Everything here is an inventory of what the source emits or expects; recommendations for lablet are confined to the final section and marked as opinion.

## Sources covered

- **OpenTelemetry GenAI semantic conventions** — [open-telemetry/semantic-conventions-genai](https://github.com/open-telemetry/semantic-conventions-genai), `main` at commit `c88d504` (2026-09-16). No tagged release yet; `model/manifest.yaml` declares `schema_url: https://opentelemetry.io/schemas/gen-ai-dev/1.42.0-dev`, `stability: development`, and depends on core semconv `v1.44.0`. Every `gen_ai.*` and `mcp.*` item below is **Development** unless marked otherwise. Files read: `docs/gen-ai/{gen-ai-spans,gen-ai-agent-spans,gen-ai-metrics,gen-ai-events,gen-ai-exceptions,mcp,anthropic}.md`, `docs/registry/attributes/{gen-ai,mcp}.md`, `model/gen-ai/{registry,spans,events}.yaml`, `model/gen-ai/gen-ai-{input-messages,output-messages,tool-definitions}.json`, `model/mcp/spans.yaml`.
- **Core semantic conventions** — [open-telemetry/semantic-conventions](https://github.com/open-telemetry/semantic-conventions) `v1.44.0` (2026-08-04). `v1.42.0` (2026-06-12) deprecated `model/gen-ai/`, `model/openai/`, `model/mcp/` in favour of the new repo; the [opentelemetry.io gen-ai pages](https://opentelemetry.io/docs/specs/semconv/gen-ai/) are now redirect stubs. Pages read: `docs/resource/{README,service,host,container,process,os,deployment-environment}.md`, `docs/exceptions/exceptions-{spans,logs}.md`, `docs/general/session.md`, registry pages for `vcs`, `user`, `gen-ai` (deprecated list).
- **MCP specification** — [modelcontextprotocol.io/specification/latest](https://modelcontextprotocol.io/specification/latest) = revision `2026-07-28`; [SEP-414](https://modelcontextprotocol.io/seps/414-request-meta) (Final) for trace context in `_meta`; [MCP Python SDK OpenTelemetry page](https://py.sdk.modelcontextprotocol.io/run/opentelemetry/) as the reference implementation.
- **OpenTelemetry Weaver** — [open-telemetry/weaver](https://github.com/open-telemetry/weaver) `v0.26.1` (2026-09-03). Read `schemas/semconv-syntax.md` (v1 groups), `schemas/semconv-syntax.v2.md` (`file_format: definition/2`, Alpha), `schemas/semconv.schema.json`, `docs/define-your-own-telemetry-schema.md`.

## Span structure

The GenAI conventions model an agent run as a tree of operation spans. There is no dedicated "run" or "turn" span; the outermost concept is `invoke_workflow` or `invoke_agent`.

| Span (operation) | Span name | Kind | Parent guidance | Source |
|---|---|---|---|---|
| `invoke_workflow` | `invoke_workflow {gen_ai.workflow.name}` | INTERNAL | Root of a multi-agent or orchestrated process | [gen-ai-agent-spans.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-agent-spans.md) |
| `invoke_agent` | `invoke_agent {gen_ai.agent.name}` (or `invoke_agent`) | INTERNAL (local framework) or CLIENT (remote agent service) | Parent of inference, `plan`, and `execute_tool` spans | same |
| `create_agent` | `create_agent {gen_ai.agent.name}` | CLIENT | Remote agent provisioning only | same |
| `plan` | `plan {gen_ai.agent.name}` (or `plan`) | INTERNAL | "The LLM call that generates the plan SHOULD be a child of the plan span"; tool spans are siblings under `invoke_agent` | same |
| inference (`chat`, `generate_content`, `text_completion`) | `{gen_ai.operation.name} {gen_ai.request.model}` e.g. `chat claude-…` | CLIENT (INTERNAL if same-process model) | Child of `invoke_agent`; covers the whole logical call **including automatic retries** | [gen-ai-spans.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-spans.md) |
| `execute_tool` | `execute_tool {gen_ai.tool.name}` | INTERNAL | "SHOULD create execute_tool spans as children of invoke_agent spans" | same |
| `embeddings` | `embeddings {gen_ai.request.model}` | CLIENT | — | same |
| `retrieval` | `retrieval {gen_ai.data_source.id}` | CLIENT | — | same |
| `fetch_response` | `fetch_response` (no id: cardinality) | CLIENT | No token usage reported on it | same |
| memory ops (`create_memory`, `search_memory`, …) | `{gen_ai.operation.name}` | CLIENT or INTERNAL | — | same |
| MCP client | `{mcp.method.name} {target}` e.g. `tools/call get_weather`; `{mcp.method.name}` if no low-cardinality target | CLIENT | "MCP tool call execution spans are compatible with GenAI execute_tool spans. If the MCP instrumentation can reliably detect that outer GenAI instrumentation is already tracing the tool execution, it SHOULD NOT create a separate span. Instead, it SHOULD add MCP-specific attributes to the existing tool execution span." | [model/mcp/spans.yaml](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/model/mcp/spans.yaml) |
| MCP server | same naming | SERVER | Remote parent extracted from `params._meta` | [mcp.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/mcp.md) |

Span events versus spans: exceptions were historically the `exception` span event; core semconv now marks that **Deprecated** in favour of exception log records, gated by `OTEL_SEMCONV_EXCEPTION_SIGNAL_OPT_IN=logs|logs/dup` ([exceptions-spans.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/exceptions/exceptions-spans.md)). GenAI content (messages) is a span attribute or a log record, never a span event.

## Attributes

Requirement levels abbreviated: Req, CondReq (with condition), Rec, OptIn. Stability: Dev = Development, St = Stable, RC = Release Candidate.

### gen_ai.* on inference spans (`chat` etc.)

Source: [gen-ai-spans.md#inference](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-spans.md) and [registry/attributes/gen-ai.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/registry/attributes/gen-ai.md).

| name | type | set on | requirement | description |
|---|---|---|---|---|
| `gen_ai.operation.name` | string (enum) | span, metric, event | Req | `chat`, `generate_content`, `text_completion`, `embeddings`, `retrieval`, `fetch_response`, `execute_tool`, `create_agent`, `invoke_agent`, `invoke_workflow`, `plan`, `create_memory_store`, `delete_memory_store`, `create_memory`, `update_memory`, `upsert_memory`, `delete_memory`, `search_memory` |
| `gen_ai.provider.name` | string (enum) | span, metric, event | Req | `openai`, `anthropic`, `aws.bedrock`, `azure.ai.openai`, `azure.ai.inference`, `gcp.gemini`, `gcp.gen_ai`, `gcp.vertex_ai`, `cohere`, `mistral_ai`, `groq`, `deepseek`, `perplexity`, `moonshot_ai`, `x_ai`, `ibm.watsonx.ai`. Replaces deprecated `gen_ai.system` |
| `error.type` | string | span | CondReq if error | St. `_OTHER` fallback; provider error code or exception type |
| `gen_ai.request.model` | string | span | CondReq if available | exact requested model name |
| `gen_ai.conversation.id` | string | span | CondReq if readily available | do NOT fabricate from trace id or hash |
| `gen_ai.conversation.compacted` | boolean | span | Rec when available | context uses compacted prior conversation |
| `gen_ai.output.type` | string (enum) | span | CondReq when request specifies | `text`, `json`, `image`, `speech` |
| `gen_ai.prompt.name` / `gen_ai.prompt.version` | string | span | CondReq when a named template is used | |
| `gen_ai.request.choice.count` | int | span | CondReq if != 1 | |
| `gen_ai.request.seed` | int | span | CondReq if in request | |
| `gen_ai.request.stream` | boolean | span | CondReq if applicable | unset => non-streaming |
| `gen_ai.request.top_k` | int | span | CondReq if applicable | |
| `gen_ai.request.temperature`, `top_p`, `frequency_penalty`, `presence_penalty` | double | span | Rec | |
| `gen_ai.request.max_tokens` | int | span | Rec | |
| `gen_ai.request.stop_sequences` | string[] | span | Rec | |
| `gen_ai.request.reasoning.level` | string | span | Rec when applicable | `low`/`medium`/`high`; Anthropic maps to `output_config.effort` ([anthropic.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/anthropic.md)) |
| `gen_ai.request.previous_response.id` | string | span | Rec | |
| `gen_ai.response.id`, `gen_ai.response.model` | string | span | Rec | |
| `gen_ai.response.finish_reasons` | string[] | span | Rec | one per choice; report `error` if none received |
| `gen_ai.response.status` | string (enum) | span | — (registry) | `queued`, `in_progress`, `completed`, `incomplete`, `failed`, `cancelled` |
| `gen_ai.response.time_to_first_chunk` | double (s) | span | Rec if streaming | |
| `gen_ai.usage.input_tokens` | int | span | Rec | "SHOULD include all types of input tokens, including cached tokens"; report **billed** counts |
| `gen_ai.usage.output_tokens` | int | span | Rec | includes reasoning tokens |
| `gen_ai.usage.cache_read.input_tokens` | int | span | Rec when applicable | tokens served from provider cache |
| `gen_ai.usage.cache_write.input_tokens` | int | span | Rec when applicable | tokens written to cache (the name is `cache_write`, not `cache_creation`) |
| `gen_ai.usage.reasoning.output_tokens` | int | span | Rec when applicable | subset of output_tokens |
| `gen_ai.usage.{text,image,audio}.input_tokens`, `.output_tokens`, `.cache_read.input_tokens` | int | span | Rec when applicable | per-modality splits |
| `server.address` / `server.port` | string / int | span | Rec / CondReq if address set | St. Real backend behind proxies |
| `gen_ai.input.messages` | any (JSON) | span or event | OptIn | schema below |
| `gen_ai.output.messages` | any (JSON) | span or event | OptIn | |
| `gen_ai.system_instructions` | any (JSON) | span or event | OptIn | |
| `gen_ai.tool.definitions` | any (JSON) | span or event | OptIn | `{type,name,description?,parameters?}`; description/parameters NOT RECOMMENDED by default |
| `gen_ai.prompt.variable` | `template[string]` | span | OptIn | emitted as `gen_ai.prompt.variable.<name>` |

Sampling-relevant (set at span start): `gen_ai.operation.name`, `gen_ai.provider.name`, `gen_ai.request.model`, `server.address`, `server.port`.

### gen_ai.* on agent spans

Source: [gen-ai-agent-spans.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-agent-spans.md).

| name | type | set on | requirement | description |
|---|---|---|---|---|
| `gen_ai.agent.name` | string | `invoke_agent`, `create_agent`, `plan`, `execute_tool` | CondReq when available (Rec on execute_tool) | |
| `gen_ai.agent.id` | string | `invoke_agent` CLIENT, `create_agent` | CondReq if applicable | provider-assigned id |
| `gen_ai.agent.description`, `gen_ai.agent.version` | string | same | CondReq if provided | |
| `gen_ai.workflow.name` | string | `invoke_workflow` | CondReq | must be low cardinality |
| `gen_ai.data_source.id` | string | `invoke_agent`, `retrieval` | CondReq if applicable | |
| `gen_ai.provider.name` | string | `invoke_agent` CLIENT, `create_agent` | Req | not on INTERNAL invoke_agent |
| request/usage/messages attributes above | | `invoke_agent` (both kinds) | Rec / OptIn | agent span may aggregate usage |

### gen_ai.tool.* on `execute_tool`

| name | type | requirement | description | source |
|---|---|---|---|---|
| `gen_ai.tool.name` | string | Req | | [gen-ai-spans.md#execute-tool](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-spans.md) |
| `gen_ai.tool.type` | string | Req | well-known `function` (client-side), `extension` (agent-side external API), `datastore` | same |
| `gen_ai.tool.call.id` | string | CondReq when available | provider call id | same |
| `gen_ai.tool.description` | string | Rec | | same |
| `gen_ai.tool.call.arguments` | any | OptIn | sensitive | same |
| `gen_ai.tool.call.result` | any | OptIn | sensitive | same |
| `error.type` | string | CondReq if error | St | same |

### mcp.* and companions on MCP spans

Source: [mcp.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/mcp.md), [registry/attributes/mcp.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/registry/attributes/mcp.md).

| name | type | stability | requirement | description |
|---|---|---|---|---|
| `mcp.method.name` | string (enum) | Dev | Req | `initialize`, `ping`, `tools/list`, `tools/call`, `prompts/list`, `prompts/get`, `resources/list`, `resources/read`, `resources/templates/list`, `resources/subscribe`, `resources/unsubscribe`, `roots/list`, `sampling/createMessage`, `elicitation/create`, `completion/complete`, `logging/setLevel`, `notifications/{cancelled,initialized,message,progress,prompts/list_changed,resources/list_changed,resources/updated,roots/list_changed,tools/list_changed}` |
| `mcp.session.id` | string | Dev | Rec when in a session | |
| `mcp.protocol.version` | string | Dev | Rec | e.g. `2025-11-25` |
| `mcp.resource.uri` | string | Dev | CondReq on resource ops; OptIn in span name | |
| `jsonrpc.request.id` | string | Dev | CondReq on requests | notifications omit |
| `jsonrpc.protocol.version` | string | Dev | Rec when not `2.0` | |
| `rpc.response.status_code` | string | RC | CondReq on error response | e.g. `-32602` |
| `gen_ai.operation.name` | string | Dev | Rec | `execute_tool` on `tools/call` |
| `gen_ai.tool.name` / `gen_ai.prompt.name` | string | Dev | CondReq when tool/prompt related | |
| `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result`, `gen_ai.prompt.variable` | any / template[string] | Dev | OptIn | |
| `network.transport` | string | St | Rec | stdio => `pipe`; HTTP => `tcp`/`quic` |
| `network.protocol.name` / `.version` | string | St | Rec when applicable | `http` `2` for Streamable HTTP; `websocket` |
| `server.address`/`server.port` (client span), `client.address`/`client.port` (server span) | string/int | St | Rec | |
| `error.type` | string | St | CondReq | |

Context propagation ([mcp.md#context-propagation](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/mcp.md), [SEP-414](https://modelcontextprotocol.io/seps/414-request-meta) Final): inject the configured propagators into `params._meta` of every request and notification; receiver extracts and uses it as the remote parent. Keys are **unprefixed** `traceparent`, `tracestate`, `baggage` (documented exception to MCP's DNS-prefix rule). Example: `"_meta": {"traceparent": "00-…-01"}`. The MCP spec `2026-07-28` itself contains no other telemetry guidance. The MCP Python SDK emits SERVER spans per inbound message with `mcp.method.name`, `mcp.protocol.version`, `jsonrpc.request.id`, `gen_ai.operation.name=execute_tool`, `gen_ai.tool.name`, and sets ERROR status when `is_error=true`.

### Deprecated / renamed gen_ai names

Source: core [registry/attributes/gen-ai](https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/) (deprecated section) and `model/gen-ai/deprecated/*.yaml` in core `v1.44.0`.

| old | replacement | note |
|---|---|---|
| `gen_ai.system` | `gen_ai.provider.name` | renamed |
| `gen_ai.usage.prompt_tokens` / `gen_ai.usage.completion_tokens` | `gen_ai.usage.input_tokens` / `gen_ai.usage.output_tokens` | renamed |
| `gen_ai.prompt` / `gen_ai.completion` (string attrs) | `gen_ai.input.messages` / `gen_ai.output.messages` | removed |
| `gen_ai.request.choice_count` | `gen_ai.request.choice.count` | renamed |
| `gen_ai.openai.request.seed` | `gen_ai.request.seed` | |
| `gen_ai.openai.request.response_format` | `gen_ai.output.type` | |
| `gen_ai.openai.request.service_tier`, `…response.service_tier`, `…response.system_fingerprint` | `openai.request.service_tier`, `openai.response.service_tier`, `openai.response.system_fingerprint` | moved to `openai.*` |
| events `gen_ai.system.message`, `gen_ai.user.message`, `gen_ai.assistant.message`, `gen_ai.tool.message`, `gen_ai.choice` | `gen_ai.client.inference.operation.details` | per-message events removed |
| everything `gen_ai.*`, `mcp.*` in core repo | same names in semantic-conventions-genai | deprecated in core `v1.42.0` (2026-06-12); MCP was first added in core `v1.39` |

### Core resource attributes

Source: core `v1.44.0` `docs/resource/*.md` (linked in Sources).

| name | type | requirement | stability | source |
|---|---|---|---|---|
| `service.name` | string | Req | St | [service.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/service.md); default `unknown_service:<exe>` |
| `service.namespace` | string | Req | St | |
| `service.instance.id` | string | Req | St | random UUID recommended |
| `service.version` | string | Rec | St | |
| `telemetry.sdk.name` / `.language` / `.version` | string | Req | St | [README.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/README.md) |
| `telemetry.distro.name` / `.version` | string | Rec | St | |
| `deployment.environment.name` | string | Rec | St | `development`, `production`, `staging`, `test`; old `deployment.environment` deprecated |
| `host.id`, `host.name`, `host.arch`, `host.type`, `host.image.{id,name,version}` | string | Rec | Dev | [host.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/host.md) |
| `host.ip`, `host.mac`, `host.cpu.*` | string[] / mixed | OptIn | Dev | |
| `os.type` | string (enum) | Req | Dev | `darwin`, `linux`, `windows`, … ([os.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/os.md)) |
| `os.name`, `os.version`, `os.description`, `os.build_id` | string | Rec | Dev | |
| `container.id` | string | Rec | St | [container.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/container.md) |
| `container.name`, `container.label.<key>`, `oci.manifest.digest` | string | Rec | Dev | |
| `container.command`, `.command_args`, `.command_line` | string / string[] | OptIn | Dev | |
| `process.pid`, `process.creation.time` | int / string | Req | RC | [process.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/resource/process.md) |
| `process.command`, `process.owner`, `process.runtime.{name,version,description}` | string | Rec | RC | |
| `process.command_args`, `process.command_line`, `process.args_count`, `process.parent_pid`, `process.working_directory`, `process.interactive`, `process.title`, `process.linux.cgroup` | mixed | OptIn | RC | |
| `vcs.ref.head.name`, `vcs.ref.head.revision`, `vcs.ref.head.type` (`branch`/`tag`) | string | — (registry only, no resource group) | RC | [vcs registry](https://opentelemetry.io/docs/specs/semconv/registry/attributes/vcs/) |
| `vcs.ref.base.{name,revision,type}`, `vcs.repository.name`, `vcs.repository.url.full`, `vcs.owner.name`, `vcs.provider.name` (`github`/`gitlab`/`gitea`/`bitbucket`), `vcs.change.{id,state,title}`, `vcs.revision_delta.direction`, `vcs.line_change.type` | string | — | RC | old `vcs.repository.ref.*`, `vcs.repository.change.*` deprecated |
| `session.id`, `session.previous_id` | string | OptIn on spans/logs; Req on `session.start`/`session.end` events | Dev | [session.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/general/session.md) |
| `user.id`, `user.name`, `user.email`, `user.full_name`, `user.hash`, `user.roles` (string[]) | string | — | Dev | [user registry](https://opentelemetry.io/docs/specs/semconv/registry/attributes/user/) |
| `exception.type`, `exception.message` | string | CondReq (one of them) | St | [exceptions-logs.md](https://github.com/open-telemetry/semantic-conventions/blob/main/docs/exceptions/exceptions-logs.md) |
| `exception.stacktrace` | string | Rec | St | |
| `exception.escaped` | boolean | — | **Deprecated** | |

## Metrics

Source: [gen-ai-metrics.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-metrics.md), [mcp.md#metrics](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/mcp.md). All Histogram, all Development.

| name | unit | dimensions (Req / CondReq / Rec) | description | buckets |
|---|---|---|---|---|
| `gen_ai.client.token.usage` | `{token}` | Req `gen_ai.operation.name`, `gen_ai.provider.name`, `gen_ai.token.type` (`input`/`output`); CondReq `gen_ai.request.model`, `server.port`; Rec `gen_ai.response.model`, `server.address` | input and output tokens | 1,4,16,64,…,67108864 |
| `gen_ai.client.operation.duration` | `s` | Req `gen_ai.operation.name`; CondReq `error.type`, `gen_ai.provider.name`, `gen_ai.request.model`, `server.port`; Rec `gen_ai.response.model`, `server.address` | operation duration | 0.01…81.92 (x2) |
| `gen_ai.client.operation.time_to_first_chunk` | `s` | Req op name, provider; CondReq model, port; Rec response.model, address | streaming TTFB | 0.01…81.92 |
| `gen_ai.client.operation.time_per_output_chunk` | `s` | same | per chunk after first | 0.01…81.92 |
| `gen_ai.server.request.duration` | `s` | Req op name, provider; CondReq `error.type`, model, port | server side (model servers) | 0.01…81.92 |
| `gen_ai.server.time_per_output_token` | `s` | Req op name, provider | | 0.01…2.5 |
| `gen_ai.server.time_to_first_token` | `s` | Req op name, provider | | 0.001…10 |
| `gen_ai.invoke_workflow.duration` | `s` | CondReq `error.type`, `gen_ai.workflow.name` | | 1…7200 |
| `gen_ai.invoke_agent.duration` | `s` | CondReq `error.type`, `gen_ai.agent.name`; Rec `gen_ai.request.model` | end-to-end agent invocation | 0.1…409.6 |
| `gen_ai.invoke_agent.inference_calls` | `{inference_call}` | Rec `gen_ai.agent.name` | inference calls per invocation | 1…128 |
| `gen_ai.invoke_agent.tool_calls` | `{tool_call}` | Rec `gen_ai.agent.name` | tool calls per invocation | 1…128 |
| `gen_ai.execute_tool.duration` | `s` | Req `gen_ai.tool.name`; CondReq `error.type`, `gen_ai.agent.name`; Rec `gen_ai.tool.type` | | 0.01…81.92 |
| `mcp.client.operation.duration` / `mcp.server.operation.duration` | `s` | Req `mcp.method.name`; CondReq `error.type`, `rpc.response.status_code`; Rec `gen_ai.operation.name`, `gen_ai.tool.name`, `mcp.protocol.version`, `mcp.session.id`, network/server attrs; OptIn `mcp.resource.uri` | send to response/ack | 0.01…300 |
| `mcp.client.session.duration` / `mcp.server.session.duration` | `s` | CondReq `error.type` if session ends in error; Rec `mcp.protocol.version`, transport attrs | | 0.01…300 |

Deprecated metric attribute: `gen_ai.system` on all of the above (renamed to `gen_ai.provider.name`).

## Events and log records

Source: [gen-ai-events.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-events.md), [gen-ai-exceptions.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-exceptions.md), `model/gen-ai/events.yaml`.

| name | carried on | fields | when emitted | requirement |
|---|---|---|---|---|
| `gen_ai.client.inference.operation.details` | log record (event) | the full inference-span attribute set (`ref_group: span.gen_ai.inference.client`), including OptIn `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.system_instructions`, `gen_ai.tool.definitions`, `gen_ai.prompt.variable` | once per inference call, "to store input and output details independently from traces"; emitted in the span's context | OptIn (Dev) |
| `gen_ai.evaluation.result` | log record | Req `gen_ai.evaluation.name`; CondReq `gen_ai.evaluation.score.value` (double), `gen_ai.evaluation.score.label`; Rec `gen_ai.evaluation.explanation`, `gen_ai.response.id`; `error.type` | after evaluating an output; "SHOULD be parented to GenAI operation span being evaluated when possible or set `gen_ai.response.id`" | Rec (Dev) |
| `gen_ai.client.operation.exception` | log record, severity WARN (13) | `exception.type`, `exception.message` (one CondReq), `exception.stacktrace` Rec; MAY copy the span's gen_ai attributes | on API error, rate limit, timeout during a client op | Rec (Dev) |
| `exception` | span event | `exception.type`, `exception.message`, `exception.stacktrace` | core semconv; **Deprecated** in favour of exception log records (`OTEL_SEMCONV_EXCEPTION_SIGNAL_OPT_IN=logs\|logs/dup`) | — |
| `session.start` / `session.end` | log record | Req `session.id`; CondReq `session.previous_id` | core, Dev | — |

Content-capture opt-in. Message content attributes are `Opt-In` requirement level everywhere. The conventions name no normative switch; the only variable mentioned (as an example, on memory attributes and in `registry.yaml`) is `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT`. `OTEL_SEMCONV_STABILITY_OPT_IN` is not referenced in the GenAI repo at all (it was the migration switch from `gen_ai.system`-era conventions, value `gen_ai_latest_experimental`, in core releases before the move). Tool definitions' `description` and `parameters` are separately "NOT RECOMMENDED to be populated by default".

Message JSON schemas (`model/gen-ai/gen-ai-input-messages.json`, `gen-ai-output-messages.json`): a message is `{role, parts[], name?}` with `role` ∈ `system`, `user`, `assistant`, `tool`; output messages add a deprecated `finish_reason` (use `gen_ai.response.finish_reasons`) with enum `stop`, `length`, `content_filter`, `tool_call`, `compaction`, `error`. Part `type` values: `text{content}`, `tool_call{id,name,arguments}`, `tool_call_response{id,response}`, `server_tool_call`, `server_tool_call_response`, `reasoning`, `blob{mime_type,modality,content}`, `file{file_id,…}`, `uri`, `compaction{id,content}`, plus a GenericPart for extensions.

## Per-run summary fields

The conventions have no "run summary" object. Per-run totals surface as:

- Attributes on the outermost `invoke_agent` / `invoke_workflow` span: `gen_ai.usage.*` token totals, `gen_ai.response.finish_reasons`, `error.type`, span status ([gen-ai-agent-spans.md](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-agent-spans.md)).
- Metrics `gen_ai.invoke_agent.duration`, `gen_ai.invoke_agent.inference_calls`, `gen_ai.invoke_agent.tool_calls` recorded once per invocation.
- Scores as `gen_ai.evaluation.result` log records with `gen_ai.evaluation.{name,score.value,score.label,explanation}`.
- No cost attribute exists anywhere in `gen_ai.*`; the text says to report billed token counts so cost can be derived downstream.

## Execution environment

Covered only by core resource conventions (table above): `service.*`, `host.*`, `os.*`, `process.*` (including `process.working_directory`, `process.command_args`, `process.runtime.*`), `container.*`, `deployment.environment.name`. Git state has registry attributes (`vcs.ref.head.revision`, `vcs.ref.head.name`, `vcs.repository.url.full`) but no resource/entity group binds them to a process; CI conventions (`cicd.*`) use them on spans. `gen_ai.request.seed` is the only seed attribute. No conventions exist for CPU/memory usage of the agent process as span attributes (those are `process.*` / `system.*` metrics), sandbox ids, or dependency versions beyond `process.runtime.version`.

## Notable design choices

- Span names are `{operation} {low-cardinality target}`; ids (response id, resource URI) are kept out of names on purpose.
- `gen_ai.conversation.id` must be a real identifier; fabricating one is forbidden. `session.id` is the generic equivalent and is `Opt-In` on spans.
- Token counts: `input_tokens` is inclusive of cached tokens; `output_tokens` is inclusive of reasoning tokens; the cache write name is `cache_write`, not Anthropic's `cache_creation`.
- Content lives in `any`-typed JSON attributes with published schemas, either on the span or on `gen_ai.client.inference.operation.details`; per-message events are gone.
- Retries are inside one inference span; there is no per-attempt span.
- MCP and GenAI deliberately overlap: one `tools/call` produces one span carrying both `mcp.*` and `gen_ai.tool.*` attributes rather than two nested spans.
- Trace context crosses MCP via unprefixed `traceparent`/`tracestate`/`baggage` in `params._meta`, on requests and notifications, not only over HTTP headers.
- Exceptions are moving from span events to log records across all of OTel; the GenAI repo already defines its own `gen_ai.client.operation.exception` log event.
- The GenAI registry is written in Weaver v2 syntax (`file_format: definition/2`): top-level `attributes:` (each with `key`, `type`, `stability`, `brief`, enum `members: [{id, value, brief, stability}]`), `spans:` (`type`, `kind`, `name.note`, `brief`, `stability`, `requirement_level`, `attributes: [{ref|ref_group, requirement_level}]`), `events:` (`name`, `requirement_level`, `attributes`), `metrics:`; plus `attribute_groups` with `visibility: internal` and `ref_group` composition. Attribute types: `string`, `int`, `double`, `boolean`, `any`, `string[]`, `int[]`, `double[]`, `boolean[]`, `template[<primitive or array>]` (e.g. `template[string]`, `template[string[]]`), and inline enums. Requirement levels: `required`, `recommended`, `opt_in`, `conditionally_required: <text>` (`recommended: <text>` also allowed). v1 syntax (`groups: [{id, type: span|event|metric|resource|attribute_group, span_kind, name, body}]`) is still accepted; v2 is marked Alpha in Weaver but is what the GenAI SIG ships. A registry's `manifest.yaml` has `name`, `description`, `schema_url`, `stability`, and `dependencies: [{schema_url, registry_path: https://github.com/open-telemetry/semantic-conventions.git@v1.44.0[model]}]` ([define-your-own-telemetry-schema.md](https://github.com/open-telemetry/weaver/blob/main/docs/define-your-own-telemetry-schema.md)); the GenAI repo is itself an example of a dependent registry.

## Recommendation for lablet (opinion)

- `invoke_agent` span as the run root, INTERNAL kind, name `invoke_agent {gen_ai.agent.name}` — **adopt**; it is the only sanctioned top-level agent span and gets `gen_ai.invoke_agent.*` metrics for free.
- Inference spans named `chat {model}` with `gen_ai.provider.name=anthropic`, `gen_ai.operation.name=chat`, full `gen_ai.request.*`/`gen_ai.response.*`/`gen_ai.usage.*` incl. `cache_read`/`cache_write` and `reasoning.output_tokens` — **adopt verbatim**; map Anthropic `cache_creation_input_tokens` to `gen_ai.usage.cache_write.input_tokens` and `effort` to `gen_ai.request.reasoning.level`.
- `execute_tool {tool}` INTERNAL spans with `gen_ai.tool.name`, `gen_ai.tool.type`, `gen_ai.tool.call.id`, and for MCP tools the `mcp.*`/`jsonrpc.*`/`network.transport` attributes on the same span (no nested MCP span) — **adopt**; inject `traceparent` into `params._meta` per SEP-414.
- Content capture as `gen_ai.input.messages` / `gen_ai.output.messages` / `gen_ai.system_instructions` / `gen_ai.tool.definitions` in the published JSON shape, gated off by default behind one explicit switch (honour `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` as the env name) — **adopt**; prefer the `gen_ai.client.inference.operation.details` log record over span attributes to keep spans small.
- Loop turns: there is no turn span in the conventions — **adapt** with `lablet.turn` spans (INTERNAL, attributes `lablet.turn.index`, `lablet.turn.stop_reason`) between `invoke_agent` and inference/tool spans, or keep turns flat and carry `lablet.turn.index` on each child; either way declare the `lablet.*` attributes in the registry.
- Run outcome, stop reason, budgets, timeouts, observer decisions — **extend** as `lablet.run.*` attributes on the root span (`lablet.run.stop_reason`, `lablet.run.turn_count`, `lablet.run.max_turns`, `lablet.run.cost_usd`); the conventions offer no cost or stop-reason field, and `gen_ai.response.status` is per-response, not per-run.
- Metrics: emit `gen_ai.client.token.usage`, `gen_ai.client.operation.duration`, `gen_ai.invoke_agent.{duration,inference_calls,tool_calls}`, `gen_ai.execute_tool.duration` with exactly the listed dimensions and buckets — **adopt**; skip `gen_ai.server.*` (model-server side only).
- Errors: `error.type` on spans + `gen_ai.client.operation.exception` log records for API failures, `exception.*` log records (not span events) for panics — **adopt**; set `OTEL_SEMCONV_EXCEPTION_SIGNAL_OPT_IN`-style behaviour to logs-only from day one since there is no legacy to migrate.
- Resource: `service.{name,version,instance.id}`, `deployment.environment.name`, `host.{name,arch}`, `os.{type,version}`, `process.{pid,command_args,working_directory,runtime.*}`, `container.id`, plus `vcs.ref.head.{revision,name}` and `vcs.repository.url.full` as resource attributes — **adapt**; `vcs.*` are not sanctioned as resource attributes, so accept the mild non-conformance rather than inventing `lablet.git.*`.
- Correlation ids: use `session.id` for the lablet session and `gen_ai.conversation.id` only when a real provider conversation id exists — **adopt**; never derive either from the trace id.
