# Claude Code

The primary reference, and the source of lablet's default profile. Pinned at Claude Code 2.1.281, released 2026-09-23 and run through the Claude Agent SDK. Python SDK 0.2.159 and TypeScript SDK 0.3.281 both bundle it. Researched 2026-09-24 and 2026-09-25 from Anthropic's docs and changelog, the SDK repositories, `anthropics/claude-code` issues and the Claude API docs. The harness is closed source, so every claim carries a confidence tag or a source, and some cells can only be settled by a capture. The "Fit for lablet" section compares against choices under consideration on 2026-09-24, several of them not yet recorded in `decisions.md`. The [matrix](matrix.md) has lablet's current behaviour and the proposals.

Claude Code changes quickly, and server-side feature flags change its behaviour even at a pinned version. Compare against it only at the pinned version, with flags off (`DISABLE_GROWTHBOOK=1`). Capture its requests with its own raw-body logging, not through a proxy: pointing `ANTHROPIC_BASE_URL` at a non-Anthropic host switches off tool search, fine-grained tool streaming and the first-byte deadline.

## Profile

### 1. Identity

- **What it is:** a Python and TypeScript library that runs the Claude Code command-line tool, shipped inside the package as a native binary, as one child process per session. It gives "the same tools, agent loop, and context management" as Claude Code [overview][loop][hosting]. Maintainer: Anthropic.
- **What's open:** only the Python wrapper, under MIT [lic-py]. The TypeScript repo holds docs and examples only, and the npm package is "All rights reserved" [lic-ts]. Claude Code, which contains the loop itself, is proprietary [lic-cc].
- **Latest version:** Claude Code 2.1.281, released 2026-09-23 [cl]. TypeScript SDK 0.3.281 and Python SDK 0.2.159 came out the same day, and both bundle Claude Code 2.1.281 [rel-ts][rel-py]. New versions ship almost daily.

### 2. Relevance

- **Adoption:** Claude Code passed $2.5B run-rate revenue and about 4% of public GitHub commits in Feb 2026 [seriesG]. The SDK had about 7.8M npm downloads in the week to 21 Sep 2026 [npm].
- **MCP host:** yes, a full one: stdio, SSE, streamable HTTP, WebSocket and in-process servers [mcp].
- **For people optimising MCP servers or skills:** it is also Anthropic's own Skills runtime [skills]. The changes it makes to MCP tools on the way to the model (deferred schemas, capped descriptions, capped results) decide how a server or skill performs [mcp]. Relevance is very high.

### 3. Providers and APIs

- Claude models only: Anthropic API, Bedrock, Google Cloud Agent Platform, Foundry, Claude Platform on AWS, or gateways that speak those formats [gw].
- Anthropic "doesn't support routing Claude Code to non-Claude models" [gw-other]. There is no OpenAI API support.

### 4. Checkability

- **Headless:** `claude -p --output-format stream-json`, or the SDK's `query()`. `--bare` skips hooks, skills, commands, subagents, plugins, MCP discovery, auto memory and CLAUDE.md [headless][cli].
- **System prompt:**
  - Replace it with `--system-prompt` or an SDK string, which "sends only what you provide". Append with `--append-system-prompt` or the preset's `append` [cli][sysprompt].
  - The SDK's default is a minimal tool-calling prompt; `-p` uses the full Claude Code prompt [sysprompt].
  - Three more injections need switching off:
    - the attribution block added to the start of the system prompt (`CLAUDE_CODE_ATTRIBUTION_HEADER=0`) [gw];
    - auto memory, which `settingSources` doesn't control (`CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`) [features];
    - the git status snapshot (`CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS=1`) [env].
  - Whether other per-turn reminders still appear under a custom prompt: unverified.
- **Tools and MCP servers:** `--tools ""` or `tools: []` removes the built-in tools. `--strict-mcp-config` or `strictMcpConfig` keeps only the MCP servers you pass [cli][ts].
- **Subagents:** leave `Agent` out of the tools and set `CLAUDE_AGENT_SDK_DISABLE_BUILTIN_AGENTS=1` [env][tools].
- **Hooks:** `--bare`, or `settingSources: []` [features].
- **Permissions:** use `dontAsk` plus allow rules. `bypassPermissions` is refused when running as root [loop].
- **Base URL:**
  - `ANTHROPIC_BASE_URL` works, but pointing it at a non-Anthropic host (such as a recording proxy) turns off three defaults: tool search, fine-grained tool streaming, and splitting the system prompt into cached blocks [env][gw][sysprompt].
  - Pin `ENABLE_TOOL_SEARCH=false` so tool schemas are sent upfront in both cases.
  - With OTel logs on, `OTEL_LOG_RAW_API_BODIES=file:<dir>` records full request and response bodies without a proxy [env].
- **Pinning:**
  - Each SDK release bundles one Claude Code build, Claude Code can be installed at a given version, and `DISABLE_AUTOUPDATER=1` stops updates [loop][setup][env].
  - Feature flags fetched from Anthropic still change behaviour at a fixed version. `DISABLE_GROWTHBOOK=1` forces the built-in defaults [env].

### 5. Loop behaviours

- **(a) Tool calling:** native Anthropic `tool_use`/`tool_result` blocks, streamed. MCP schemas are hidden behind a `ToolSearch` tool by default [loop][mcp].
- **(b) Parallel calls:** several calls per response are allowed.
  - Read-only tools run concurrently, up to 10 at once. These are Read, Glob, Grep and MCP tools marked `readOnlyHint`.
  - Edit, Write, Bash and unmarked tools run one at a time [loop][env].
- **(c) Result formatting and truncation:**
  - Bash, successful: output is sent inline up to about 30,000 characters. Beyond that it's saved to a file and the model gets the path plus a 2,000-character preview.
  - Bash, failed: the model gets a head-and-tail excerpt of about 10,000 characters [tools].
  - MCP: results over 25,000 tokens go to a file, replaced by a message naming the path. A tool can raise its own limit to 500k characters with `_meta["anthropic/maxResultSizeChars"]` [mcp].
  - Read: output is line-numbered. Large files return a first page plus a `PARTIAL view` notice [tools].
  - When a tool returns `structuredContent`, its text blocks aren't forwarded. This is documented for SDK tools; for external servers it's unverified [custom].
- **(d) Tool errors:**
  - Exceptions and `isError` results go back to the model and the loop continues [custom].
  - Arguments that fail the schema fail with `InputValidationError` [cl].
  - No cap on repeated tool errors is documented. Caps exist only for structured-output retries (5) and failed auto-compactions (3) [env][cl].
- **(e) Stop conditions:**
  - A text-only response ends the run with `success`.
  - These end it with error subtypes: `maxTurns` (counts tool-use turns, unlimited by default), `maxBudgetUsd`, running out of structured-output retries, and `error_during_execution`.
  - `stop_reason` reports `end_turn`, `max_tokens` or `refusal` [loop].
  - There's no wall-clock limit other than abort or SIGTERM [ts][headless].
- **(f) Max output tokens:**
  - The cap is set per model (64k default on Opus 4.6), with 32,000 for model IDs Claude Code doesn't recognise. `CLAUDE_CODE_MAX_OUTPUT_TOKENS` overrides it [env][cl].
  - A cut-off at `max_tokens` isn't terminal: Claude Code continues automatically. How many times it continues is unverified [cl].
