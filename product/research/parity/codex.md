# Codex CLI

The secondary reference, for how OpenAI models are driven through the Responses API. Pinned at rust-v0.156.1 (commit `b412ff3`), released 2026-09-23. Researched 2026-09-24 and 2026-09-25 from the source at that tag and OpenAI's docs. Nothing was executed, so the reference configuration needs a capture to confirm it. The "Fit for lablet" section compares against choices under consideration on 2026-09-24, several of them not yet recorded in `decisions.md`. The [matrix](matrix.md) has lablet's current behaviour and the proposals.

Parity with Codex is partial. In its shipped configuration, current OpenAI models call tools in "code mode": the model writes JavaScript for a single `exec` tool, and that code calls the real tools. Its MCP tools are also deferred behind `tool_search`. The reference configuration below switches both off through a stripped model catalog, which puts Codex on its direct-tool path, the one gpt-5.5 uses. Even then, it can't be made fully clean: three MCP resource tools can't be switched off, and MCP tools reach the model inside a `namespace` tool rather than as top-level function tools.

The source at the pinned tag corrects two statements in the profile below, both taken from the docs. MCP timeouts default to 30 s for startup and 300 s per call, not 10 s and 60 s. MCP tools aren't sent as flat function tools.

## Profile

Codex is only usable as a secondary reference for lablet. To make it comparable you need a hand-edited model catalog plus about ten feature switches, and fast releases will keep breaking that setup. I read the source through the GitHub API at tag `rust-v0.156.1`, and the docs at learn.chatgpt.com, where developers.openai.com/codex now redirects. Paths are relative to `codex-rs/` (https://github.com/openai/codex/tree/rust-v0.156.1/codex-rs).

### 1. Identity

OpenAI's open-source terminal coding agent, written in Rust, licensed Apache-2.0 and maintained by OpenAI (https://github.com/openai/codex). The latest stable release is **0.156.1, published 2026-09-23**. It's a hotfix that adds GPT-6 Sol and Luna to the model catalog (https://github.com/openai/codex/releases/tag/rust-v0.156.1). Several alpha builds ship every day.

### 2. Relevance

- About 126k stars and 19.7k forks (GitHub API, 2026-09-24).
- More than 5M weekly users in June 2026 (https://www.constellationr.com/insights/news/openai-touts-broadening-codex-usage-5-million-weekly-active-users).
- It's a full MCP host (stdio and streamable HTTP, with OAuth) and a skills host, so it's the main OpenAI-side user of MCP servers and skills.

### 3. Providers/APIs

- **Responses API only.** `wire_api="chat"` now fails with an error (`model-provider-info/src/lib.rs`; https://github.com/openai/codex/discussions/7782).
- **Built-in providers:** openai, amazon-bedrock, ollama and lmstudio. You can add custom `model_providers.*` entries.
- **Transport:** the OpenAI provider uses Responses over WebSocket and falls back to HTTPS.
- **No Anthropic Messages API.**

### 4. Checkability

- **Headless:** `codex exec` runs one turn, then exits (exit code 1 on failure).
  - Approval defaults to `never` and the sandbox to read-only.
  - Flags: `--json`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `-c k=v` and `--disable <feature>` (`exec/src/cli.rs`, `exec/src/lib.rs`, `cli/src/main.rs`; https://learn.chatgpt.com/docs/non-interactive-mode).
- **System prompt:** `model_instructions_file` **replaces** the base instructions. `developer_instructions` **appends** a developer message (`core/src/config/mod.rs`). Other injected blocks each need their own switch:
  - `include_permissions_instructions`
  - `include_environment_context`
  - `include_apps_instructions`
  - `include_collaboration_mode_instructions`
  - `skills.include_instructions`
  - `project_doc_max_bytes=0` for AGENTS.md
- **Restricting tools:**
  - MCP servers take `enabled_tools` and `disabled_tools`.
  - Built-ins switch off with repeated `--disable`: shell_tool, view_image, sleep_tool, apps, plugins, tool_suggest, image_generation, goals. Also set `web_search="disabled"`.
  - apply_patch, code mode and multi-agent come from the model catalog. Removing them needs a stripped `model_catalog_json`.
  - A third-party lockdown at 0.155.1 does exactly this and still ends up with three MCP-resource helper tools (https://github.com/Fledgewing/SwitchYard/pull/270; `core/src/tools/spec_plan.rs`).
- **Sub-agents:** only `agents.enabled=false` turns them off. `features.multi_agent` alone doesn't, because the catalog's `multi_agent_version` takes precedence.
- **Hooks:** `--disable hooks`, or run with a clean `CODEX_HOME`.
- **Approvals:** exec rejects approval requests and cancels elicitations (`exec/src/lib.rs`). Under `never`, any MCP tool without `readOnlyHint` is **denied**. The fixes are `default_tools_approval_mode="approve"` on the server, or a full-access sandbox (`core/src/mcp_tool_call.rs`, `codex-mcp/src/mcp/mod.rs`).
- **Recording proxy:** two options, each with a cost (`model-provider/src/provider.rs`).
  - `openai_base_url` keeps the built-in provider. The proxy must handle WebSocket, or Codex uses up retries before falling back to HTTPS.
  - A custom provider uses plain HTTPS streaming (SSE) by default but loses server-side compaction.
- **Pinning:** install exact npm or binary versions. API-key auth uses the catalog bundled in the binary (`models-manager/models.json`). ChatGPT login fetches a remote catalog that can change without a release (`models-manager/src/manager.rs`).

### 5. Loop behaviours

- **(a) Tool calling:**
  - Responses function tools (`strict:false`), freeform tools, one `mcp__<server>__` namespace per MCP server, and `tool_choice:"auto"` (`core/src/client.rs`).
  - Every current catalog model (gpt-5.6-\*, gpt-6-\*) runs in `code_mode_only`. The model writes JavaScript for one `exec` tool, and that code calls the real tools inside a V8 runtime.
  - MCP tools are deferred behind `tool_search` (`models-manager/models.json`, `code-mode-protocol/src/description.rs`, `core/src/mcp_tool_exposure.rs`).
- **(b) Parallel calls:**
  - Codex sends `parallel_tool_calls:true`, except for "responses-lite" models, where it sends false.
  - Calls start while the response is still streaming. Parallel-safe tools share a lock; the rest run one at a time.
  - MCP tools run in parallel only if they have `readOnlyHint` or the server sets `supports_parallel_tool_calls` (`core/src/tools/parallel.rs`, `core/src/tools/handlers/mcp.rs`).
- **(c) Result formatting and truncation:**
  - Truncation keeps half head and half tail, with a `…N tokens truncated…` marker.
  - The limit is 10,000 tokens, estimated at about 4 bytes per token, plus 20% when stored. `tool_output_token_limit` overrides it.
  - Shell results get an `Exit code / Wall time / Output:` header.
  - For MCP results, `structuredContent` replaces `content`, and `isError` never reaches the model (`utils/string/src/truncate.rs`, `core/src/tools/mod.rs`, `protocol/src/models.rs`).
- **(d) Tool errors:** bad JSON arguments return "failed to parse function arguments: …" to the model. An unknown tool returns `unsupported call: <name>` (`core/src/tools/handlers/mod.rs`, `core/src/tools/registry.rs`). The research found no cap on repeated errors (unverified).
- **(e) Stop conditions:** a turn ends at the first response with no tool call, unless it sets `end_turn:false`. The research found no turn, budget or wall-clock limit (unverified). Stop hooks can force the turn to continue (`core/src/session/turn.rs`).
- **(f) Max output tokens:** never sent (`codex-api/src/common.rs`). A `response.incomplete` is treated as a retryable stream error. It's retried up to 5 times, then the turn fails (`codex-api/src/sse/responses.rs`, `protocol/src/error.rs`).
- **(g) Retries:**
  - Limits: 5 stream retries and 4 HTTP retries (5xx and transport errors), with a 300 s idle timeout.
  - Backoff starts at 200 ms and doubles, with ±10% jitter. Codex follows "try again in N s" hints from the server.
  - A plain HTTP 429 is terminal.
  - Lost connections retry indefinitely, waiting 5 to 60 s (`model-provider-info/src/lib.rs`, `async-utils/src/backoff.rs`, `codex-api/src/api_bridge.rs`, `core/src/responses_retry.rs`).
- **(h) Prompt caching:** automatic. `prompt_cache_key` is the thread id, so each run gets its own key (`core/src/client.rs`).
- **(i) Compaction:**
  - Triggers at 90% of the context window, or at `model_auto_compact_token_limit`.
  - OpenAI and Azure use server-side compaction, which is opaque.
  - Other providers get a summary written by the model. It keeps at most 20k tokens of user messages and drops all tool calls and results.
  - There's no masking stage. Tool outputs at the end of the history are replaced with placeholders only when the compaction request itself overflows (`protocol/src/openai_models.rs`, `core/src/compact.rs`, `core/src/compact_remote_history.rs`).
- **(j) Reasoning:** settings for effort, summary and verbosity. Requests use `store:false` with `reasoning.encrypted_content`, and the encrypted reasoning is sent back on every request (`core/src/client.rs`).
- **(k) Built-in tools:**
  - The shell tool is `exec_command` plus `write_stdin`. Each command starts a fresh shell, so the shell isn't stateful. Only processes that are still running persist.
  - Others: apply_patch, update_plan, view_image, web_search, MCP-resource tools and spawn-agent tools (`core/src/tools/spec_plan.rs`, `core/src/tools/handlers/shell_spec.rs`).
- **(l) MCP:**
  - Servers belong to a thread, so each exec run starts its own.
  - Timeouts: 10 s for startup and 60 s per tool call, as the docs say. The source at the pinned tag uses 30 s and 300 s; see the reference configuration below. Optional servers get only 1 s before the first tool list is built.
  - The server's `instructions` become the namespace description, and tool annotations decide approvals and parallel runs (`codex-mcp/src/runtime.rs`, `codex-mcp/src/mcp/mod.rs`, `codex-mcp/src/rmcp_client.rs`).
- **(m) Skills:** loaded progressively. A catalog of name, description and path goes in a developer message, capped at 2% of the context window. The model reads `SKILL.md` when it needs it, and a `$skill` mention injects it directly (`ext/skills/src/catalog_prompt.rs`; https://learn.chatgpt.com/docs/build-skills).
- **(n) Structured output:** `--output-schema` puts a strict JSON schema on every request in the turn. `-o` writes the last message to a file. With `--json`, the `turn.completed` event carries token usage (`codex-api/src/common.rs`, `exec/src/exec_events.rs`).
- **(o) Telemetry:**
  - OTel logs, traces and metrics over OTLP. Events use Codex's own names, such as `codex.api_request` and `codex.tool_result`.
  - Only a few span attributes follow the GenAI conventions (`gen_ai.usage.*`).
  - By default, metrics go to OpenAI's analytics service (Statsig) (`otel/src/config.rs`, `core/src/config/otel.rs`).

### 6. Fit for lablet (opinion)

| Lablet's tentative choice                           | Codex                                                         |
| --------------------------------------------------- | ------------------------------------------------------------- |
| Tool-error cap off by default                       | **Supports**                                                  |
| Truncation at max_tokens is terminal, 32k default   | **Contradicts**: Codex sets no cap and retries before failing |
| OpenAI via the Responses API                        | **Supports**                                                  |
| Skills loaded progressively                         | **Supports**                                                  |
| MCP servers live across runs, with a liveness check | **Contradicts**: servers last one run                         |
| Prompt cache shared across runs                     | **Contradicts**: the key is per thread                        |
| Mask old tool outputs first, summarise later        | **Contradicts**: summary only, often server-side              |

Main obstacles to using it as a reference:

- The code-mode and deferred tool surface can only be removed with a custom model catalog.
- It runs OpenAI models only.
- Many prompt blocks are injected by default, each with its own switch.
- MCP tools without `readOnlyHint` are denied in exec unless approval is configured.
- The WebSocket transport complicates a recording proxy.
- Server-side compaction is opaque.
- Releases ship daily.
- Telemetry doesn't follow the GenAI conventions.

### 7. Verdict (opinion)

Codex is the best view of how OpenAI's own loop drives the Responses API: encrypted reasoning replay, retries, MCP annotation handling and progressive skills. That makes it a good **secondary** reference for OpenAI models. It shouldn't be the primary one, because its tool surface, compaction, MCP lifetime and OpenAI-only providers all differ from lablet, and switching them off takes a hand-edited model catalog that frequent releases keep breaking.

## Reference configuration

The configuration, then how Codex behaves in it.

### Recipe (confirmed in source, needs capture)

**Pinning.**

- Install with `npm install -g @openai/codex@0.156.1`, or use a release asset:
  - `codex-x86_64-unknown-linux-musl.tar.gz`, sha256 `aff46539a83aff86e3c62c592bce2c50d95391f9df289afaf03a50c01d14533d`
  - `codex-aarch64-apple-darwin.tar.gz`, sha256 `2bd64af14dedd47795f2f6bfd5d125cf79199acc2c7ba222144e08127111a5ca`
- Setting `model_catalog_json` makes Codex use a fixed catalog that never calls `/models` (`model-provider/src/provider.rs`, `models_manager`).
- An API key through a custom provider never reaches ChatGPT's remote catalog, and neither does an empty `CODEX_HOME` with no `auth.json`.

**`catalog.json`.** The catalog default is `gpt-6-astra` (priority 1). Only the tool-surface fields differ from the stock entry.

```json
{
  "models": [{
    "slug": "gpt-6-astra",
    "display_name": "gpt-6-astra (lablet reference)",
    "description": null,
    "default_reasoning_level": "low",
    "supported_reasoning_levels": [
      { "effort": "low", "description": "low" },
      { "effort": "medium", "description": "medium" },
      { "effort": "high", "description": "high" },
      { "effort": "xhigh", "description": "xhigh" },
      { "effort": "max", "description": "max" }
    ],
    "shell_type": "disabled",
    "visibility": "list",
    "supported_in_api": true,
    "priority": 1,
    "availability_nux": null,
    "upgrade": null,
    "model_messages": null,
    "include_skills_usage_instructions": false,
    "include_plugin_usage_instructions": false,
    "include_apps_usage_instructions": false,
    "default_reasoning_summary": "none",
    "support_verbosity": true,
    "default_verbosity": "low",
    "apply_patch_tool_type": null,
    "truncation_policy": { "mode": "tokens", "limit": 10000 },
    "supports_image_detail_original": true,
    "context_window": 272000,
    "max_context_window": 872000,
    "experimental_supported_tools": [],
    "input_modalities": ["text", "image"],
    "supports_search_tool": false,
    "use_responses_lite": false,
    "tool_mode": "direct",
    "multi_agent_version": "disabled"
  }]
}
```

**`run-codex-ref.sh`.** It writes `config.toml` into a fresh `CODEX_HOME` for each run. Your system prompt goes in `system.md` next to it; Codex trims its whitespace on load. The script passes shellcheck, and the config parses as TOML and matches `core/config.schema.json`.

```bash
#!/usr/bin/env bash
# Usage: PROXY_BASE_URL=http://127.0.0.1:8787/v1 MCP_COMMAND=/abs/server ./run-codex-ref.sh "task"
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
: "${PROXY_BASE_URL:=http://127.0.0.1:8787/v1}"
: "${MCP_COMMAND:?absolute path of the stdio MCP server}"
codex --version | grep -q ' 0\.156\.1$' || { echo "need codex-cli 0.156.1" >&2; exit 2; }
CODEX_HOME="$(mktemp -d)" && export CODEX_HOME   # no auth.json, AGENTS.md, hooks, skills, plugins
work="$(mktemp -d)"                               # empty cwd: no project AGENTS.md or .codex/
cat >"$CODEX_HOME/config.toml" <<EOF
model = "gpt-6-astra"
model_provider = "lablet_proxy"
model_catalog_json = "$here/catalog.json"
model_instructions_file = "$here/system.md"
approval_policy = "never"
sandbox_mode = "read-only"
web_search = "disabled"
include_permissions_instructions = false
include_apps_instructions = false
include_collaboration_mode_instructions = false
include_environment_context = false
project_doc_max_bytes = 0
check_for_update_on_startup = false

[model_providers.lablet_proxy]
name = "lablet-proxy"
base_url = "$PROXY_BASE_URL"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
supports_websockets = false
request_max_retries = 4
stream_max_retries = 5
stream_idle_timeout_ms = 300000

[mcp_servers.target]
command = "$MCP_COMMAND"
args = []
required = true
startup_timeout_sec = 30
tool_timeout_sec = 300
default_tools_approval_mode = "approve"
omit_tools_from = ["deferred", "code_mode"]

[agents]
enabled = false
[skills]
include_instructions = false
[skills.bundled]
enabled = false
[tools.experimental_request_user_input]
enabled = false
[history]
persistence = "none"
[analytics]
enabled = false
[feedback]
enabled = false
EOF
exec codex exec --strict-config --ephemeral --skip-git-repo-check --ignore-rules --json -C "$work" \
  --disable shell_tool --disable view_image --disable sleep_tool --disable hooks \
  --disable multi_agent --disable apps --disable plugins --disable image_generation \
  --disable goals --disable unbounded_connection_retries \
  "$1" </dev/null
```

**Environment.**

- The key comes from `OPENAI_API_KEY`, which you set yourself, or the proxy can inject it. The script never reads it.
- Set `CODEX_CA_CERTIFICATE` if the proxy terminates TLS with its own certificate authority.
- The stdio server inherits only `HOME`, `LOGNAME`, `PATH`, `SHELL`, `USER`, `LANG`, `LC_ALL`, `TERM`, `TMPDIR` and `TZ` (`rmcp-client/src/utils.rs`). Use `env` or `env_vars` to pass anything else.
- No `-c` overrides are needed. Any key above can move to `-c key=value`.

**Why the less obvious switches are there.**

- **The catalog's `tool_mode` overrides feature flags** (`core/src/tools/mod.rs`). The stock astra entry would also bring:
  - code mode;
  - `tool_search` deferral;
  - Responses Lite, which moves `instructions` and `tools` into `input` and sets `parallel_tool_calls:false`;
  - multi-agent v2 and `apply_patch`;
  - the `current_time` and `request_user_input_async` tools, via `experimental_supported_tools`.
- **Some tools are on by default:**
  - `request_user_input` stays registered unless `[tools.experimental_request_user_input]` is set to false (`core/src/config/mod.rs`).
  - The `goals` feature adds goal tools in exec (`ext/goal`).
  - The `apps` feature adds a hosted `codex_apps` MCP server (`ext/mcp/src/lib.rs`).
  - `web_search` defaults to a hosted `cached` tool.
- **`omit_tools_from`** keeps MCP tools direct even if the catalog fails to load (`core/tests/suite/code_mode.rs`).
- **Approvals:** `codex exec` forces `approval_policy=never`, which denies MCP tools without `readOnlyHint` unless the server's mode is `approve` (`exec/src/lib.rs`, `core/src/mcp_tool_call.rs`).

**What it costs.**

1. `list_mcp_resources`, `list_mcp_resource_templates` and `read_mcp_resource` are always added once any MCP server is configured. No setting removes them (`spec_plan.rs`, `add_mcp_resource_tools`).
2. **Transport and compaction:**
   - The built-in `openai` provider always tries WebSocket first. Its ID can't be overridden and the WebSocket flags have been removed, so it only falls back to HTTPS after five stream retries.
   - A custom provider sends plain `POST {base_url}/responses` with SSE.
   - Codex decides provider capabilities by checking `name == "OpenAI"`. With any other name there's no server-side compaction, and two internal fields (`internal_chat_message_metadata_passthrough`, `encrypted_function_args`) are stripped from input items (`model-provider/src/provider.rs`, `core/src/client.rs`).
   - **Likely:** naming the custom provider `"OpenAI"` brings back remote compaction over HTTP, as a `compaction_trigger` item on `/responses`, but also brings back that metadata. Not verified.
3. This is Codex's direct-tool path (the one gpt-5.5 uses), not stock gpt-6-astra behaviour.
4. `--disable unbounded_connection_retries` is a harness choice. Stock Codex retries dropped connections indefinitely, waiting 5 to 60 s.

### R1: request shape (confirmed: `core/src/client.rs` `build_responses_request`, `codex-api/src/common.rs`)

- **Fields sent:**
  - `instructions` is `system.md` exactly.
  - `tool_choice:"auto"`, `parallel_tool_calls:true`, `store:false`, `stream:true`.
  - `reasoning:{effort:"low"}`. A summary is added only if `model_reasoning_summary` isn't `none`.
  - `include:["reasoning.encrypted_content"]` and `text:{verbosity:"low"}`.
  - `prompt_cache_key` is the session id, which is new on every run, so the cache isn't keyed across runs.
  - `client_metadata` holds the installation, session, thread, turn and window ids plus an `x-codex-turn-metadata` JSON string.
- **Never sent:** `max_output_tokens`, `service_tier`, `metadata`, `truncation`, `previous_response_id`.
- **`input`:** the first request contains only `{type:"message", id:"msg_<uuidv7>", role:"user", content:[{type:"input_text"}]}`. Every injected context block renders empty (`core/src/session/world_state.rs`). Later requests replay model items verbatim (reasoning with `encrypted_content`, `function_call`, assistant messages), then the outputs.
- **`tools`:** the three resource tools (`strict:false`), then `{type:"namespace", name:"mcp__target", description, tools:[{type:"function", name, description, strict:false, parameters}]}`, sorted by name.
- **Schemas are rewritten** (`tools/src/json_schema*`):
  - Only these keywords survive: `type`, `description`, `enum`, `items`, `minItems`, `properties`, `required`, `additionalProperties`, `anyOf`, `oneOf`, `allOf`, `$ref`, `$defs`.
  - `const` becomes `enum`, and properties are sorted.
  - Schemas over 5,000 bytes are compacted, starting by stripping descriptions.
  - `outputSchema` and annotations aren't sent.

### R2: tool results (confirmed: `core/src/tools/context.rs`, `protocol/src/models.rs`, `utils/output-truncation`, `codex-mcp/src/tools.rs`)

- **Names:** calls come back as `function_call` with `namespace:"mcp__<server>"` and the bare tool `name`. Names are sanitised to `[A-Za-z0-9_]`, so `-` becomes `_`. Collisions get `_` plus 12 hex characters of SHA-1, and each name is capped at 128 characters. The flat `mcp__server__tool` form appears only in hooks and telemetry.
- **Output item:** `{type:"function_call_output", id:"fco_…", call_id, output}`.
  - If `structuredContent` is non-null, `output` is the string `"Wall time: 0.1234 seconds\nOutput:\n"` followed by compact JSON, and `content` is dropped.
  - Otherwise `output` is an array: the Wall-time header as `input_text`, then text as `input_text`, images as `input_image` (a data URL, detail `high`), audio as `input_audio`, and any other type as JSON text.
- **`isError`** never reaches the model.
- **Error text:** bad JSON arguments give `err: …`, and a failed or timed-out call gives `tool call error: …`.
- **Truncation:** the budget is 10,000 tokens × 1.2 = 12,000 tokens, counted at 4 bytes per token (48,000 bytes), including the header.
  - A string keeps its first and last 24,000 bytes around `…N tokens truncated…`.
  - In an array, text items are counted in order; the one that crosses the limit is cut in the middle, the rest become `[omitted N text items ...]`, and images are always kept.
  - `tool_output_token_limit` or a per-tool `output_token_limit` overrides the budget.

### R3: loop end and errors (confirmed: `core/src/session/turn.rs`, `core/src/responses_retry.rs`, `codex-api/src/sse/responses.rs`, `codex-api/src/api_bridge.rs`)

- **Loop end:** the turn ends at the first `response.completed` with no tool call, unless that response has `end_turn:false`. `run_turn` has no turn, token or time cap. Exec exits 0, or 1 on an error that won't be retried.
- **`response.incomplete`** is treated like a dropped stream. It's retried 5 times (0.2, 0.4, 0.8, 1.6 and 3.2 s, ±10%), then the turn fails.
- **Retry input (likely, needs capture):** tools start while the response is still streaming. On a failure, in-flight tools finish and are recorded, and the retry is rebuilt from history, so it resends the partial output rather than starting over.
- **HTTP-level retries:** 4 retries on 5xx and transport errors, starting at 200 ms and doubling; `Retry-After` is ignored. Any HTTP 429 ends the run.
- **Stream-level retries:** 5, with a 300 s idle timeout. `rate_limit_exceeded` honours the "try again in N s" hint. `server_is_overloaded`, quota errors and HTTP 400 end the run. A persistent 500 means up to 30 POSTs.
- **Context window:** `context_length_exceeded` isn't retried and doesn't trigger compaction; the turn fails.
- **Compaction is local here:**
  - It triggers when the last response's `total_tokens` plus estimated new items reaches the lower of 90% of the window (244,800 tokens) and `model_auto_compact_token_limit`, or reaches 95% (258,400).
  - It runs only mid-turn while tool calls are pending, or before a turn starts. Compaction after the final answer is off by default.
  - It's an extra request with no tools, containing the history plus `SUMMARIZATION_PROMPT`.
  - The new history is up to 20,000 tokens of user messages plus the summary. Tool calls and results are dropped (`core/src/compact.rs`).

### R4: parallel calls (confirmed: `core/src/tools/parallel.rs`, `core/src/tools/handlers/mcp.rs`)

- **Which calls overlap:** each response's calls share a lock. A tool with `readOnlyHint:true`, or any tool on a server with `supports_parallel_tool_calls=true`, takes a shared slot; the three resource tools also count as read-only. Everything else runs alone, in first-come order. There's no cap on how many run at once.
- **Order:** results come back in call order. The next request lists all calls in stream order, then all outputs in call order.

### R5: MCP lifecycle (confirmed: `codex-mcp/src/rmcp_client.rs`, `codex-mcp/src/connection_manager*`, `core/src/session/mcp_runtime.rs`; server-crash behaviour likely)

- **Start:** servers start when the session starts. Codex sends `initialize` with protocol version 2025-06-18 and client `codex-mcp-client`/0.156.1. Exec auto-cancels elicitation requests.
- **Required servers:** with `required=true`, session start waits for the server, and a failure ends exec with no model request. A non-required server gets a shared 1 s grace period and is left out of the first request if it isn't ready.
- **Timeouts:** startup 30 s and tool calls 300 s in source; the docs still say 10 s and 60 s ([config reference](https://learn.chatgpt.com/docs/config-file/config-reference)). The recipe pins both.
- **No recovery:** there's no reconnect or restart. `tools/list_changed` is only logged, so the tool list is fixed for the run. A server that dies turns later calls into error text.
- **Server `instructions`** become the namespace `description` (up to 512 KiB), so they reach the model inside `tools`, not in `instructions`.
- **What the server sees:** each `tools/call` `_meta` carries the call, thread and session ids.

### Capture checks

1. **Offline:** `codex debug models` should show only the stripped entry. `codex debug prompt-input "x"` should show a single user message. Check `codex features list` too.
2. **First POST:** every R1 field is present, no WebSocket upgrade, and no `x-openai-internal-codex-responses-lite` header. `tools` is the three resource tools plus one namespace. `instructions` is byte-identical to `system.md`.
3. **Whole run:** confirm that api.openai.com accepts namespace tools and client-assigned item ids through the proxy. The reference depends on both.
4. **Tool results:** return `structuredContent`, plain content, `isError`, and a 60 KB output. Check the header and the truncation.
5. **Parallel calls:** in one response, two read-only calls and one write call. Check server-side timestamps and the output order.
6. **Proxy fault injection:** inject `response.incomplete`, `context_length_exceeded`, repeated 500s, a 429, a stall over 300 s, and `end_turn:false`. Check retry counts, backoff, what the retry input contains, and the exit code.
7. **Compaction:** set `model_auto_compact_token_limit=20000` to force local compaction. Separately, try `name = "OpenAI"` and look for `compaction_trigger`.
8. **MCP:** a startup slower than 30 s, a tool that runs past its timeout, killing the server mid-run, and a `list_changed` notification.

## Corrections after verification

A second pass on 2026-09-25 read the source at the pinned tag again and settled or corrected these points. The matrix already reflects them.

- **Malformed arguments (profile 5d).** `failed to parse function arguments: …` is what built-in tools return, including the three MCP resource tools (`core/src/tools/handlers/mod.rs`). MCP server tools return `err: …` after the wall-time header (`core/src/mcp_tool_call.rs`).
- **Limits (profile 5d and 5e).** The source has no turn cap, wall clock, tool-error counter or loop detection, which settles the "unverified" there. An under-development `features.rollout_budget`, off by default, can end a session at a weighted token total (`core/src/rollout_budget.rs`).
- **Retry input (R3).** Confirmed, not only likely: calls that completed before a failure are drained and recorded, and each retry rebuilds its input from history (`core/src/session/turn.rs`).
- **Empty responses and refusals.** An empty response ends the turn with no special case. A policy failure ends the turn with no retry.
- **Overload.** `server_is_overloaded` ends the run, like an HTTP 429 (`codex-api/src/api_bridge.rs`).
- **Compaction (R3).** It's done by the server for a provider named `OpenAI`, and also for an Azure Responses provider (`model-provider/src/provider.rs`).