- **(g) Retries:**
  - 10 retries (up to 15) with exponential backoff, on 5xx, 529, timeouts and throttling 429s.
  - It honours `Retry-After` (with its own backoff as a minimum) and `x-should-retry`.
  - It never re-sends a request after a text block or tool call has completed.
  - When a request is too long for the context window, it retries with a smaller `max_tokens`, then compacts [errors][cl][gw].
- **(h) Prompt caching:**
  - `cache_control` goes on system blocks and message entries. The order is system prompt with tools, then project context, then conversation [gw][cache].
  - A custom prompt can be split into a static and a dynamic block, each with its own cache breakpoint, on the direct API only [sysprompt].
  - The cache lasts 5 minutes with an API key; 1 hour is optional [cache].
  - The default Claude Code prompt includes the working directory, platform and git state unless `excludeDynamicSections` is set [sysprompt].
- **(i) Context management:**
  - Claude Code "clears older tool outputs first, then summarizes" [how]. It also sends the API's `context_management` beta field [gw].
  - Auto-compaction runs at the model's context limit (about 967K on 1M-token models) and can be tuned or turned off [model][env].
  - After compacting, it re-reads up to five files and re-injects skill bodies [ctx].
  - What triggers the clearing of tool outputs is unverified.
- **(j) Thinking:**
  - Adaptive thinking is the default, with effort from `low` to `max`, or a fixed `MAX_THINKING_TOKENS` budget. Thinking can't be turned off on Opus 5.5 or Fable [ts][env].
  - Thinking from earlier turns is sent back, under the API's preserved-thinking check [gw][sysprompt].
- **(k) Built-in tools and Bash:**
  - Tools: Read, Edit, Write, Bash, WebFetch, WebSearch, ToolSearch, Agent, Skill and task tools. Glob and Grep are off by default on macOS and Linux [loop][tools].
  - Each Bash command runs in a new process, so the shell isn't stateful. The working directory carries over inside the project; environment variables don't.
  - Bash timeout is 2 minutes by default, 10 at most [tools][env].
- **(l) MCP:**
  - Servers start with each session's process [hosting][sdk-mcp].
  - Remote servers are reconnected up to five times; stdio servers never are. Remote servers' tool lists are cached across sessions [mcp][sdk-mcp].
  - Server instructions are sent to the model, capped at 2,048 characters [mcp].
  - Of the tool annotations, only `readOnlyHint` changes behaviour. Tool-list change notifications are honoured [mcp][custom].
- **(m) Skills:** loaded progressively. Names and descriptions are listed within about 1% of the context window, and a skill's body loads when the model calls `Skill`. Skills come from the filesystem only [skills][sdk-skills].
- **(n) Completion:**
  - Completion is implicit: a response with no tool calls ends the run.
  - `--json-schema` or `outputFormat` adds a `StructuredOutput` tool. Its output is validated, and the model is re-prompted up to 5 times [so][env][cl].
- **(o) Telemetry:**
  - OTel metrics and events, plus traces in beta.
  - Spans are named `claude_code.interaction`, `llm_request` and `tool`. They carry some `gen_ai.*` attributes but don't follow the GenAI span naming.
  - An incoming `TRACEPARENT` is honoured [mon][obs].

### 6. Fit for lablet (opinion)

| Lablet choice                                       | Claude Code                                                                                                                                          |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tool-error cap off by default                       | Supports: no cap is documented [custom]                                                                                                              |
| `max_tokens` cut-off is terminal, 32k default       | Contradicts: it continues, and 32k is only its default for unrecognised models. Set `CLAUDE_CODE_MAX_OUTPUT_TOKENS=32000` to align the cap [cl][env] |
| OpenAI via the Responses API                        | Not applicable: Claude models only [gw-other]                                                                                                        |
| Skills loaded progressively                         | Supports [skills]                                                                                                                                    |
| MCP servers live across runs, with a liveness check | Contradicts: servers live for one session. Remote servers are reconnected and their tool lists cached [hosting][mcp]                                 |
| Prompt cache shared across runs                     | Supports, with a fixed custom prompt or `excludeDynamicSections` [cache][sysprompt]                                                                  |
| Mask old tool outputs first, summarise later        | Supports [how]                                                                                                                                       |

**Main obstacles:**

- **Closed and moving:** the harness is closed source, releases almost daily, and flags fetched from Anthropic change its behaviour. Parity has to be checked by observation at a pinned version.
- **Claude only:** it can't be a reference for OpenAI runs.
- **Hidden inputs:** it injects content into the prompt, and some defaults change when the base URL points at a proxy.
- **`max_tokens` handling:** it continues after a cut-off, where lablet stops.
- **MCP changes:** lablet has to mirror or document the description cap, results spilled to files, and deferred schemas.
- **MCP lifetime:** servers live for one session only.

### 7. Verdict (opinion)

It's the strongest primary reference for runs on Claude models: the most widely used MCP and skills host, and the behaviours lablet cares about can be configured and are documented. It can't cover OpenAI runs, and because the harness is closed and flag-driven, every parity claim needs checking against raw request logs at a pinned version.

---

[overview]: https://code.claude.com/docs/en/agent-sdk/overview
[loop]: https://code.claude.com/docs/en/agent-sdk/agent-loop
[hosting]: https://code.claude.com/docs/en/agent-sdk/hosting
[lic-py]: https://github.com/anthropics/claude-agent-sdk-python/blob/main/LICENSE
[lic-ts]: https://github.com/anthropics/claude-agent-sdk-typescript/blob/main/LICENSE.md
[lic-cc]: https://github.com/anthropics/claude-code/blob/main/LICENSE.md
[cl]: https://code.claude.com/docs/en/changelog
[rel-ts]: https://github.com/anthropics/claude-agent-sdk-typescript/releases
[rel-py]: https://github.com/anthropics/claude-agent-sdk-python/releases
[seriesG]: https://www.anthropic.com/news/anthropic-raises-30-billion-series-g-funding-380-billion-post-money-valuation
[npm]: https://api.npmjs.org/downloads/point/last-week/@anthropic-ai/claude-agent-sdk
[mcp]: https://code.claude.com/docs/en/mcp
[skills]: https://code.claude.com/docs/en/skills
[gw]: https://code.claude.com/docs/en/llm-gateway-protocol
[gw-other]: https://code.claude.com/docs/en/llm-gateway
[headless]: https://code.claude.com/docs/en/headless
[cli]: https://code.claude.com/docs/en/cli-reference
[sysprompt]: https://code.claude.com/docs/en/agent-sdk/modifying-system-prompts
[features]: https://code.claude.com/docs/en/agent-sdk/claude-code-features
[env]: https://code.claude.com/docs/en/env-vars
[ts]: https://code.claude.com/docs/en/agent-sdk/typescript
[tools]: https://code.claude.com/docs/en/tools-reference
[setup]: https://code.claude.com/docs/en/setup#install-a-specific-version
[custom]: https://code.claude.com/docs/en/agent-sdk/custom-tools
[errors]: https://code.claude.com/docs/en/errors
[cache]: https://code.claude.com/docs/en/prompt-caching
[how]: https://code.claude.com/docs/en/how-claude-code-works
[model]: https://code.claude.com/docs/en/model-config
[ctx]: https://code.claude.com/docs/en/context-window
[sdk-mcp]: https://code.claude.com/docs/en/agent-sdk/mcp
[sdk-skills]: https://code.claude.com/docs/en/agent-sdk/skills
[so]: https://code.claude.com/docs/en/agent-sdk/structured-outputs
[mon]: https://code.claude.com/docs/en/monitoring-usage
[obs]: https://code.claude.com/docs/en/agent-sdk/observability

## Loop control in detail

The docs are at code.claude.com (fetched 2026-09-25), plus the [changelog](https://code.claude.com/docs/en/changelog) ("CL x.y.z"), the SDK repos (Python SDK 0.2.159 bundles CLI 2.1.281), the Claude API docs and `anthropics/claude-code` issues. "Reverse-read" means a user's string analysis of the shipped binary, posted in an issue; it never counts as confirmed. CL 2.1.282 entries come after the pin. Several behaviours depend on server-side GrowthBook feature flags, so they can vary by account and date.

### B1 Natural end

- The loop ends on a response that has no `tool_use` blocks ([agent-loop](https://code.claude.com/docs/en/agent-sdk/agent-loop#turns-and-messages)). **confirmed**
- Result subtypes are `success`, `error_max_turns`, `error_max_budget_usd`, `error_during_execution` and `error_max_structured_output_retries`. `result` is set only on `success`. **confirmed**
- `stop_reason` is the last response's value: `end_turn`, `max_tokens`, `refusal` or `tool_deferred`, and `null` after a crash. **confirmed**
- `terminal_reason` says why the loop ended: `completed`, `max_turns`, `prompt_too_long`, `rapid_refill_breaker`, `api_error`, `malformed_tool_use_exhausted`, `budget_exhausted`, `structured_output_retry_exhausted`, and others. `success` with `is_error:true` means the last API call failed ([SDKResultMessage](https://code.claude.com/docs/en/agent-sdk/typescript#sdkresultmessage)). **confirmed**
- An empty or thinking-only `end_turn` has no documented retry or nudge. CL 2.1.251 fixed sessions getting stuck after a thinking-only turn. **needs capture**

### B2 Structured output

- `--json-schema`/`outputFormat` turns the schema into a synthesized `StructuredOutput` tool, offered beside the normal tools ([structured-outputs](https://code.claude.com/docs/en/agent-sdk/structured-outputs), [#92584](https://github.com/anthropics/claude-code/issues/92584)). **likely**
- Validation is client-side, against JSON Schema draft-07, and `format` isn't enforced. A strict (constrained-decoding) tool schema is attempted behind a remote flag, with a silent fallback. **likely**
- A failed call gets an error result naming the property, such as `must have required property 'findings'` ([#77026](https://github.com/anthropics/claude-code/issues/77026)), and the model retries. **confirmed**
- `MAX_STRUCTURED_OUTPUT_RETRIES` defaults to 5: the first attempt plus 4 retries. After that the run ends `error_max_structured_output_retries`, with the last error in `errors[]` ([env-vars](https://code.claude.com/docs/en/env-vars)). **confirmed**
- A valid call ends the run straight away, with `stop_reason:"tool_use"` and `terminal_reason:"completed"` (#77026). CL 2.1.187 stopped the model calling it again after a success. **likely**
- If the model ends in prose instead, the run ends `success` with no `structured_output` ([SDK troubleshooting](https://code.claude.com/docs/en/agent-sdk/troubleshooting#structured_output-is-none-but-the-result-says-success)). **confirmed**
- Whether a nudge comes before that, and whether sibling calls in the same response run: **needs capture**.

### B3 max_tokens

- It started in CL 2.1.0: "automatically continue when response is cut off due to output token limit". CL 2.1.269 fixed the cache after an auto-resume, and CL 2.1.271 added the "picking the thought back up" status. **confirmed**
- Claude Code re-queries with a hidden user message: "Output token limit hit. Resume directly — no apology, no recap of what you were doing. Pick up mid-thought if that is where the cut happened. Break remaining work into smaller pieces." **likely** (the text comes from a user's wire capture)
- It does this at most 3 times. An Anthropic maintainer confirmed "3 recovery rounds" on 2.1.233, and said the cut-off thinking is stored empty ([#85983](https://github.com/anthropics/claude-code/issues/85983)). **confirmed**
- No sign was found that `max_tokens` is raised for the retry. **likely**
- When the rounds run out, the turn fails with `API Error: Claude's response exceeded the N output token maximum. To configure this behavior, set the CLAUDE_CODE_MAX_OUTPUT_TOKENS environment variable.` ([#69434](https://github.com/anthropics/claude-code/issues/69434)). **likely**
- A `tool_use` cut mid-arguments seems to be treated as an unparseable call rather than executed. That path sends a hidden "Your tool call was malformed and could not be parsed. Please retry." once (reverse-read of 2.1.158, [#64589](https://github.com/anthropics/claude-code/issues/64589)). CL 2.1.251 drops the broken output from the retry. CL 2.1.281 fixed endless alternation between the two paths, which ignored `--max-turns`. **needs capture**

### B4/B5 Empty responses and refusals

- Empty responses: see B1. Dropped connections and stalled streams are handled differently:
  - Before any output, the request is retried.
  - After partial text with no tool calls, `-p` and SDK runs prompt Claude to continue, up to 3 times (CL 2.1.246, [errors](https://code.claude.com/docs/en/errors#the-response-above-may-be-incomplete)). **confirmed**
- Refusals come back as `stop_reason:"refusal"` with `stop_details`, from Fable 5.x, Opus 5.5 and Opus 5.
- If the flagged category has a fallback (biology goes to Opus 5, cybersecurity to Opus 4.8), Claude Code re-runs the request on that model. The session then stays on it, because `switchModelsOnFlag` is on by default ([model-config](https://code.claude.com/docs/en/model-config#automatic-model-fallback)). **confirmed**
- Otherwise the turn ends with `API Error: <model> can't help with this…`. That message carries `stop_reason:"refusal"` and `stop_details` (TS SDK 0.3.162), and so does the result. There is no retry on the same model. **confirmed**
- The result's `subtype` and `is_error` for a refusal: **needs capture**.

### B6/B7 Limits

- `maxTurns`, `--max-turns` and `CLAUDE_CODE_MAX_TURNS` count tool-use round trips, with no default. **confirmed**
- At the limit, the last turn's tools have run and the next model call is skipped. The run ends `error_max_turns`; the SDK raises "Reached maximum number of turns" and the CLI exits non-zero ([agent-loop](https://code.claude.com/docs/en/agent-sdk/agent-loop#turns-and-budget), [cli-reference](https://code.claude.com/docs/en/cli-reference)). **confirmed**
- Whether the hidden recovery calls count as turns: **needs capture**.
- `maxBudgetUsd` is a client-side cost estimate that includes subagents, checked after each response. The response that crosses the cap is still made and counted, then the run ends `error_max_budget_usd`. There is no default, and 0 is rejected ([cost-tracking](https://code.claude.com/docs/en/agent-sdk/cost-tracking)). **confirmed**
- There is no wall-clock cap. There is no token budget unless you set `taskBudget` (alpha, sent as `output_config.task_budget`). **confirmed**

### B8 Repeated identical calls

There is no loop or stuck detection. An Anthropic maintainer wrote on 17 Aug 2026: "there's no general loop-detection permission gate yet" ([#73307](https://github.com/anthropics/claude-code/issues/73307)). No changelog entry up to 2.1.281 adds one. **confirmed**

### A6 Output cap

- The API's max output is 128K for Fable 5.1, Opus 5.5 and Sonnet 5, and 64K for Haiku 4.5 ([models](https://platform.claude.com/docs/en/models/overview)). **confirmed**
- Model IDs Claude Code doesn't recognize get 32,000. **confirmed**
- `CLAUDE_CODE_MAX_OUTPUT_TOKENS` sets the cap for most requests. Values above the model's limit are lowered to it. Raising it shrinks the window before auto-compaction ([env-vars](https://code.claude.com/docs/en/env-vars)). **confirmed**
- Per-model defaults aren't documented. The evidence:

| Model                                    | Default `max_tokens` | Source                                                                                   |
| ---------------------------------------- | -------------------- | ---------------------------------------------------------------------------------------- |
| Opus 4.6                                 | 64K                  | CL 2.1.77                                                                                |
| Opus 4.7, Opus 4.8                       | 64K                  | reverse-read                                                                             |
| Sonnet 4.6                               | 32K                  | reverse-read                                                                             |
| Opus 5                                   | 64,000               | a user's wire capture ([#93596](https://github.com/anthropics/claude-code/issues/93596)) |
| Opus 5.5, Sonnet 5, Fable 5.1, Haiku 4.5 | unknown              | **needs capture**                                                                        |

### A7/A8 Thinking and sampling

- Claude Code sends `thinking:{"type":"adaptive"}` to 4.6-and-later models, and effort as `output_config.effort` ([gateway guide](https://code.claude.com/docs/en/llm-gateway-protocol#feature-pass-through)). **confirmed**
- Default effort is `medium` on Opus 5.5 and `high` on Fable 5.1 and Sonnet 5 ([model-config](https://code.claude.com/docs/en/model-config#adjust-effort-level)). **confirmed**
- Haiku 4.5 takes no effort setting and uses fixed-budget thinking. **confirmed** The budget is probably max_tokens − 1. **likely**
- Earlier thinking is sent back each turn: "reasoning is normally kept in the conversation history". After more than an hour idle, Claude Code sends `clear_thinking_20251015` with `keep:1` ([postmortem](https://www.anthropic.com/engineering/april-23-postmortem)). **confirmed**
- Outside that case, `context_management` appears to hold only `clear_thinking` ([#44521](https://github.com/anthropics/claude-code/issues/44521)). **likely**
- There are no temperature, top_p or top_k options. Temperature defaults to 1 (CL 2.1.31). Whether the field is sent: **needs capture**.

### A9 Parallel flag

- No evidence was found that `disable_parallel_tool_use` is set; it isn't exposed or documented ([#64237](https://github.com/anthropics/claude-code/issues/64237)). **likely**
- Read-only tools, including MCP tools with `readOnlyHint`, run concurrently, up to 10 at once. Other tools run one at a time. A failed call no longer cancels the others in its batch (CL 2.1.161).
- `tool_choice` is presumably left at auto. **needs capture**

### D1–D3 Provider errors

- Retries default to 10 (`CLAUDE_CODE_MAX_RETRIES`, capped at 15), with exponential backoff. Each attempt gets its own `API_TIMEOUT_MS`, which defaults to 600,000. **confirmed**
- The backoff formula is 500 ms × 2^(n−1), capped at 32 s, plus 0–25% jitter (reverse-read of 2.1.220, [#81330](https://github.com/anthropics/claude-code/issues/81330)). **likely**
- `CLAUDE_CODE_RETRY_WATCHDOG=1` retries 429 and 529 indefinitely and other transient errors up to 300 times, waiting at most 5 minutes between attempts. **confirmed**
- These are retried: 5xx, 529, timeouts and dropped connections before any output, stalled streams (one extra attempt), throttling 429s (not spend-limit ones), and 400s where input plus `max_tokens` exceeds the window. **confirmed**
- These are not retried: TLS certificate failures, organization policy denials, and failures after a completed block (Claude Code keeps the output and continues). **confirmed**
- With API-key auth on Opus, Fable or Mythos models, Claude Code stops retrying repeated 529s early unless `FALLBACK_FOR_ALL_PRIMARY_MODELS` is set ([errors](https://code.claude.com/docs/en/errors#automatic-retries)). **confirmed**
- `retry-after` (integer seconds) is honoured as a minimum wait, and the backoff also applies as a minimum (CL 2.1.97). A value over 60 s ends the retries at once, outside watchdog mode. `x-should-retry: true|false` is honoured ([response headers](https://code.claude.com/docs/en/llm-gateway-protocol#response-headers)). **confirmed**
- On the direct API there is a 180 s deadline for the first response byte, plus stream idle watchdogs ([network-config](https://code.claude.com/docs/en/network-config#streaming-idle-watchdogs)). Each retry emits a `system/api_retry` event with `retry_delay_ms`. **confirmed**
- If input plus `max_tokens` exceeds the window, Claude Code retries with a smaller `max_tokens`, or compacts if that can't fit. "Prompt is too long" triggers an automatic compaction and a retry. If compaction can't help, the turn ends ([errors](https://code.claude.com/docs/en/errors#prompt-is-too-long)). **confirmed**

### E1 Clearing old tool outputs

- The docs say it "clears older tool outputs first, then summarizes the conversation if needed" ([how-claude-code-works](https://code.claude.com/docs/en/how-claude-code-works#when-context-fills-up)). **confirmed**
- It's client-side. `/cost` counts a cache miss as expected "when Claude Code has itself just rewritten the conversation … by clearing old tool results" ([costs](https://code.claude.com/docs/en/costs)). `context_management` is sent, but user captures show only `clear_thinking` and `applied_edits: []`, so the API's `clear_tool_uses` isn't used. **likely**
- Reported details, not confirmed ([#42542](https://github.com/anthropics/claude-code/issues/42542), [#86075](https://github.com/anthropics/claude-code/issues/86075)): **unverified**
  - Cleared results are replaced with the placeholder `[Old tool result content cleared]`.
  - One trigger is an idle gap. Another is a count-based rule that removes results through the API's `cache_edits`. Both take their thresholds from remote flags.
  - A separate per-message budget swaps large text results for `<persisted-output>` references.
- Whether it runs in headless SDK runs by default, what triggers it, and how many results it keeps: **needs capture**. There is no documented switch for it.

### E2 Summarisation (auto-compact)

- On the Anthropic API, 1M-window models (Opus 5.5, Sonnet 5, Fable 5.1) compact at about 967K tokens (CL 2.1.247, CL 2.1.260). Haiku 4.5 compacts "at the 200K boundary" ([model-config](https://code.claude.com/docs/en/model-config#default-auto-compact-thresholds)). **confirmed**
- The exact 200K threshold (window − 33K, by analogy with 967K): **needs capture**.
- It's a separate model call that reuses the prompt cache. It sends the same system prompt, tools and history, plus a summarisation instruction as the last user message. It inherits the session's thinking setting (CL 2.1.198). It uses the fallback model chain, but never a model with a smaller window ([prompt-caching](https://code.claude.com/docs/en/prompt-caching#compacting-the-conversation)). **confirmed**
- The summary keeps requests and intent, key concepts, files with snippets, errors and fixes, pending tasks, and current work ([context-window](https://code.claude.com/docs/en/context-window#what-survives-compaction)). **confirmed**
- Afterwards Claude Code re-injects the system prompt, CLAUDE.md and memory, git status, up to 5 recently read or edited files (up to 5K tokens each), and invoked skills. Under the parity config that leaves roughly the system prompt plus the summary. **confirmed**
- The SDK emits `system/compact_boundary` with `{trigger, pre_tokens}`. Auto-compaction gives up after 3 consecutive failures (CL 2.1.76) or when context refills straight after compacting 3 times in a row (CL 2.1.89). **confirmed**
- To tune or disable it: `DISABLE_AUTO_COMPACT`, `DISABLE_COMPACT`, `autoCompactEnabled`, `--autocompact`, `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, and `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` (which can only lower the threshold). **confirmed**

### C2–C4 Tool-call errors

- An unknown tool name, or input that fails the schema, is rejected before hooks run. It comes back as a `tool_use_error` result and the loop continues ([hooks](https://code.claude.com/docs/en/hooks#posttoolusefailure)). **confirmed**
- The texts are `Error: No such tool available: <name>` and `InputValidationError: … failed due to the following issue: …`. Before CL 2.1.282 the validation error names only the first bad parameter, so 2.1.281 does that. **likely**
- For MCP tools, arguments that parse as JSON but don't match the schema appear to be forwarded unvalidated. The server's error comes back as an `is_error` result ([#74800](https://github.com/anthropics/claude-code/issues/74800)). **likely**
- `MCP_TOOL_TIMEOUT` defaults to 100,000,000 ms, about 28 hours. **confirmed**
- HTTP and SSE servers also have a per-request timer of at least 60 s. Idle timeouts are 5 minutes for network servers and 30 minutes for stdio, and don't apply to in-process SDK servers. `-p` doesn't move long calls to the background ([mcp](https://code.claude.com/docs/en/mcp)). **confirmed**
- The text the model gets on a timeout: **needs capture**.
- Bash defaults to 120 s, with a 600 s maximum.

### Capture checks

For all of these, set `CLAUDE_CODE_ENABLE_TELEMETRY=1 OTEL_LOGS_EXPORTER=console OTEL_LOG_RAW_API_BODIES=file:<dir>`. That writes one untruncated request body per attempt, tagged with `query_source`; earlier thinking is redacted, so look for its presence, not its content ([monitoring](https://code.claude.com/docs/en/monitoring-usage#api-request-body-event)). Add `--output-format stream-json --verbose`. Don't capture through an `ANTHROPIC_BASE_URL` proxy: it turns off tool search, streaming of tool-call arguments, and the first-byte deadline.

1. **Baseline request for each model.** Record `max_tokens`, `thinking`, effort, `temperature`, `tool_choice`, `context_management.edits`, `strict` and `defer_loading` on tools, and the beta headers. Also look for the attribution block and the `# Environment` system message (#93596). MCP tools are deferred behind `ToolSearch` by default; `ENABLE_TOOL_SEARCH=false` loads them up front.
2. **Output cap.** Run with `CLAUDE_CODE_MAX_OUTPUT_TOKENS=300`. Record the hidden message text and the number of rounds, whether partial text or `tool_use` is kept, whether rounds count against `--max-turns`, and the final error.
3. **Cut-off tool call.** Make an MCP call with large arguments under that same cap, and record what the model receives next.
4. **Empty reply.** On an empty or thinking-only `end_turn`, check for any nudge and record the `result` value.
5. **Structured output.** With `--json-schema`, check sibling calls next to `StructuredOutput`, the exact error text, any nudge after a prose ending, and whether any request follows a successful call.
6. **Tool errors.** Record the exact `tool_result` for invalid arguments, an unknown tool name, and an MCP timeout (`MCP_TOOL_TIMEOUT=2000`).
7. **Long runs.** Do an MCP-heavy run with an idle gap of more than an hour. Look for the clearing placeholder, `<persisted-output>`, `cache_edits` and dropped thinking, and record Haiku 4.5's first `pre_tokens`.
8. **Retries.** Point it at a mock upstream only for this test, returning 529, 500 and 429, with and without `retry-after`, and with `x-should-retry:false`. Record the `retry_delay_ms` series.
9. **Flags.** Record the feature-flag state, or compare against a run with `DISABLE_GROWTHBOOK=1`.

## Request shape and tool surface in detail

The docs were read on 2026-09-25 and cover up to 2.1.282. Python SDK 0.2.159 bundles CLI 2.1.281. TS SDK 0.3.281 states "parity with v2.1.281".

**Sources.** MCP = code.claude.com/docs/en/mcp · SDKMCP = …/agent-sdk/mcp · TSRCH = …/agent-sdk/tool-search · SP = …/agent-sdk/modifying-system-prompts · FEAT = …/agent-sdk/claude-code-features · CT = …/agent-sdk/custom-tools · LOOP = …/agent-sdk/agent-loop · ENV = …/env-vars · CLI = …/cli-reference · TR = …/tools-reference · GW = …/llm-gateway-protocol · PC = …/prompt-caching · SK = …/skills · MON = …/monitoring-usage · HK = …/hooks · CL = …/changelog · PY = github.com/anthropics/claude-agent-sdk-python @5889056 · API-TS = platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool · API-MSG = …/build-with-claude/mid-conversation-system-messages · PB = github.com/Piebald-AI/claude-code-system-prompts (third-party text extraction, tracks 2.1.281) · LIVE = observed in an unpinned Claude Code session during the research.

### A1 System prompt and injected context

A custom string replaces the whole prompt. It goes as one block, or as two blocks when it contains the `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` line (direct API only, 2.1.275+). **confirmed** (SP, CLI)

| Still reaches the request                                                                                                                                                                            | Switch                                                                                             | Tag                                                     |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| Attribution block `x-anthropic-billing-header: cc_version=…; cc_entrypoint=…; cch=…`, sent as the first system block. api.anthropic.com strips it before the model if it arrives unchanged and first | `CLAUDE_CODE_ATTRIBUTION_HEADER=0`                                                                 | confirmed (GW, ENV)                                     |
| An identity line before the custom prompt: "You are a Claude agent, built on Anthropic's Claude Agent SDK." Seen through a proxy with TS SDK 0.2.16 (claude-agent-sdk-typescript #237, March 2026)   | none documented                                                                                    | needs capture                                           |
| Environment block (cwd, OS, shell, git-repo flag, memory paths)                                                                                                                                      | none needed: preset only, `--exclude-dynamic-system-prompt-sections` is ignored for custom prompts | confirmed (SP, CLI)                                     |
| CLAUDE.md and rules, delivered as a user message                                                                                                                                                     | `settingSources: []`, `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`                                           | confirmed (SP, FEAT)                                    |
| Auto memory (read whatever `settingSources` says)                                                                                                                                                    | `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`                                                                | confirmed (FEAT)                                        |
| Git status snapshot                                                                                                                                                                                  | `CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS=1`, or run in a non-git cwd                                  | switch confirmed; where it lands needs capture          |
| A `<system-reminder>` carrying context entries (CLAUDE.md, date and similar). "Turn-start notices" cover new tools, MCP changes, date and todos (CL 2.1.281)                                         | none documented                                                                                    | needs capture (PB `system-reminder-session-context.md`) |
| Skill listing                                                                                                                                                                                        | `skills: []`, `--disable-slash-commands`, `CLAUDE_CODE_DISABLE_BUNDLED_SKILLS=1`                   | likely                                                  |
| MCP server instructions; deferred-tool list                                                                                                                                                          | none: the server supplies the first, tool search needs the second                                  | confirmed                                               |
| Todo/task reminders                                                                                                                                                                                  | Task tools are off by default on Opus 4.8 and later and are removed by `tools`                     | likely                                                  |
| Managed policy (settings, hooks, CLAUDE.md)                                                                                                                                                          | can't be disabled from the SDK                                                                     | confirmed (FEAT)                                        |

How reminders are carried depends on the model:

- On Opus 4.8/5/5.5, Fable and Mythos, harness reminders are mid-conversation `role:"system"` messages. Sonnet 5 stopped using them in 2.1.201 (CL, API-MSG).
- Otherwise they are `<system-reminder>` text in user turns, sometimes appended after tool_result content (claude-code #79649).
- `verbatim_prompts` doesn't remove this context. It only delays the turn-start context until after the first tool call (PY `types.py`).

Tag: **likely**.

### A2 Tool definitions and tool search

| `ENABLE_TOOL_SEARCH` | Behaviour (MCP, TSRCH, ENV)                                                                                                                                               |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| unset                | Every MCP tool is deferred, with no threshold. Tools load upfront instead when `ANTHROPIC_BASE_URL` is not first-party, on pre-4.5 Vertex models, and on Foundry-on-Azure |
| `true`               | Always defer; the beta header is sent even through proxies                                                                                                                |
| `auto` / `auto:N`    | Defer only when the deferrable definitions reach 10% (or N%, 0–100) of the context window                                                                                 |
| `false`              | Everything loads upfront                                                                                                                                                  |

- `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS` forces tool search off and wins over `ENABLE_TOOL_SEARCH`. Removing `ToolSearch` also turns it off. `alwaysLoad` on a server, or `_meta["anthropic/alwaysLoad"]` on a tool, exempts it.
- History: `auto` at 10% was the default from 2.1.7. **confirmed**
- **What the model sees:** tool names only, plus server instructions. SDK tools can add a one-line `searchHint`. The names arrive in a reminder that says to load schemas with ToolSearch `select:` (PB, LIVE). **confirmed** names only; reminder wording **likely**.
- **Which API feature:** it isn't Anthropic's server-side `tool_search_tool_regex`/`bm25`. It is a client-side `ToolSearch` tool plus API `defer_loading: true` definitions. ToolSearch's tool_result carries `tool_reference` blocks, which the API expands into full definitions (API-TS "custom implementation"; GW; CL 2.1.70).
  - An April 2026 capture showed the beta `advanced-tool-use-2025-11-20` (claude-code #49139). **confirmed** mechanism; header **needs capture**.
- **ToolSearch schema** (LIVE, PB): `{query: string, max_results: number (default 5)}`.
  - Query forms: `select:A,B`, keywords, and `+term` (the term must appear in the name).
  - Up to 5 matches load by default. The model sees each as a `<function>{…}</function>` line inside `<functions>`.
  - A search with no match reports servers that failed to connect (MCP). **likely**
- **Why it turns off for non-Anthropic base URLs:** most proxies don't forward `tool_reference` blocks. **confirmed** (MCP)
- **Open:** does every request's `tools` carry all MCP definitions with `defer_loading`, or only discovered ones? claude-agent-sdk-python #525 (January 2026) claims only discovered ones; CL 2.1.268 says the tool list stays byte-stable. **needs capture**

### A3 MCP tool naming

- Names are `mcp__<server>__<tool>`. Plugin servers use `mcp__plugin_<plugin>_<server>__<tool>`.
- Characters outside `[A-Za-z0-9_-]` become `_`. This is documented for plugin tool names and prompt commands; LIVE shows server "Claude Browser" becoming `mcp__Claude_Browser__…`.
- `claude mcp add` only accepts server names made of letters, digits, `-` and `_`.
- `CLAUDE_AGENT_SDK_MCP_NO_PREFIX=1` drops the prefix for SDK servers (ENV). Since 2.1.271, `select:` also accepts bare tool names.

Tags: **confirmed** for the pattern, **likely** that the same normalisation applies to non-plugin tools.

- **Length:** the API regex is `^[a-zA-Z0-9_-]{1,128}$` (platform docs, implement-tool-use). No truncation or hashing is documented. In January 2026 names over 64 characters (the limit then) were sent unchanged and got a 400 (claude-code #19882, v2.1.14).
- **Collisions after normalisation:** undocumented.

Tag: **needs capture**.

### A4 MCP server instructions

- Each server's instructions, and each tool description, is cut at 2,048 characters (introduced in 2.1.84).
- `CLAUDE_CODE_MAX_MCP_DESCRIPTION_LENGTH` changes the cap (positive integers only, 2.1.280+). **confirmed**
- Instructions load at session start alongside the tool names.
- LIVE shows a block headed `# MCP Server Instructions`, one sentence introducing it, then a `## <server>` subsection per server, all delivered as a conversation message.
- Whether it sits in `system` or in a message when the prompt is custom: **needs capture**. CL 2.1.70 fixed a cache bust caused by instructions from a server that connected late.

### A5 Prompt caching

- **Where the breakpoints go:** Claude Code puts `cache_control` on system blocks and on message entries, including mid-conversation `role:"system"` entries (GW).
  - A split custom prompt gets one breakpoint per block (SP).
  - The static part is "cached globally" (CL 2.1.275).
  - The 2.1.110 capture showed system blocks marked `{"type":"ephemeral","ttl":"1h","scope":"global"}` with beta `prompt-caching-scope-2026-01-05`, and messages marked `{"ttl":"1h"}` (#49139).
  - Deferred tools can't carry `cache_control` (API-TS).
  - Still unknown: how many breakpoints (the API allows 4), whether tools get one, and whether it marks the last message or the last two. **needs capture**
- **TTL:** 5 minutes for API-key and cloud-provider auth. 1 hour for the main conversation on a subscription within plan usage.
  - Overrides: `CLAUDE_CODE_PROMPT_CACHE_TTL` and `CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL` (`5m` or `1h`, 2.1.242+), `ENABLE_PROMPT_CACHING_1H`, `FORCE_PROMPT_CACHING_5M` (PC). **confirmed**
- **What keeps the prefix identical across runs:**
  - The custom prompt is static.
  - The attribution block is stripped by the API and has been stable within a conversation since 2.1.181.
  - Deferred tools sit outside the cached prefix.
  - Per-run content (date, git snapshot, deferred list, reminders) is in `messages`.
  - Cache entries are per model. Changing effort invalidates the cache on most models, and fast mode adds a header that is part of the cache key (PC).
  - **confirmed**; that two runs actually share the system and tools cache still **needs capture**.

### C5 Output size limits

- **Bash** (TR):
  - A successful result stays inline up to about 30,000 characters. Beyond that the model gets a file path plus the first 2,000 characters.
  - A failure stays inline up to about 10,000 characters. Beyond that it gets a head-and-tail excerpt and no path.
  - `BASH_MAX_OUTPUT_LENGTH` sets the read-back window: 30,000 by default, 150,000 maximum.
  - The `bashOutputMaxChars` setting sizes both, up to 128,000 (2.1.261+).
  - **confirmed**
- **MCP** has two layers (PY `tests/test_mcp_large_output.py`, which says it was confirmed against Anthropic's internal CLI source on 2026-03-27):
  1. A token limit, `MAX_MCP_OUTPUT_TOKENS`, default 25,000. The 10,000-token warning is shown in the terminal only. An over-limit result is saved under the session's `tool-results` directory. The model gets an error-style line giving the character and line counts and the saved path (third-party dev.to measurement on 2.1.273).
  2. A persist threshold of 50,000 characters. The result becomes `<persisted-output>` containing `Output too large (…KB). Full output saved to: <path>` and `Preview (first 2KB):`.
  - `_meta["anthropic/maxResultSizeChars"]` raises the persist threshold up to 500,000 characters and bypasses the token layer for text (2.1.91, 2.1.98).
  - Results containing images stay under the token limit.
  - With no Read or Bash tool, the model can't open the saved file.
  - Tags: **confirmed** thresholds, **likely** wording. Whether the spilled result carries `is_error`: **needs capture**.
- **Read** (TR, LIVE):
  - Reads 2,000 lines by default.
  - The default token cap is undocumented; `CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS` overrides it.
  - A whole-file read over the cap returns the first page with a "PARTIAL view" notice.
  - PDFs over 10 pages need `pages`, at most 20 per call.

### C6/C7 MCP result shaping

**External servers:**

- Text blocks pass through as text; images pass through as images.
- If `structuredContent` is set, the model gets the JSON plus any image or resource blocks, and text blocks are dropped (CT; CL 2.0.21, 2.1.128). This is stated on the SDK page; that external servers behave the same is **likely**.
- `resource_link` becomes a text block with name, URI and description; an embedded text resource becomes text.
- `isError: true` takes the failure path:
  - PostToolUseFailure fires, and its `error` string matches what the model receives.
  - Multiple error blocks are joined (2.1.89) and long strings are middle-truncated (HK).
  - Expect a string result with `is_error: true`. **likely**
- Content `_meta` keys under `com.anthropic/` are dropped (TS SDK 0.3.282).

**In-process (SDK) tools:**

- Python `@tool` forwards only `content` and `is_error` (no `structuredContent`). It flattens resource links and text resources to text, and drops binary resources, audio and unknown blocks.
- The Python SDK validates arguments (`Input validation error: …`). Exceptions and unknown tools come back as `isError` results.
- Server notifications (list_changed, progress, logging) are dropped (PY `__init__.py`, `sdk_mcp_bridge.py`).
- TS `tool()` supports `structuredContent`, saves audio to disk and sends the path.

Tag: **confirmed**.

### C1 Parallel execution

- Read-only built-ins and MCP tools with `readOnlyHint: true` run concurrently. Everything else runs sequentially, and custom tools default to sequential (LOOP, CT).
- The cap is `CLAUDE_CODE_MAX_TOOL_USE_CONCURRENCY`, default 10, shared with subagents (ENV).
- Since 2.1.161 a failing call no longer cancels the others in the batch.

Tag: **confirmed**.

- How mixed batches are grouped, and whether tool_results keep call order, is undocumented. Third-party source analyses say consecutive safe calls form a group and results keep block order. **needs capture**

### F1–F5 MCP lifecycle

- **F1 start and lifetime:**
  - Servers from `mcpServers` start with the CLI subprocess (one per SDK session) and live until it exits.
  - Stdio servers connect 3 at a time (`MCP_SERVER_CONNECTION_BATCH_SIZE`).
  - The first turn waits for stdio servers, up to `MCP_TIMEOUT` or `CLAUDE_CODE_MCP_STARTUP_WAIT_MS` (2.1.274+).
  - **confirmed** (SDKMCP, ENV)
- **F2 timeouts:**
  - Startup: `MCP_TIMEOUT`, 30,000 ms.
  - Tool call: `MCP_TOOL_TIMEOUT`, 100,000,000 ms (about 28 h). A per-server `timeout` of at least 1,000 ms overrides it.
  - Idle: `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`, 30 minutes for stdio, 5 minutes for remote servers, none for SDK servers.
  - HTTP/SSE requests also have a per-request timer: the largest of 60 s, the tool timeout and `MCP_TIMEOUT`.
  - Calls running past 2 minutes move to the background (`CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS`), except in non-interactive mode unless `CLAUDE_AUTO_BACKGROUND_TASKS=1`.
  - **confirmed**
- **F3 reconnect:**
  - Stdio servers are never reconnected.
  - Remote servers get 5 reconnect attempts, with backoff starting at 1 s and doubling.
  - A remote server's first connection is retried 3 times on transient errors; discovery requests also 3 times.
  - SDK servers: a failed handshake is retried every turn, but a server that dies after a good handshake stays down (PY).
  - **confirmed**
- **F4 annotations that change behaviour:**
  - `readOnlyHint` controls parallelism. The other hints are informational.
  - `_meta` keys: `anthropic/maxResultSizeChars`, `anthropic/alwaysLoad`, and `anthropic/requiresUserInteraction` (which is denied under `dontAsk`).
  - Tools with invalid schemas are excluded, and schemas with a root-level `anyOf` are rewritten, only when feature flags are fetched.
  - **confirmed**
- **F5 `tools/list_changed`:**
  - Triggers a refresh; a failed refresh keeps the previous list (2.1.214+).
  - With tool search on, new names are announced on the next request.
  - Python in-process servers can't send it.
  - **confirmed**

### G1/G2 Skills

- **G1 listing:**
  - Every skill name, plus `description` and `when_to_use` capped at 1,536 characters each (`skillListingMaxDescChars`), goes in a system-reminder, not in the Skill tool's description (PB 2.1.218, LIVE).
  - The budget is 1% of the context window, falling back to 8,000 characters (`skillListingBudgetFraction`, `SLASH_COMMAND_TOOL_CHAR_BUDGET`).
  - On overflow, descriptions of the least-invoked skills go first; names always stay.
  - The listing isn't re-injected after compaction.
  - **confirmed** (SK, ENV)
- **G2 what the Skill tool returns:**
  - The rendered SKILL.md is added as one message, tagged `<command-name>`, and stays in context.
  - Invoking the same skill again with identical content adds only a short "already loaded" note.
  - A skill outside the `skills` list is rejected with an allowlist error.
  - The exact tool_result text **needs capture**.

### Reference configuration (provisional)

```python
# pip install claude-agent-sdk==0.2.159   # bundles Claude Code 2.1.281
import asyncio
from claude_agent_sdk import query, ClaudeAgentOptions
SRV, TS = "undertest", True               # TS=False: tool-search-off variant
opts = ClaudeAgentOptions(
  system_prompt=open("prompt.md").read(),
  tools=["ToolSearch"] if TS else [],     # no other built-ins
  mcp_servers={SRV: {"type": "stdio", "command": "/abs/server", "args": []}},
  strict_mcp_config=True, allowed_tools=[f"mcp__{SRV}__*"],
  permission_mode="dontAsk", setting_sources=[], skills=[],
  model="<full model id>", effort="high", cwd="/abs/empty-non-git-dir",
  extra_args={"disable-slash-commands": None},
  env={"ENABLE_TOOL_SEARCH": "true" if TS else "false",
    "CLAUDE_CONFIG_DIR": "/abs/fresh-config",
    "CLAUDE_CODE_ATTRIBUTION_HEADER": "0", "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
    "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1", "CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS": "1",
    "CLAUDE_AGENT_SDK_DISABLE_BUILTIN_AGENTS": "1", "CLAUDE_CODE_DISABLE_BUNDLED_SKILLS": "1",
    "CLAUDE_CODE_DISABLE_POLICY_SKILLS": "1", "ENABLE_CLAUDEAI_MCP_SERVERS": "false",
    "DISABLE_AUTOUPDATER": "1", "DISABLE_GROWTHBOOK": "1", "DISABLE_TELEMETRY": "1",
    "DISABLE_ERROR_REPORTING": "1", "CLAUDE_CODE_SIMPLE_SYSTEM_PROMPT": "0",
    "MCP_SDK_GENERATION": "v2", "MCP_PROTOCOL_NEGOTIATION": "legacy",
    "CLAUDE_CODE_PROMPT_CACHE_TTL": "5m", "MAX_MCP_OUTPUT_TOKENS": "25000",
    "CLAUDE_CODE_MAX_TOOL_USE_CONCURRENCY": "10", "CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS": "0",
    "DISABLE_AUTO_COMPACT": "1", "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
    "OTEL_LOG_RAW_API_BODIES": "file:/abs/bodies", "OTEL_LOGS_EXPORTER": "otlp",
    "OTEL_METRICS_EXPORTER": "none", "OTEL_EXPORTER_OTLP_PROTOCOL": "http/protobuf",
    "OTEL_EXPORTER_OTLP_ENDPOINT": "http://127.0.0.1:4318"})
async def main():
    async for m in query(prompt=open("task.md").read(), options=opts): print(m)
asyncio.run(main())
```

- **Auth:** the human exports `ANTHROPIC_API_KEY` in their own shell; the Python SDK merges `env` over the inherited environment.
- **CLI equivalent:** `claude -p --output-format stream-json --verbose --system-prompt-file prompt.md --tools ToolSearch --mcp-config cfg.json --strict-mcp-config --allowedTools 'mcp__undertest__*' --permission-mode dontAsk --setting-sources= --disable-slash-commands --model … --effort high`, with the same env.
- **Why `["ToolSearch"]`:** `tools=[]` probably removes ToolSearch too (it is a built-in, and removing it turns tool search off). **needs capture**
- **Why not `--bare`:** it registers only shell and file tools, and whether ToolSearch survives is unverified.
- **Base URL:** leave `ANTHROPIC_BASE_URL` unset; setting it flips the tool-search default.
- **What the body log gives you:**
  - Files: `<dir>/<uuid>.request.json`, `<dir>/<request_id>.response.json` and `index.jsonl` (2.1.274+).
  - Extended thinking is redacted, and headers are not included (MON).
  - To see headers without changing the base URL, use `HTTPS_PROXY` with a mitm CA trusted through `NODE_EXTRA_CA_CERTS`.
- **Needed for determinism:**
  - Pinned SDK/CLI version, full model id, effort and thinking.
  - A fresh `CLAUDE_CONFIG_DIR` and an empty non-git cwd.
  - API-key auth (a subscription gets the 1-hour TTL).
  - Feature flags off, so code defaults apply. This also turns off invalid-schema exclusion and can skip tools with a root-level `anyOf`. Without flags the MCP v2 runtime becomes the default, hence the `MCP_SDK_GENERATION` and `MCP_PROTOCOL_NEGOTIATION` pins.
  - `CLAUDE_CODE_SIMPLE_SYSTEM_PROMPT=0` pins which variant of tool descriptions you get.
  - No `max_budget_usd`: PB lists a budget reminder that may be injected when it is set.

### Capture checks

1. `system`: block count, the attribution block with and without `=0`, the SDK identity line, and `cache_control` (ttl, scope) on each block.
2. `tools`: whether ToolSearch is present with `tools=[]` versus `["ToolSearch"]`; whether MCP definitions carry `defer_loading` on every request or appear only after discovery; any breakpoint on tools.
3. First user turn: date, git status, the context reminder, deferred-list wording and the MCP instructions block. Check whether reminders are `role:"system"` messages on Opus 5.5 and `<system-reminder>` text on Sonnet 5.
4. A ToolSearch round trip: its input, and a result made of `tool_reference` blocks. Also look for reminders added after each batch of tool results (CL 2.1.268 mentions one).
5. Parallel calls: a batch of read-only calls with staggered latencies plus one write call. Check the grouping and the order of tool_results.
6. MCP results: `structuredContent` plus text, `resource_link`, a binary resource, `isError` (look for the `is_error` flag), results of 49,999 and 50,001 characters, and a result over 25,000 tokens.
7. Names: a tool called `get.data`, names of 70 and 130 characters, and two servers that collide after normalisation.
8. Two identical runs less than 5 minutes apart: check `cache_read` on system and tools.
9. `anthropic-beta` values and the `metadata` field, captured through the proxy.

### Using the reference configuration

The configuration above is provisional until a capture confirms it. It switches auto-compaction off (`DISABLE_AUTO_COMPACT=1`), which suits short request-shape captures. Leave compaction on for outcome comparisons, because compaction is part of the behaviour being compared. Check each variable against the pinned version's docs before a capture, since several were introduced recently.

## Corrections after verification

A second pass on 2026-09-25 checked the matrix's claims against the primary sources and found these errors in the sections above. The matrix already reflects them.

- **Structured output (profile 5n).** The model gets a first attempt plus 4 retries, not 5 retries (env-vars, `MAX_STRUCTURED_OUTPUT_RETRIES`).
- **Tool errors (profile 5d).** Structured-output retries and auto-compaction aren't the only caps. Repeated unparseable tool calls end the run with `terminal_reason: malformed_tool_use_exhausted` (TypeScript SDK `SDKResultMessage`).
- **Output cut off at `max_tokens` (B3).**
  - The cap of 3 resume rounds is better sourced to the constant found in [#64589](https://github.com/anthropics/claude-code/issues/64589), finding 1. [#85983](https://github.com/anthropics/claude-code/issues/85983) reports 3 rounds seen in one test.
  - The malformed-call retry needs `stop_reason` to be `tool_use`, and a cut-off response has `max_tokens`. So a tool call cut off mid-arguments probably takes the resume path, not the malformed-call path. Changelog 2.1.281 treats the two as separate paths.
- **Retries (D1–D3).** `FALLBACK_FOR_ALL_PRIMARY_MODELS` doesn't turn off the early stop on repeated 529s. It extends that stop to every model when no fallback model is set. It's documented in env-vars, not errors.
- **Summarisation (E2).** The system prompt isn't re-injected after compaction; it stays. CLAUDE.md, memory, git status, recent files and invoked skills are re-injected. The docs' "200K boundary" wording isn't about Haiku 4.5, so its threshold needs capture.
- **MCP timeouts (F1–F5).** The per-request timer for HTTP and SSE servers takes the largest of 60 s, the tool timeout and `MCP_TIMEOUT`. An unset `MCP_TOOL_TIMEOUT`'s 28-hour default doesn't enter that comparison. A call with no response or progress also stops after 30 minutes (stdio) or 5 minutes (network servers).
- **Skills (G1).** The 1,536-character cap applies to each skill's `description` and `when_to_use` combined, not to each separately. That the listing goes in a reminder comes from a third-party extraction and an unpinned session, so it's likely, not documented.
- **Output cap (A6).** The 64,000 figure for Opus 5 comes from a user's capture through a proxy, on versions 2.1.237 and 2.1.268 rather than the pinned 2.1.281.
