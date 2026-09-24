# Field evidence

Researched 2026-09-24 and 2026-09-25. Five loops that break ties where the references are silent or disagree, followed by a screen of thirteen more. Each profile was compiled by a research agent from the sources it cites. The "Fit for lablet" section compares against choices under consideration on 2026-09-24, several of them not yet recorded in `decisions.md`. The [matrix](matrix.md) has lablet's current behaviour and the proposals.

## OpenCode

Paths: a bare path is under `packages/opencode/src/` at [v1.18.32](https://github.com/anomalyco/opencode/tree/v1.18.32); `core/` means `packages/core/src/`; a path marked "v2:" is at [v2.0.16](https://github.com/anomalyco/opencode/tree/v2.0.16).

### 1. Identity

- OpenCode is an open-source TypeScript coding agent (terminal UI, `run`, server, desktop app). It is made by Anomaly Innovations, the company behind SST ([org rename](https://x.com/thdxr/status/2007199285251842478)). MIT licence; the full source is public.
- **Latest stable: v1.18.32, 2026-09-21.** This is the GitHub "Latest" release, npm `opencode-ai@latest`, and what the [changelog](https://opencode.ai/changelog) and [docs](https://opencode.ai/docs) describe. It is built on AI SDK 6.0.168.
- **2.0 line: v2.0.16, 2026-09-24.** 2.0.0 shipped on 2026-09-11.
  - It is a rewrite on a separate `v2` branch, published as npm `@opencode/cli`, with git tags but no GitHub Releases and its own [docs](https://opencode.ai/v2/docs).
  - The homepage installer now installs v2, and v2 replaces the v1 binary ([migrate-v1](https://opencode.ai/v2/docs/migrate-v1/)).
  - v2 replaces the AI SDK with its own provider layer (v2: `packages/ai/src/protocols/`), runs one shared background server, and breaks the plugin and server APIs.
  - Both lines still get releases. v2 had 17 releases in 13 days.

### 2. Relevance

- About 210k GitHub stars. [The homepage](https://opencode.ai) claims 16M monthly developers.
- It is a full MCP host: local and remote servers, OAuth, resources, and per-agent tool patterns ([MCP docs](https://opencode.ai/docs/mcp-servers/)). It also loads SKILL.md skills ([skills](https://opencode.ai/docs/skills/)).
- Harbor, an agent-evaluation framework, drives it with `opencode run --format=json` ([opencode.py](https://github.com/harbor-framework/harbor/blob/main/src/harbor/agents/installed/opencode.py)).
- A study of agent harnesses found about 40× differences in tokens per solved task between OpenCode and Goose, and 23–30× between OpenHands and Goose ([arXiv 2607.22585](https://arxiv.org/abs/2607.22585)).

### 3. Providers, and whether the loop is the same across them

It supports 75+ providers through the models.dev catalogue and AI SDK packages. These include Anthropic Messages, OpenAI **Responses** (`provider/provider.ts`), OpenAI-compatible Chat Completions, Gemini and Bedrock.

**The loop changes with the model:**

- The base system prompt is chosen from the model ID: anthropic, gpt, codex, gpt-astra, beast, gemini, kimi and others (`session/system.ts`).
- GPT models get `apply_patch` instead of `edit`/`write` (`tool/registry.ts`).
- Tool schemas are rewritten per provider, and OpenAI tools are sent with `strict:false` (`provider/transform.ts`, `session/llm/request.ts`).
- Default temperature and sampling, caching and reasoning settings differ by model family (`provider/transform.ts`).

### 4. Checkability for a reference configuration

- **Headless:** `opencode run --format json` streams events. Alternatively, start `opencode serve` and use `run --attach` ([CLI](https://opencode.ai/docs/cli/)).
- **System prompt:** an agent `prompt` **replaces** the base prompt, but four things are always appended (`session/llm/request.ts`, `session/prompt.ts`):
  - an environment block with model ID, working directory, git status, platform and date
  - AGENTS.md or CLAUDE.md
  - MCP server instructions
  - the skills list

  Only a plugin hook can remove them. Otherwise, use a clean directory, `OPENCODE_CONFIG_DIR` and `OPENCODE_DISABLE_CLAUDE_CODE=1`.
- **Tools:** restrict them with `permission` patterns, where the last match wins, e.g. `"*": "deny"` then `"srv_*": "allow"`. MCP tools are named `server_tool` (`permission/index.ts`, `mcp/catalog.ts`).
- **Sub-agents and plugins:** deny `task` to remove sub-agents. Set `OPENCODE_DISABLE_DEFAULT_PLUGINS` and configure no plugins.
- **Hidden call:** pass `--title`, or OpenCode makes an extra model call to generate a session title (`session/prompt.ts`).
- **Permission prompts:** `run` denies the question and plan tools and rejects every "ask" prompt; `--auto` approves them instead (`cli/cmd/run.ts`).
- **Recording proxy:** set `provider.<id>.options.baseURL` ([providers](https://opencode.ai/docs/providers/)).
- **Pinning:** install `opencode-ai@1.18.32` and set `autoupdate: false`. Model limits come from a live catalogue at models.opencode.ai; freeze them with `OPENCODE_MODELS_PATH` (`core/models-dev.ts`).

### 5. Loop behaviours (v1 unless marked v2)

a. **Tool calling:** each AI SDK `streamText` call is one step, and OpenCode runs the loop. Repair lower-cases a wrong tool name; otherwise the call goes to an `invalid` tool that returns the error (`session/llm.ts`, `tool/invalid.ts`).

b. **Parallel calls:** they aren't forced, but the prompts encourage them (`session/prompt/anthropic.txt`). The AI SDK starts each tool without waiting for the others ([source](https://github.com/vercel/ai/blob/ai%406.0.168/packages/ai/src/generate-text/run-tools-transformation.ts)). There is no read/write distinction. v2 also runs each call concurrently (v2: `core/session/runner/step.ts`).

c. **Result formatting and truncation:** output is capped at 2,000 lines or 50 KB (setting `tool_output`). It keeps the head, except bash, which keeps the tail. The full output is saved to a file and a hint is added (`tool/truncate.ts`). MCP text blocks are joined with blank lines; images become attachments (`session/tools.ts`).

d. **Errors:** invalid arguments return "…rewrite the input" (`tool/tool.ts`), and an MCP `isError` becomes a tool error (`mcp/catalog.ts`). There is no cap on errors. Three identical calls in a row trigger a `doom_loop` permission prompt (`session/processor.ts`). `run` rejects it, which ends the run. v2 has no doom-loop check ([permissions](https://opencode.ai/v2/docs/permissions/)).

e. **Stop conditions:** the run stops when a response ends without tool calls, when a permission is rejected, or on a content-filter stop (`session/prompt.ts`). The steps cap is unlimited by default; at the cap, a text-only instruction is added. The research found no token or cost budget and no run timeout (unverified). Provider header and stream timeouts default to 5 minutes (changelog, 1.18.27).

f. **Max output tokens:** the smaller of the model's limit and 32,000, overridable by an environment variable (`provider/transform.ts`). A cut-off response (finish reason `length`) gets no special handling, so the loop simply ends.

g. **Retries:** up to 5, waiting 2 s × 2ⁿ plus 25% jitter, at most 30 s without headers. `retry-after` and `retry-after-ms` are honoured with no cap; the SDK's own retries are off (`session/retry.ts`, `session/llm.ts`). v2 retries 10 times with gaps of at most 10 s, and caps retry-after at 15 minutes (v2: `core/session/runner/retry.ts`).

h. **Prompt caching:** for Claude models, cache breakpoints go on the system prompt and the last two messages. For OpenAI, Azure, xAI and Mistral, the cache key (`promptCacheKey`) is the session ID. Bedrock uses its cachePoint marker (`provider/transform.ts`).

i. **Context management:** automatic compaction starts at the input limit minus the smaller of 20k and max output (`session/overflow.ts`). It summarises older history, keeps the most recent 2k–15k tokens, and continues on its own (`session/compaction.ts`).

- Clearing old tool outputs is opt-in (`compaction.prune`, default false) and runs only after the loop ends.
- v2 keeps only summary checkpoints and has no clearing ([compaction](https://opencode.ai/v2/docs/compaction/)).

j. **Reasoning:**

- GPT-5 defaults to medium effort, with summaries, `store:false` and encrypted reasoning.
- Claude 5.1+ gets adaptive thinking by default; earlier Claude models need `--variant`.
- Reasoning is replayed with its signatures when the model is unchanged, and as plain text after a model switch (`provider/transform.ts`, `session/message-v2.ts`).

k. **Built-in tools:** bash, read, glob, grep, edit/write or apply_patch, task, webfetch, todowrite, skill, question, and websearch for some providers. Each bash call starts a fresh process, so no shell state carries over; it takes a working-directory argument and has a 2-minute timeout (`tool/shell.ts`).

l. **MCP:**

- Transports are stdio, or Streamable HTTP with an SSE fallback.
- Servers live for one project instance, which means one `run` unless you attach to `serve`.
- A server that closes is marked failed; there is no ping or reconnect.
- Server instructions are added to the prompt. Tool annotations are ignored. Tool-list change notifications are handled.
- The connect/list timeout is 30 s, although the docs say 5 s (`mcp/index.ts`, `mcp/catalog.ts`).
- v2 reconnects expired sessions (v2: `core/mcp/index.ts`).

m. **Skills:** loaded progressively. Names and descriptions go in the system prompt, and the `skill` tool loads SKILL.md when needed (`session/system.ts`, [skills](https://opencode.ai/docs/skills/)).

n. **Structured output:** through the SDK or server, `format: json_schema` adds a `StructuredOutput` tool the model must call ([SDK](https://opencode.ai/docs/sdk/), `session/prompt.ts`). `run` has no option for it.

o. **Telemetry:** setting `OTEL_EXPORTER_OTLP_ENDPOINT` exports OpenCode's internal spans and logs over OTLP (`core/observability/otlp.ts`). Model-call spans from the AI SDK need `experimental.openTelemetry`, and carry only some of the older `gen_ai.*` attributes (`session/llm.ts`, [AI SDK](https://ai-sdk.dev/docs/ai-sdk-core/telemetry)). v2 ignores that setting ([migrate-v1](https://opencode.ai/v2/docs/migrate-v1/)).

### 6. Fit for lablet (opinion)

| Lablet's tentative choice                         | OpenCode                                                                                           |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Tool-error cap off by default                     | Supports; the doom-loop check only catches identical repeats                                       |
| Truncation at max_tokens is terminal, 32k default | Supports, exact match                                                                              |
| OpenAI via Responses API                          | Supports                                                                                           |
| Skills loaded progressively                       | Supports                                                                                           |
| MCP servers live across runs, with liveness check | Partial: only via `serve` or v2's shared server; no liveness ping found                            |
| Prompt cache shared across runs                   | Partial: OpenAI cache key is per session; the prompt contains the date and working directory       |
| Mask old tool outputs first, summarise later      | Contradicts: summarises by default; clearing is opt-in, runs after the run ends, and is gone in v2 |
| Loop doesn't change with the provider             | Contradicts (§3)                                                                                   |

Main obstacles:

1. **The v1→v2 switch.** The website installs v2 while GitHub still marks v1 as Latest. v2 is two weeks old and [reloads config mid-session](https://anoma.ly/notes/opencode-reloaded/).
2. **Model-dependent behaviour.** Prompts, tools and reasoning settings vary per model.
3. **Unremovable prompt content.** The injected environment block and instructions can't be switched off by config.
4. **Moving model limits.** They come from a live catalogue unless frozen.
5. **Thin telemetry.** A recording proxy is needed for tokens and timing.

### 7. Verdict (opinion)

OpenCode is a strong secondary reference: it is the most-used open, multi-provider MCP host, its source is readable, and four of its defaults match lablet's. It is not suitable as the primary reference, because it deliberately changes its loop for each model family and is in the middle of a rewrite; pin v1.18.32 now and profile v2 again once GitHub marks it Latest.

## GitHub Copilot CLI

### 1. Identity

- **What it is:** GitHub's terminal coding agent, generally available since 2026-02-25 [blog][ga]. The CLI licence is proprietary: unmodified redistribution only, no derivative works, no public source [lic][lic].
- **SDK:** MIT-licensed [lic][sdklic]. It is a JSON-RPC client in six languages, and its Rust crate bundles the CLI [blog][sdkga]. The CLI runs the loop [sdk][loop].
- **Versions to pin:** CLI **1.0.88** (2026-09-22; latest prerelease 1.0.89-1 from 2026-09-23) [rel][rel]. SDK **v1.0.14** (2026-09-16), which bundles CLI 1.0.85 [sdk][pkg14]. The next SDK preview is v1.0.15-preview.2 (2026-09-23) [rel][sdkrel].

### 2. Relevance

- **Adoption:** The research found no public user count for the CLI alone. For Copilot as a whole:
  - 4.7M paid subscribers in January 2026 [news][tc].
  - 21% work adoption in mid-2026, against 39% for Claude Code [survey][jb].
  - Microsoft's internal rollout of the CLI covered tens of thousands of engineers [paper][msr].
- **MCP:** a full MCP host with a built-in `github-mcp-server` [doc][ref-mcp]. Its tool search matches tools by name, description and parameters, so an MCP server's tool surface directly affects whether its tools get found [doc][ts].
- **Skills:** it reads Agent Skills, including from `.claude/skills` [doc][ref-skills].

### 3. Providers and provider invariance

BYOK supports three provider types [doc][byok], [sdk][sdkbyok]:

- `openai` (the default) uses Chat Completions and also covers Ollama and vLLM. `COPILOT_PROVIDER_WIRE_API` switches it to the Responses API, and WebSocket Responses is supported [cl][cl892].
- `azure` covers Azure OpenAI.
- `anthropic` uses the Messages API.

Models must support tool calling and streaming. Signing in to GitHub is optional [blog][byokcl].

**The loop is not provider-invariant:**

- The model ID picks the "agent configuration (tools, prompts, reasoning behavior)" [sdk][t3179].
- Some models get `apply_patch` instead of `edit` [doc][ref-tools].
- Tool search runs only with Claude and GPT-5.4+ [doc][ts].
- The Anthropic thinking mode varies per model [cl][cl783].
- Since 1.0.81, BYOK Chat Completions requests reportedly send `temperature: 0` [#4950][i4950].
- Models missing from the built-in catalogue get default token limits [#3118][i3118].

### 4. Checkability

- **Headless:** `copilot -p … --output-format json --no-ask-user --allow-all-tools` [doc][ref-opts], or the SDK's `sendAndWait` [sdk][loop].
- **System prompt:** the CLI can only turn off custom instructions [doc][ref-opts]. The SDK can append to the prompt, replace it, or customise any of its 12 sections [sdk][t1094], [sdk][t1190]. The runtime still injects:
  - the current date and time, which a hook can strip [sdk][upt];
  - notices when shell commands complete [sdk][e8432];
  - MCP server instructions [doc][ref-opts].
- **Restricting tools:**
  - `--available-tools` takes precedence over `--excluded-tools`.
  - Each MCP server has a tool filter, and `--disable-builtin-mcps` removes the built-in servers [doc][ref-opts], [doc][ref-mcp].
  - The SDK's `mode:"empty"` requires explicit tool allowlists [sdk][mt].
  - `toolSearch:false` stops tools being deferred [doc][ts].
- **Switching features off:** for sub-agents, withhold the `task` tool and don't pass `--fleet` [doc][ref-tools]. For hooks, set `disableAllHooks` [doc][cfg]. For permission prompts, pass `--yolo`.
- **Recording proxy:** set `COPILOT_PROVIDER_BASE_URL` to the proxy. The SDK's own tests replay all three wire APIs this way [sdk][proxy].
- **Traffic to GitHub:** `COPILOT_OFFLINE=true` limits traffic to the model provider and turns off telemetry [blog][byokcl]. Without it, the CLI auto-updates [doc][cfg], and server-side experiment flags reportedly toggle tool deferral [#4663][i4663].
- **Pinning:** `npm i -g @github/copilot@1.0.88` [readme][readme] with `COPILOT_AUTO_UPDATE=false` [doc][ref-env].

### 5. Loop behaviours

- **(a) Tool calling:** native function calling on each provider's API [sdk][e837], one model call per turn [sdk][loop].
- **(b) Parallel tool calls:** the system prompt asks for them [sdk][t1129]. They run concurrently, and the opt-out was removed in 0.0.418 [cl][cl3044], [cl][cl2304], [sdk][askuser]. A reported BYOK request capture shows no `parallel_tool_calls` field [#4950][i4950]. Whether reads and writes are treated differently is unverified.
- **(c) Tool results:** plain text. Shell output ends with `<exited with exit code N>` [sdk][s-shell]. Output over 20 KiB goes to a file, and the model gets the path plus a preview [doc][ctx].
- **(d) Tool errors:** returned as tool results, in the form "Failed to execute `x` tool … due to error: …". Calling an unknown tool returns the list of valid tool names [sdk][s-err], [sdk][s-unk]. No cap on repeated errors is documented.
- **(e) Stop conditions:**
  - The run ends when a response has no tool calls [sdk][loop].
  - Autopilot mode adds `task_complete` nudges and `--max-autopilot-continues`. The docs give its default as both unlimited and 5 [doc][ref-opts], [doc][auto].
  - `--max-ai-credits` is a soft cap.
  - No turn cap or timeout is documented.
- **(f) Max output tokens:** a per-model limit from a catalogue, which BYOK users can override [doc][byok], [sdk][t3203]. The default is unverified. Truncated output reportedly triggers an automatic "continue" call [#4733][i4733].
- **(g) Retries:** five retries, honouring `Retry-After` with bounded backoff, including after failures mid-stream [#2760][i2760], [#4680][i4680], [cl][cl2489]. Retries aren't configurable. BYOK Azure 429 responses reportedly use up all five retries in 0.15 s [#3679][i3679].
- **(h) Prompt caching:** system-prompt blocks carry explicit cache breakpoints, and each block is classed as static across sessions or not. Cache TTL and cache breaks are tracked [sdk][e8283], [sdk][e3200]. Cache hits across runs are unverified.
- **(i) Context management:** an LLM summary starts in the background at 80% of the context window and blocks at 95%. Both thresholds can be tuned in the SDK [doc][ctx], [sdk][t1968]. An HTTP 413 also triggers it [sdk][e523]. The fallback drops messages [sdk][e2858]. No masking of old tool outputs is documented.
- **(j) Reasoning:** `--reasoning-effort` ranges from low to max [doc][ref-opts]. The default is medium [doc][cfg], and some models have their own defaults [cl][cl1598]. Signed Anthropic thinking blocks and Chat Completions reasoning text are both replayed [cl][cl201], [#4950][i4950].
- **(k) Built-in tools:** persistent shell sessions (sync, async or detached), plus `view`, `edit`, `create`, `apply_patch`, `grep`, `glob`, `web_fetch`, `skill`, `task` and `ask_user` [doc][ref-tools], [cl][cl765].
- **(l) MCP:**
  - Transports: stdio, streamable HTTP and SSE.
  - Servers start per session, so once per `-p` run [sdk][mcpdoc].
  - Tool lists are cached, then rediscovered [doc][ref-cache]. Servers reconnect after transient failures [cl][cl13].
  - Server instructions are supported [cl][cl2590].
  - Annotations: the docs require permission for every MCP call [doc][ref-trust], but one report says `readOnlyHint` is honoured [issue][memlio].
- **(m) Skills:** loaded progressively. The model picks a skill by its description, and `SKILL.md` is injected only then [doc][skills].
- **(n) Structured output and completion:** the SDK accepts a `responseSchema` [sdk][t3440]. A run is done when the session goes idle; calling `task_complete` is optional [sdk][loop].
- **(o) Telemetry:** OTLP/HTTP or file export. Spans follow the GenAI semantic conventions (`invoke_agent` → `chat` → `execute_tool`) and carry token, cache and compaction data [doc][ref-otel]. W3C trace context is propagated [sdk][sdkotel].

### 6. Fit for lablet (opinion)

| Lablet choice                                     | Copilot                                           |
| ------------------------------------------------- | ------------------------------------------------- |
| Tool-error cap off by default                     | Supports                                          |
| Truncation at max_tokens is terminal, 32k default | Contradicts: per-model limit, then auto-continue  |
| OpenAI via Responses API                          | Partial: supported, but not the default           |
| Skills loaded progressively                       | Supports                                          |
| MCP servers live across runs, with liveness check | Contradicts: servers are per session              |
| Prompt cache shared across runs                   | Unverified: the static-block tracking hints at it |
| Mask old tool outputs first, summarise later      | Contradicts: summarises only                      |
| Loop unchanged by provider                        | Contradicts: behaviour is keyed to the model      |

Main obstacles:

- The engine is closed, so its behaviour has to be inferred from docs, SDK schemas and issues.
- Releases land almost daily and server-side experiment flags change behaviour too (1.0.81 changed sampling parameters), so reference runs need a pinned version and offline mode.
- Model-keyed behaviour muddies comparisons across providers, and replacing the system prompt requires the SDK.
- It makes model calls lablet doesn't: auto-continue, compaction summaries and utility calls. One utility call reportedly sent the internal `gpt-5.4-nano` model ID to a BYOK endpoint [#4680][i4680].

### 7. Verdict (opinion)

Copilot CLI is a good secondary reference: the SDK makes it scriptable and recordable, its telemetry is close to lablet's, and it matters to people tuning MCP servers. It shouldn't be the primary or the multi-provider reference, because its loop is closed, changes almost daily, and deliberately behaves differently for each model.

[ga]: https://github.blog/changelog/2026-02-25-github-copilot-cli-is-now-generally-available/
[lic]: https://github.com/github/copilot-cli/blob/main/LICENSE.md
[sdklic]: https://github.com/github/copilot-sdk/blob/main/LICENSE
[sdkga]: https://github.blog/changelog/2026-06-02-copilot-sdk-is-now-generally-available/
[loop]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/docs/features/agent-loop.md
[rel]: https://github.com/github/copilot-cli/releases
[pkg14]: https://github.com/github/copilot-sdk/blob/v1.0.14/nodejs/package.json#L8
[sdkrel]: https://github.com/github/copilot-sdk/releases
[tc]: https://techcrunch.com/2026/01/29/satya-nadella-insists-people-are-using-microsofts-copilot-ai-a-lot
[jb]: https://blog.jetbrains.com/research/2026/08/ai-coding-agent-adoption-2026/
[msr]: https://arxiv.org/abs/2607.01418
[ref-mcp]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#mcp-server-configuration
[ts]: https://docs.github.com/en/copilot/concepts/agents/copilot-cli/tool-search
[ref-skills]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#skills-reference
[byok]: https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models
[sdkbyok]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/docs/auth/byok.md#L209
[cl892]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L892
[byokcl]: https://github.blog/changelog/2026-04-07-copilot-cli-now-supports-byok-and-local-models/
[t3179]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/types.ts#L3179
[ref-tools]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#tool-availability-values
[cl783]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L783
[i4950]: https://github.com/github/copilot-cli/issues/4950
[i3118]: https://github.com/github/copilot-cli/issues/3118
[ref-opts]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#command-line-options
[t1094]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/types.ts#L1094
[t1190]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/types.ts#L1190
[upt]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/docs/hooks/user-prompt-transformed.md
[e8432]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/generated/session-events.ts#L8432
[mt]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/docs/setup/multi-tenancy.md#L31
[cfg]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference#user-settings-copilotsettingsjson
[proxy]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/test/harness/replayingCapiProxy.ts
[i4663]: https://github.com/github/copilot-cli/issues/4663
[readme]: https://github.com/github/copilot-cli/blob/main/README.md
[ref-env]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#environment-variables
[e837]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/generated/session-events.ts#L837
[t1129]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/types.ts#L1129
[cl3044]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L3044
[cl2304]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L2304
[askuser]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/rust/tests/e2e/ask_user.rs#L153-L163
[s-shell]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/test/snapshots/builtin_tools/should_capture_stderr_output.yaml
[ctx]: https://docs.github.com/en/copilot/concepts/agents/copilot-cli/context-management
[s-err]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/test/snapshots/tools/handles_tool_calling_errors.yaml
[s-unk]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/test/snapshots/tools/overrides_built_in_tool_with_custom_tool.yaml
[auto]: https://docs.github.com/en/copilot/concepts/agents/copilot-cli/autopilot
[t3203]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/types.ts#L3203
[i4733]: https://github.com/github/copilot-cli/issues/4733
[i2760]: https://github.com/github/copilot-cli/issues/2760
[i4680]: https://github.com/github/copilot-cli/issues/4680
[cl2489]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L2489
[i3679]: https://github.com/github/copilot-cli/issues/3679
[e8283]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/generated/session-events.ts#L8283
[e3200]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/generated/session-events.ts#L3200
[t1968]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/types.ts#L1968
[e523]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/generated/session-events.ts#L523
[e2858]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/generated/session-events.ts#L2858
[cl1598]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L1598
[cl201]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L201
[cl765]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L765
[mcpdoc]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/docs/features/mcp.md#L182
[ref-cache]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#tool-snapshot-caching
[cl13]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L13
[cl2590]: https://github.com/github/copilot-cli/blob/57dd2440141be0b7d6d628472890f861e3b3ca55/changelog.md#L2590
[ref-trust]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#mcp-server-trust-levels
[memlio]: https://github.com/yanggaome/memlio/issues/2
[skills]: https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills
[t3440]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/nodejs/src/types.ts#L3440
[ref-otel]: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference#opentelemetry-monitoring
[sdkotel]: https://github.com/github/copilot-sdk/blob/075f027363fc3b1e904d09370763731c3ecd2d88/docs/observability/opentelemetry.md

## Goose

Read from the source at tag `v1.52.0` of github.com/aaif-goose/goose. Source paths below use these prefixes: `g/` = `crates/goose/src/`, `pt/` = `crates/goose-provider-types/src/`, `pv/` = `crates/goose-providers/src/`, `cli/` = `crates/goose-cli/src/`.

### 1. Identity

- **What it is:** an open-source agent built around MCP, with a CLI, a desktop app and an ACP server. About 72% of the code is Rust, and the licence is Apache-2.0 ([repo](https://github.com/aaif-goose/goose)).
- **Maintainer:** Block made Goose a founding project of the Linux Foundation's Agentic AI Foundation on 9 Dec 2025 ([LF](https://www.linuxfoundation.org/press/linux-foundation-announces-the-formation-of-the-agentic-ai-foundation)). In April 2026 the repo moved to `aaif-goose/goose` and the docs to goose-docs.ai ([post](https://goose-docs.ai/blog/2026/04/07/goose-moves-to-aaif/)).
- **Latest version:** **v1.52.0, released 23 Sep 2026.** Releases come roughly weekly ([releases](https://github.com/aaif-goose/goose/releases)).

### 2. Relevance

- **Adoption:** 54.6k stars, 6.3k forks and about 640 contributors (GitHub API).
- **MCP host role:** every extension is an MCP server, over stdio, streamable HTTP or in-process (`g/agents/extension.rs`). Goose supports MCP elicitation and MCP Apps. It also ships a hidden `goose mcp-probe` command that its MCP conformance driver uses (`cli/cli.rs`, `cli/bin/mcp_conformance_driver.rs`).
- **Skills:** Goose finds `SKILL.md` skills in `.agents`, `.goose` and `.claude` directories (`g/skills/mod.rs`).
- People who write MCP servers and skills already target Goose.

### 3. Providers

- **Supported:** Anthropic, OpenAI and OpenAI-compatible hosts, Google, Bedrock, Vertex, Azure, Databricks, OpenRouter, Ollama, local models and custom providers declared in JSON ([docs](https://goose-docs.ai/docs/getting-started/providers)). Some providers pass work through to Claude Code or Codex, and those bring their own loop.
- **OpenAI API choice:** Goose uses the Responses API for o-series, gpt-5 and gpt-6 model names, or when the base path ends in `/responses`. Otherwise it uses Chat Completions (`pv/openai.rs`).
- **Same loop across providers?** The loop is shared, but its behaviour depends on the provider:
  - Explicit cache breakpoints exist only for Anthropic-style APIs (`pt/cache_semantics.rs`).
  - Tool-execution order differs (see 5b).
  - Defaults for max output tokens and thinking come from a model catalogue bundled in the binary (`pt/model.rs`).

### 4. Checkability

- **Headless:** `goose run -t|-i|--recipe --no-session --output-format json|stream-json` prints the messages, token, cache and cost totals, and a status (`cli/cli.rs`, `cli/session/mod.rs`).
- **System prompt:** `--system` and a recipe's `instructions` are appended to the default prompt.
  - `GOOSE_SYSTEM_PROMPT_FILE_PATH` replaces only the base template. Hints and other extras are still appended (`cli/session/builder.rs`, `g/agents/prompt_manager.rs`).
  - Each reply also adds a `<turn-context>` user message with the time, working directory and extension context. It can't be switched off. It's skipped only when the context limit is under 32k (`g/agents/moim.rs`).
- **Restricting tools:** use `--no-profile` with `--with-extension` or `--with-streamable-http-extension`, or a recipe's `extensions:` with `available_tools` (`cli/session/builder.rs`). Goose renames MCP tools to `<ext>__<tool>` (`g/agents/extension_manager/mod.rs`).
- **Switching things off:**
  - `--no-profile` drops sub-agents (the `summon` extension) and skills. Sub-recipes appear only when a recipe declares them.
  - Hooks still load from installed plugins (`g/hooks/mod.rs`). Hints still load from AGENTS.md or .goosehints (`g/hints/load_hints.rs`). To isolate a run, set `GOOSE_PATH_ROOT`, use a clean HOME and working directory, and set `CONTEXT_FILE_NAMES`.
  - The default mode, `GOOSE_MODE=auto`, asks for no approvals (`pt/goose_mode.rs`).
  - Set `GOOSE_DISABLE_SESSION_NAMING=true`, or Goose makes an extra model call to name the session.
- **Recording proxy:** `ANTHROPIC_HOST` and `OPENAI_HOST`/`OPENAI_BASE_PATH` override the base URL (`g/providers/*_def.rs`). Goose also keeps raw logs of the last 10 requests (`g/providers/utils.rs`).
- **Pinning:** install with `GOOSE_VERSION=v1.52.0 download_cli.sh`. Two loops exist side by side: the default `Agent::reply`, and a state machine switched on with `GOOSE_STATE_MACHINE=1` (`g/agents/state_machine/mod.rs`).

### 5. Loop behaviours (default loop unless noted)

- **a. Tool calling:** native. An opt-in "toolshim" has a second model (Ollama `mistral-nemo`) pull tool calls out of plain text (`g/providers/toolshim.rs`).
- **b. Parallel calls:** Goose never sets `parallel_tool_calls`, so the provider's default applies.
  - Calls that arrive in the same streamed chunk run concurrently (`g/agents/agent.rs`).
  - Anthropic streams one tool call per chunk, so its calls run one after another. OpenAI Chat Completions delivers them together (`pt/formats/anthropic.rs`, `pt/formats/openai.rs`).
  - Each call is stored in history as its own assistant/user message pair ([PR #11837](https://github.com/aaif-goose/goose/pull/11837)).
  - The opt-in state machine runs all of a turn's calls together (`ops_toolcalling.rs`).
- **c. Tool results:**
  - Only the MCP `content` field reaches the model, never `structuredContent`. Blocks meant only for the user are dropped (`pt/conversation/message.rs`).
  - On Anthropic, an MCP `isError` result isn't flagged `is_error` (`pt/formats/anthropic.rs`).
  - Text over 200k characters is written to a temp file (`g/agents/large_response_handler.rs`).
- **d. Errors:**
  - Unknown tools and unparseable arguments come back to the model as error results.
  - Goose repairs broken JSON and converts strings to numbers or booleans where the schema expects them (`pt/json.rs`, `g/agents/reply_parts.rs`).
  - There's no cap on tool errors. The opt-in `--max-tool-repetitions` refuses identical repeated calls (`g/tool_monitor.rs`).
- **e. Stop conditions:**
  - A turn ends when a reply has no tool calls.
  - `GOOSE_MAX_TURNS` defaults to 1000 model calls and ends with a fixed assistant message.
  - Refusals and authentication errors end the turn. Empty replies get 3 retries.
  - There's no token budget or wall-clock limit (`g/agents/agent.rs`, `g/agents/types.rs`).
- **f. Max output tokens:**
  - The limit is `GOOSE_MAX_TOKENS`, else the catalogue value, else 4096 (`pt/model.rs`).
  - A tool call cut off at the limit becomes an error result and the loop continues.
  - Cut-off text ends the turn normally, with a warning (`pt/formats/anthropic.rs`, `cli/session/output.rs`).
- **g. Retries:** 3 retries, starting at 1s and doubling to 30s, with ±20% jitter.
  - Goose retries 429, 5xx, network errors and most other 4xx errors (`pt/retry.rs`, `pv/http_status.rs`).
  - Requests time out after 600s.
- **h. Prompt caching:**
  - On Anthropic, cache breakpoints sit on the system prompt, the last tool and the last two user messages.
  - The cache lifetime is 5 minutes. Headless runs cut `GOOSE_CACHE_TTL=1h` back to 5 minutes.
  - Tools are sorted and the date is rounded to the hour, a code comment says for caching across sessions (`pt/formats/anthropic.rs`, `cli/session/builder.rs`, `g/agents/prompt_manager.rs`).
- **i. Compaction:**
  - At 80% of the context window, a model call summarises the whole history (`g/context_mgmt/mod.rs`).
  - The default loop checks this once per user message; the state machine checks on every pass.
  - A context-length error also triggers compaction.
  - Summarising old tool call pairs has been opt-in since [#11764](https://github.com/aaif-goose/goose/issues/11764). Nothing is masked.
- **j. Thinking:**
  - Set with `GOOSE_THINKING_EFFORT`. When it's unset, the source turns thinking off, but the [docs](https://goose-docs.ai/docs/guides/environment-variables) say adaptive thinking is the default for Claude 4.6+.
  - Signed thinking is replayed; thinking from a different model is dropped.
  - Responses API calls send `store:false` and don't ask for encrypted reasoning (`pt/formats/anthropic.rs`, `pt/formats/openai_responses.rs`).
- **k. Built-in tools:**
  - The developer extension offers `write`, `edit` (exact find and replace), `shell`, `tree` and `read_image`.
  - Each shell call is a fresh `bash -c` with a 300s timeout.
  - Output over 2000 lines or 50 KB is cut to the last 50 lines plus a path to the full file (`g/agents/platform_extensions/developer/`).
- **l. MCP:**
  - There's no SSE transport.
  - Servers live as long as the goose process, with no liveness check.
  - Server instructions go into the system prompt. Tool annotations matter only in smart-approve mode.
  - There's no tool router for large tool sets. An optional Code Mode reveals tools progressively (`platform_extensions/code_execution.rs`).
- **m. Skills and recipes:**
  - Skill names and descriptions sit in the system prompt, and a `load_skill` tool returns a skill's full text (`g/skills/client.rs`).
  - Recipes are YAML files holding a prompt, instructions, extensions, settings and a retry policy.
- **n. Structured output:** a recipe's `response` schema adds a `recipe__final_output` tool, and Goose keeps prompting until the model calls it (`g/agents/final_output_tool.rs`).
- **o. Telemetry:**
  - Goose exports traces, metrics and logs over OTLP/HTTP.
  - Its `invoke_agent`, `chat` and `execute_tool` spans carry GenAI usage and cache attributes. Capturing message content is opt-in (`g/otel/otlp.rs`, `g/agents/gen_ai_telemetry.rs`).

### 6. Fit for lablet (opinion)

| Lablet choice                           | Goose                                                                   |
| --------------------------------------- | ----------------------------------------------------------------------- |
| Tool-error cap off                      | Supports (d)                                                            |
| Truncation terminal, 32k default        | Contradicts (f). `GOOSE_MAX_TOKENS=32000` matches only the limit        |
| OpenAI via Responses                    | Supports for OpenAI's own models. Compatible hosts use Chat Completions |
| Skills loaded progressively             | Supports (m)                                                            |
| MCP servers across runs, liveness check | Contradicts (l)                                                         |
| Prompt cache shared across runs         | Supports (h)                                                            |
| Mask old outputs, then summarise        | Contradicts (i)                                                         |

**Main obstacles:**

- The per-reply turn-context message and the appended prompt extras can't be removed.
- MCP tool names get an extension prefix.
- Tool-execution order and history shape change with the provider and with which loop runs.
- Releases come weekly, so behaviour moves quickly.
- There's no machine-readable stop reason.
  - Hitting max turns and most provider failures show up as assistant text.
  - JSON `status` flags only stream errors, cancellation or a trailing error block (`cli/session/mod.rs`). This was read from the source, not run.

### 7. Verdict (opinion)

Goose is a strong secondary reference: it supports many providers, is built around MCP, runs headless, can be pinned, and emits GenAI traces and raw request logs. It isn't a good primary, because it can't run lablet's exact prompt and tool names, and its default loop changes tool-execution order with the provider.

## OpenHands Software Agent SDK

File paths are in [software-agent-sdk v1.49.5](https://github.com/OpenHands/software-agent-sdk/tree/v1.49.5), relative to `openhands-sdk/openhands/sdk/`. `tools/…` means `openhands-tools/openhands/tools/…`.

### 1. Identity

- OpenHands is an open-source coding agent from All Hands AI. The GitHub org is `OpenHands`, formerly `All-Hands-AI`. It's MIT-licensed and the source is public.
- **Pin: Software Agent SDK v1.49.5, released 2026-09-23** (`openhands-sdk` and `openhands-tools` on PyPI). The agent loop now lives only in the SDK. `OpenHands/OpenHands` v1.23.0, released the same day, is "Agent Canvas": a frontend that uses SDK 1.49.5 ([release](https://github.com/OpenHands/OpenHands/releases/tag/v1.23.0)).
- **CodeAct agent vs SDK:** the old V0 `CodeActAgent`, with its Docker runtime and many condensers, was removed from the main repo in 1.7.0 (2026-05-01). The SDK's `Agent` continues the CodeAct design. Papers: [SDK](https://arxiv.org/abs/2511.03690), [CodeAct](https://arxiv.org/abs/2402.01030).

### 2. Relevance

- The main repo has about 89k stars. The SDK powers the OpenHands CLI and OpenHands Cloud (SDK README).
- The [OpenHands Index](https://www.openhands.dev/blog/openhands-index) runs many models through the SDK as a fixed harness. That's the "same loop, different model" setup lablet wants.
- The SDK README badge claims 77.6 on SWE-bench Verified but doesn't name the model.
- It's a full MCP host and supports AgentSkills.
- Harbor's `openhands-sdk` agent declares `atif=True, skills=True, mcp_servers=True` ([source](https://github.com/harbor-framework/harbor/blob/main/src/harbor/agents/installed/openhands_sdk.py)), so a path to ATIF trajectories already exists.

### 3. Providers

- Every call goes through LiteLLM (locked at 1.93.0): Anthropic, OpenAI Chat and Responses, Bedrock, Vertex, and OpenAI-compatible servers.
- The step loop is the same for every provider. How requests are shaped is not: it depends on substrings of the model name (`llm/utils/model_features.py`, `llm/options/chat_options.py`):
  - the Responses API is used only for `gpt-5*`, `gpt-6*` and `codex-mini`;
  - prompt-cache markers are added only for Claude;
  - the thinking mode is chosen per model;
  - sampling parameters are dropped for reasoning models;
  - the output-token cap comes from LiteLLM's model metadata.
- So behaviour shifts when only the model changes, unless you pin `api_mode`, `capability_overrides` and the limits.

### 4. Checkability

- **Headless:** yes, through a short SDK script calling `Conversation(...).run()`. The CLI's `--headless` mode approves every action ([docs](https://docs.openhands.dev/openhands/usage/cli/headless)).
- **Custom system prompt:** `Agent(system_prompt=…)` is sent verbatim. A second, per-run block is added only when you pass an `AgentContext` or secrets (`agent/base.py`).
- **Restricting tools:** `tools=[]`, `mcp_config` and `filter_tools_regex` limit the set. `include_default_tools=["FinishTool"]` removes the `think` tool, which is otherwise always attached.
- **Off by default:** sub-agents, the security analyser, confirmation (`NeverConfirm`, `conversation/state.py`), the critic and the condenser.
- **On by default:**
  - Stuck detection; turn it off with `stuck_detection=False`.
  - Plugins found in HOME and in the workspace, which can add skills, MCP servers and hooks (`conversation/impl/local_conversation.py`). Run with an empty HOME.
- **Recording proxy:** set `LLM(base_url=…)`. For most models, setting a base URL lowers the default output cap to 16,384 tokens (`llm/llm.py`), so set `max_output_tokens` explicitly.
- **Pinning:**
  - Pin the exact PyPI versions and LiteLLM.
  - Set `LITELLM_LOCAL_MODEL_COST_MAP=True`. Otherwise LiteLLM downloads its model map at import ([source](https://github.com/BerriAI/litellm/blob/v1.93.0/litellm/litellm_core_utils/get_model_cost_map.py)), and OpenHands derives its defaults from that map.
  - There have been 97 SDK releases since October 2025.
- **Weight:** runs in-process on Python 3.12, with Docker optional. An agent that uses only MCP tools needs just `openhands-sdk`.

### 5. Loop behaviours

- **a. Tool calling:** native function calling. Text-based emulation is used only when `native_tool_calling=False` (`llm/mixins/non_native_fc.py`).
- **b. Parallel tool calls:** the model may request several per turn. They run one at a time unless `tool_concurrency_limit>1`. Any call after `finish` in the same turn is dropped (`agent/parallel_executor.py`, `agent/agent.py`).
- **c. Result formatting and truncation:**
  - Every tool schema gets an extra `summary` field. Tools not marked read-only also get a `security_risk` field (`tool/tool.py`, `mcp/tool.py`).
  - MCP results are prefixed `[Tool '<name>' executed.]`, and `structuredContent` is dropped (`mcp/definition.py`).
  - Output is cut from the middle: terminal output at 30,000 characters, any tool's text at 50,000 (`tools/terminal/constants.py`, `llm/message.py`).
- **d. Tool errors, malformed arguments, stuck detection:**
  - Bad JSON, unknown tools and exceptions become error results. Before that, OpenHands tries repairs, such as dropping null arguments and mapping `bash` to `terminal` (`agent/utils.py`).
  - After 3 identical failing calls it nudges the model; on the 4th it stops the run as stuck.
  - It also stops on 4 identical call-and-result pairs, 3 agent messages in a row, or 6 steps of alternation (`conversation/stuck_detector.py`).
  - There's no general cap on tool errors.
  - Empty replies and content-filter blocks get a nudge, and the loop continues.
- **e. Stop conditions:** the `finish` tool, a reply with no tool calls, `max_iteration_per_run=500`, an optional cost budget in USD, being stuck, or an error (`conversation/impl/local_conversation.py`).
- **f. Max output tokens:** defaults to the model's maximum (64k for Claude Sonnet 4.x). `finish_reason` is never read. From the code, a tool call cut off at the limit becomes a malformed-argument error, or runs if it still parses. Cut-off text ends the run as finished (`llm/llm.py`, `agent/response_dispatch.py`).
- **g. Retries:** 5 attempts, waiting 8, 16, 32 and 64 s, on connection errors, 429, 500, 503, timeouts and empty responses. Quota errors aren't retried, and the research found no `Retry-After` handling (`llm/utils/retry_mixin.py`).
- **h. Prompt caching:** on Claude, cache breakpoints go on the static system prompt and on the last user or tool message. On OpenAI, `prompt_cache_key` defaults to the conversation id, and `prompt_cache_retention` is 24h for GPT-5 and GPT-4.1 (`llm/llm.py`, `llm/options/`).
- **i. Context management:**
  - The only condensers are NoOp, Pipeline and LLMSummarizing; observation masking went with V0.
  - The summariser replaces roughly the older half of the history with an LLM-written summary.
  - It's triggered softly by event count, and hard by the token limit, an explicit request or a context-window error.
  - A bare `Agent` has no condenser. The default preset triggers at 80 events and keeps the first 4 (`context/condenser/`, `tools/preset/default.py`).
- **j. Reasoning:** `reasoning_effort="high"` by default. Claude models that use manual thinking get a thinking budget of up to 200k tokens. Thinking blocks, encrypted Responses reasoning (`store=False`) and some models' `reasoning_content` are sent back on later turns (`llm/options/chat_options.py`, `llm/message.py`).
- **k. Built-in tools:** `finish` and `think` are always present. The tools package adds:
  - a stateful tmux `terminal`: one command at a time, with a soft timeout;
  - `file_editor`, `task_tracker`, a browser, glob, grep and `apply_patch`;
  - a delegation tool.
- **l. MCP:**
  - Built on fastmcp 3.2, with stdio, SSE, streamable HTTP and OAuth.
  - One client per conversation, closed when the conversation ends. Listing tools times out at 30 s, and each call at 300 s.
  - Before each call it checks the connection and reconnects once if needed.
  - It handles `tools/list_changed`. With more than one server, tools are renamed `{server}_{tool}` ([fastmcp](https://github.com/PrefectHQ/fastmcp/blob/v3.2.0/src/fastmcp/client/transports/config.py)).
  - Server `instructions` are never used (`mcp/utils.py`, `mcp/tool.py`).
- **m. Skills:** skills are listed by name and description in `<available_skills>`, and an `invoke_skill` tool (added automatically) loads the full text. Keyword triggers insert a skill's text into the user turn (`context/agent_context.py`, `skills/skill.py`).
- **n. Completion protocol:** `finish(message)`, with an optional `response_schema` for a structured result (`tool/builtins/finish.py`).
- **o. Telemetry:**
  - OpenTelemetry spans via Laminar, when `OTEL_EXPORTER_OTLP_*` is set: a conversation span, `agent.step` spans, and LLM and tool spans.
  - Only a few attributes are `gen_ai.*`; this isn't the full GenAI semantic conventions (not checked in detail).
  - It also records token counts (including cache and reasoning tokens), cost and latency, and saves a JSON event log (`observability/laminar.py`, `llm/utils/telemetry.py`).

### 6. Fit for lablet (opinion)

| Lablet choice                                       | OpenHands                                                                      |
| --------------------------------------------------- | ------------------------------------------------------------------------------ |
| Tool-error cap off by default                       | Supports: no cap. Turn stuck detection off too                                 |
| Truncation at `max_tokens` is terminal, 32k default | Contradicts: truncation isn't detected, and the default is the model's maximum |
| OpenAI via the Responses API                        | Supports, for GPT-5-class models                                               |
| Skills loaded progressively                         | Supports                                                                       |
| MCP servers live across runs, with liveness check   | Contradicts: one client per conversation                                       |
| Prompt cache shared across runs                     | Supports on Claude. OpenAI needs a fixed `prompt_cache_key`                    |
| Mask old tool outputs first, summarise later        | Contradicts: summarisation only                                                |

**Main obstacles:**

- The extra schema fields, the renamed tools and the MCP result prefix change the very tool surface you'd be measuring. Removing them means patching OpenHands.
- It repairs arguments and nudges the model, which lablet doesn't do, and it ignores the finish reason.
- Request shape depends on the model name and on LiteLLM. It also releases very often, and picks up plugins from HOME and the workspace.
- MCP server startup time counts inside every run.

### 7. Verdict (opinion)

The OpenHands SDK is a credible **secondary** reference: one pinnable loop that many models, providers and Harbor's ATIF pipeline already run. It isn't a good primary reference, because it rewrites tool schemas and MCP results and keeps going after truncated output, so its token and tool-call counts won't match an unmodified MCP surface.

## Gemini CLI

Source paths are at tag v0.61.0 of https://github.com/google-gemini/gemini-cli (`core/` = `packages/core/src/`, `cli/` = `packages/cli/src/`).

### 1. Identity

- Google's terminal coding agent: TypeScript, Node ≥20, google-gemini org, Apache-2.0, open source ([repo](https://github.com/google-gemini/gemini-cli), [npm](https://registry.npmjs.org/@google/gemini-cli/0.61.0)).
- **Latest stable: v0.61.0, 2026-09-23.** Preview and nightly builds come out almost daily ([releases](https://github.com/google-gemini/gemini-cli/releases)).
- **Being wound down.** On 2026-05-19 Google named Antigravity CLI as its successor; it's written in Go and its source isn't published.
  - Since 2026-06-18, Gemini CLI no longer serves free, AI Pro or Ultra users.
  - It still works with paid API and enterprise keys, and Google says updates will continue ([blog](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/), [#27274](https://github.com/google-gemini/gemini-cli/discussions/27274), [#27304](https://github.com/google-gemini/gemini-cli/issues/27304)).

### 2. Relevance

- About 107k stars, 14.6k forks and ~290k npm downloads a week ([npm](https://api.npmjs.org/downloads/point/last-week/@google/gemini-cli)).
- It is a full MCP host: tools, prompts, resources, OAuth and `list_changed` (`core/tools/mcp-client.ts`). It also supports Agent Skills and extensions (`docs/cli/skills.md`).
- So it's directly relevant to tuning MCP servers and skills on Gemini models. New consumer use is moving to Antigravity.

### 3. Providers

- Gemini protocol only, through the `@google/genai` library. It can authenticate with an API key, Vertex AI or ADC (Application Default Credentials), Google login (Code Assist), or `GATEWAY`, which is any Gemini-compatible base URL (`core/core/contentGenerator.ts`).
- It has no OpenAI or Anthropic provider. Other models only work through a translating proxy such as [LiteLLM](https://docs.litellm.ai/docs/tutorials/litellm_gemini_cli).

### 4. Checkability

- **Headless:** runs with `-p` or when stdin isn't a TTY; `--output-format json|stream-json`; exit codes 0/1/42/53 (`docs/cli/headless.md`). In an untrusted folder a headless run fails unless you pass `--skip-trust` (`docs/cli/trusted-folders.md`).
- **System prompt:** `GEMINI_SYSTEM_MD=<file>` replaces the built-in prompt; it does not append (`docs/cli/system-prompt.md`). Two things still get added:
  - GEMINI.md memory is appended to the prompt (`core/prompts/promptProvider.ts`).
  - The first user message carries the date, OS, temp directory, directory tree and a `<loaded_context>` block (`core/utils/environmentContext.ts`, `core/config/config.ts`).
- **Tool restriction:** `tools.core` allowlist, `tools.exclude`, per-server `includeTools`, and `--allowed-mcp-server-names` (`docs/reference/configuration.md`).
- **Switch-offs:** `experimental.enableAgents`, `hooksConfig.enabled`, `skills.enabled`, `model.disableLoopDetection`, `general.topicUpdateNarration`, `general.enableAutoUpdate`, `privacy.usageStatisticsEnabled` and `context.includeDirectoryTree`. Setting `GEMINI_CLI_HOME` isolates the config (configuration.md).
- **Approvals:** headless denies any tool no rule matches, plus shell, writes and `activate_skill` (`core/policy/policy-engine.ts`, `core/policy/policies/write.toml`). A reference run needs `--approval-mode=yolo` or explicit allow rules.
- **Proxy:** `GOOGLE_GEMINI_BASE_URL` / `GOOGLE_VERTEX_BASE_URL`. The source only checks URL syntax, though the docs say HTTPS except localhost (`contentGenerator.ts`). `--record-responses` and `--fake-responses` record or replay model responses (configuration.md).
- **Pinning:** pin the npm version and also pass `-m`. The default model setting, `auto`, runs an LLM classifier on every request (`core/routing/modelRouterService.ts`).

### 5. Loop behaviours

- **a. Tool calling:** native `functionCall`/`functionResponse` over `generateContentStream` (`core/core/turn.ts`, `core/core/geminiChat.ts`).
- **b. Parallel calls:**
  - Every tool schema, MCP tools included, gets an injected `wait_for_previous` flag (`core/tools/tools.ts`, `core/tools/mcp-tool.test.ts`).
  - The default prompt tells the model to call in parallel (`core/prompts/snippets.ts`).
  - Contiguous calls run concurrently (`Promise.all`) unless the model sets that flag, or the tool is an edit or `update_topic`.
  - There's no read/write split by tool kind (`core/scheduler/scheduler.ts`).
- **c. Results and truncation:**
  - Results go back as `{output}` or `{error}` (`core/utils/generateContentResponseUtilities.ts`, `core/scheduler/tool-executor.ts`).
  - MCP text is wrapped in `<untrusted_context>` tags (`core/tools/mcp-tool.ts`).
  - Shell and MCP output over 40,000 characters keeps the first 20% and last 80%, and the full output is saved to a file (`core/utils/fileUtils.ts`).
- **d. Errors and loops:**
  - Unknown tools and bad arguments are returned to the model as errors (`scheduler.ts`).
  - The research found no cap on consecutive errors. Only fatal types such as NO_SPACE_LEFT exit (`cli/utils/errors.ts`).
  - `MALFORMED_FUNCTION_CALL` is retried as an invalid stream (`geminiChat.ts`).
  - Loop detection is on by default. It looks for 5 identical calls or repeated content, and runs an LLM check after 30 turns. The first hit sends the model a nudge; the second ends the turn (`core/services/loopDetectionService.ts`, `core/core/client.ts`).
- **e. Stop conditions:**
  - The run ends when a reply has no function calls.
  - `model.maxSessionTurns` is unlimited by default; if set and exceeded, it exits with 53.
  - Loop detection, hooks and invalid streams also stop the run (`cli/nonInteractiveCli.ts`).
  - The research found no run-level timeout or budget. A shell call is killed after 300 s without output, and MCP calls time out after 10 minutes (`core/tools/mcp-client.ts`).
- **f. Max output tokens:**
  - The main chat sets no `maxOutputTokens` (`core/config/defaultModelConfigs.ts`). An issue reports that it then behaves as 8,192 ([#23081](https://github.com/google-gemini/gemini-cli/issues/23081), unverified).
  - Headless ignores the finish reason, so text cut off at the limit ends the run as a success.
  - An empty reply with `MAX_TOKENS` is retried, then fails (`nonInteractiveCli.ts`, `geminiChat.ts`).
- **g. Retries:**
  - Up to 10 attempts, starting at 5 s and doubling to a 30 s cap, with ±30% jitter.
  - It retries 429, 499, 5xx and network errors, and never 400.
  - The wait comes from Google's `RetryInfo` or "retry in Xs" in the message, not the HTTP `Retry-After` header. Waits over 300 s are treated as terminal (`core/utils/retry.ts`, `core/utils/googleQuotaErrors.ts`).
  - Failures mid-stream get 4 attempts, starting at 1 s (`geminiChat.ts`).
- **h. Caching:**
  - It sets up no explicit cache; `caches.create` doesn't appear anywhere in the repo.
  - It relies on Gemini's implicit caching, which only works with API-key or Vertex auth (`docs/cli/token-caching.md`).
- **i. Context management:**
  - Before each turn it masks old tool outputs, by default. It always keeps the newest 50k tokens of tool output and the latest turn, and only masks once at least 30k tokens can go. Masked outputs become a `<tool_output_masked>` preview plus a file path (`core/context/toolOutputMaskingService.ts`, `client.ts`).
  - Summarisation starts at 50% of the context window, which defaults to 1,048,576 tokens. Old tool results are first cut to a 50k budget. It then makes a summary call and a second call to check it, and keeps the most recent 30% of history (`core/context/chatCompressionService.ts`, `core/core/tokenLimits.ts`).
- **j. Thinking:**
  - `includeThoughts: true`. Gemini 3 uses `thinkingLevel` HIGH; Gemini 2.5 uses an 8,192-token budget (`defaultModelConfigs.ts`, `core/config/models.ts`).
  - Thought text is dropped from history, but thought signatures are sent back. Any missing signature is filled with `skip_thought_signature_validator` (`geminiChat.ts`).
- **k. Built-in tools and shell:**
  - Built-ins: file read/write/replace, glob, grep, shell, web fetch and search, todos, ask_user, activate_skill, update_topic, MCP resources and subagents (`core/tools/`).
  - Each shell call is a fresh `bash -c` subshell that records background PIDs. It returns Output, Error, Exit Code, Signal and PIDs (`core/tools/shell.ts`).
- **l. MCP:**
  - Transports: stdio, SSE and streamable HTTP; no WebSocket.
  - Servers start in parallel and live as long as the process, with no health check (`core/tools/mcp-client-manager.ts`).
  - Server instructions go into the first user message (`core/context/memoryContextManager.ts`).
  - `readOnlyHint` is used only in approval-policy rule matching (`docs/reference/policy-engine.md`).
  - Tool names are `mcp_<server>_<tool>`, up to 63 characters (`mcp-tool.ts`).
- **m. Skills and extensions:**
  - Skills load progressively. The system prompt lists names and descriptions, and `activate_skill` returns the SKILL.md body plus a listing of its resources (`docs/cli/skills.md`, `core/tools/activate-skill.ts`).
  - Every installed extension is on unless `-e` narrows the set (`docs/cli/cli-reference.md`).
- **n. Final output:** the main agent has no structured final output. `complete_task` is only for subagents (`core/tools/complete-task.ts`). The JSON output has `response`, `stats` and `error`.
- **o. Telemetry:**
  - OpenTelemetry logs, metrics and traces, exported over OTLP, to a file, or to Google Cloud. It's off by default.
  - It follows the GenAI conventions only partly. It emits `gen_ai.client.*` metrics and `gen_ai.*` attributes, but span names don't use the `{operation} {model}` form, and its log events are named `gemini_cli.*` (`docs/cli/telemetry.md`, `core/telemetry/trace.ts`).

### 6. Fit for lablet (opinion)

| Lablet choice                                       | Gemini CLI                                                                                                                            |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| Tool-error cap off by default                       | Supports. Loop detection is its default safety net instead.                                                                           |
| Truncation at max_tokens is terminal, 32k default   | Partly. Cut-off text ends the run silently, empty replies are retried, and it has no 32k default (one can be set via `modelConfigs`). |
| OpenAI via the Responses API                        | Gemini CLI doesn't use it (details below).                                                                                            |
| Skills loaded progressively                         | Supports.                                                                                                                             |
| MCP servers live across runs, with a liveness check | Contradicts. Servers live for one process and are never checked.                                                                      |
| Prompt cache shared across runs                     | Consistent. Gemini's implicit caching works across runs.                                                                              |
| Masking old tool outputs first, summarising later   | Strongly supports. It's the default behaviour.                                                                                        |

**Gemini's OpenAI-compatible endpoint does matter.** Matching Gemini CLI needs either a Gemini provider in lablet or Chat Completions against Gemini's OpenAI-compatible endpoint. That endpoint doesn't document the Responses API ([docs](https://ai.google.dev/gemini-api/docs/openai)). It must also carry `extra_content.google.thought_signature` back on tool calls, or Gemini 3 rejects the request with a 400 ([example](https://github.com/vellum-ai/vellum-assistant/issues/42421)).

**Main obstacles:**

- It only speaks the Gemini wire format.
- It injects content: the environment message, the `wait_for_previous` flag, `<untrusted_context>` wrapping and rewritten schemas.
- It makes extra LLM calls: the model classifier, loop checks and the summary check.
- Headless denies tools by default.
- Truncation at max_tokens is silent.
- The code changes quickly, and the product is being wound down.

### 7. Verdict (opinion)

Suitable as a **secondary** reference: its masking-first compaction, progressive skills and MCP handling mirror lablet's choices, and it is open and highly configurable. Its Gemini-only protocol, its wind-down in favour of the closed-source Antigravity CLI, and its many injected behaviours rule it out as the primary one.

## Screen of other loops

### Gemini CLI is losing its users

Google moved consumer users from Gemini CLI to Antigravity CLI, a closed-source Go program, on 19 May 2026. Gemini CLI stopped serving free, AI Pro and Ultra accounts on 18 Jun 2026. It still works with paid API keys and enterprise licences, and the repo is active (v0.62.0 preview, 23 Sep 2026). But npm downloads are 1.4M a month, against 79M for Codex. It may no longer be worth a full profile; Qwen Code, an open fork of it, is covered below. [Google announcement](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/)

### The nine named candidates

**OpenAI Agents SDK: useful as secondary evidence only**

- **What:** OpenAI's agent framework, MIT licence. Python v0.22.3 (17 Sep 2026), JavaScript v0.18.0 (10 Sep 2026). In April 2026 it added sandboxed agents and a Codex-style harness (Python first).
- **MCP and providers:** it is an MCP host: stdio, Streamable HTTP, SSE and OpenAI-hosted MCP, with allow and block lists per server. It uses Responses by default, supports Chat Completions and any `base_url`, and reaches Anthropic only through beta LiteLLM or Any-LLM adapters. It is a library, not a CLI.
- **Loop:** native tool calls, and all calls in one turn run at once (capped by `max_function_tool_concurrency`). Going past `max_turns` raises `MaxTurnsExceeded` unless `error_handlers` catch it. `tool_use_behavior` sets the stop rules. Tool search and deferred tool loading work only with Responses.
- **Why secondary:** fully checkable, but end users never run a fixed default harness. It is a good cross-check for loop semantics.
- [releases](https://github.com/openai/openai-agents-python/releases), [loop](https://openai.github.io/openai-agents-python/running_agents/), [MCP](https://openai.github.io/openai-agents-python/mcp/), [models](https://openai.github.io/openai-agents-python/models/), [April 2026 update](https://devops.com/openai-upgrades-its-agents-sdk-with-sandboxing-and-a-new-model-harness/)

**OpenCode: worth a deep profile**

- **What:** `sst/opencode` now redirects to [anomalyco/opencode](https://github.com/anomalyco/opencode). MIT, TypeScript, 210k stars. Stable release v1.18.32 (21 Sep 2026); a separate 2.0 line is also being tagged (v2.0.16).
- **MCP and providers:** MCP host for local and remote servers. Models go through the Vercel AI SDK with a `baseURL` per provider: OpenAI uses Responses, others use Chat Completions, and Anthropic and local servers are supported.
- **Headless:** `opencode run --format json`, plus `opencode serve`, an HTTP API with an SDK.
- **Loop:** native tool calls with tool-call repair. The system prompt changes with the model family ([prompt files](https://github.com/anomalyco/opencode/tree/dev/packages/opencode/src/session/prompt)). Compaction is automatic, with optional pruning of old tool outputs. Tool output is cut at 2,000 lines or 50 KB, a `steps` setting caps turns, and OpenTelemetry output is experimental.
- **Checkability:** an agent's `prompt` replaces the system prompt. `permission` allows or denies each tool, including MCP tools by wildcard.
- [CLI](https://opencode.ai/docs/cli/), [agents](https://opencode.ai/docs/agents/), [config](https://opencode.ai/docs/config/)

**Cline: useful as secondary evidence only**

- **What:** Apache-2.0. Extension v4.1.20 (22 Sep 2026), CLI v3.0.65 (24 Sep 2026). A TypeScript SDK released in May 2026 now powers the CLI.
- **MCP and headless:** MCP host with a server marketplace. Many providers, including OpenAI-compatible servers, Ollama and LM Studio. Headless with `--json`, piped input, `--auto-approve` and `-t` (timeout); also supports ACP.
- **Loop:** native tool calls for frontier models since v3.35 (Oct 2025). Other models fall back to tool calls written as XML inside the prompt; whether that survived the SDK rewrite is unverified. Tools run in parallel. It compacts context and retries when a local model runs out of output tokens.
- **Why secondary:** 2026 usage is second-tier. It is not in the JetBrains or Pragmatic Engineer survey rankings and has 344k npm downloads a month. The XML fallback is worth reading for lablet's local-model path.
- [repo](https://github.com/cline/cline), [CLI](https://docs.cline.bot/usage/cli-overview), [native tools](https://cline.bot/blog/cline-v3-35), [SDK](https://cline.ghost.io/introducing-cline-sdk-the-upgraded-agent-runtime/)

**Kilo Code: useful as secondary evidence only. Roo Code: not suitable**

- **Kilo Code:** MIT, v7.7.9 (23 Sep 2026). Kilo CLI 1.0 (Feb 2026) and the v7 VS Code extension (Apr 2026) both run on a fork of OpenCode's server. Profiling OpenCode and comparing Kilo's prompts against it covers Kilo. [CLI 1.0](https://blog.kilo.ai/p/kilo-cli), [v7](https://blog.kilo.ai/p/new-kilo-for-vs-code-is-live)
- **Roo Code:** final release v3.54.0, and the repo was archived on 15 May 2026. roocode.com now redirects to Roomote, a cloud agent. [repo](https://github.com/RooCodeInc/Roo-Code)

**Cursor agent (CLI and SDK): useful as secondary evidence only**

- **What:** proprietary. The CLI changelog was last updated 26 Aug 2026. `@cursor/sdk`, in beta since 29 Apr 2026, exposes the same harness as the editor.
- **MCP and headless:** MCP host via `mcp.json`, with `--approve-mcps`. Headless with `agent -p --output-format json|stream-json`. Allow and deny rules can name MCP tools with `Mcp(server:tool)`.
- **Why secondary:** Cursor states that every request goes through its servers "for final prompt building", even with your own API key. So you cannot record model traffic or replace the system prompt. You can still record the MCP side (which tools it lists and calls, in what order, with what arguments) through an MCP proxy. With 12% of developers using it at work, that data is worth collecting.
- [headless](https://cursor.com/docs/cli/headless), [MCP](https://cursor.com/docs/cli/mcp), [permissions](https://cursor.com/docs/cli/reference/permissions), [API keys](https://cursor.com/docs/settings/api-keys)

**GitHub Copilot: the CLI is worth a deep profile; VS Code agent mode is secondary evidence only**

- **CLI:** proprietary licence, generally available since 25 Feb 2026, v1.0.88 (22 Sep 2026). An MIT-licensed SDK drives it. MCP host with GitHub's own server built in.
- **Headless:** `copilot -p … --output-format json`, and tools can be limited with `--available-tools` or `--excluded-tools`.
- **Loop:** native tool calls. Context compaction starts in the background at 80% full and blocks at 95%. Subagents run with `--fleet`.
- **Checkability:** you can bring your own key and point it at any base URL, choosing OpenAI, Azure or Anthropic and the wire API. `COPILOT_OFFLINE` stops it contacting GitHub's servers. The SDK can replace the system prompt, and it exports OTLP traces. The engine is closed, but it can be controlled and observed on every lablet provider path.
- **VS Code agent mode:** its code moved into microsoft/vscode (MIT) in May 2026. It accepts your own key and a custom endpoint without a GitHub sign-in, but has no headless mode.
- **Cloud coding agent:** not screened in depth.
- [GA](https://github.blog/changelog/2026-02-25-github-copilot-cli-is-now-generally-available/), [programmatic mode](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-programmatic-reference), [own keys](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models), [SDK](https://github.com/github/copilot-sdk), [VS Code own keys](https://code.visualstudio.com/blogs/2026/06/18/byok-vscode)

**Terminus 2: useful as secondary evidence only**

- **What:** the reference agent in Harbor (Apache-2.0, Harbor v0.23.0, 12 Sep 2026), and the harness behind the Terminal-Bench leaderboards.
- **MCP:** not an MCP host. A task's MCP servers are only listed in the instruction text. Models go through LiteLLM with `api_base`.
- **Loop:** one tmux session. The model replies in JSON or XML text containing keystrokes, with no native tool calls. It must confirm `task_complete` twice. Summarisation starts when fewer than 8,000 tokens are free, and the turn cap defaults to effectively unlimited.
- **Why secondary:** useful for comparing models, not for MCP or skills. Harbor itself is useful: its adapters pass a task's MCP servers to Codex, Goose, OpenCode, Copilot CLI, Cursor CLI, OpenClaw, Hermes, Cline and pi.
- [source](https://github.com/harbor-framework/harbor/blob/main/src/harbor/agents/terminus_2/terminus_2.py), [adapters](https://github.com/harbor-framework/harbor/tree/main/src/harbor/agents/installed)

**Amp: not suitable**

- **What:** proprietary, with continuous builds (latest 24 Sep 2026).
- **MCP and headless:** MCP host via `--mcp-config`. Headless with `amp -x --stream-json`.
- **Loop:** its context handling has flipped. It dropped compaction for "Handoff" in Oct 2025, then brought compaction back in its May 2026 rebuild.
- **Why not suitable:** when you choose a mode's model, Amp keeps its own prompt and tools. Custom endpoint URLs are only in early access for paid tiers (13 Sep 2026). Usage is small, at 101k npm downloads a month.
- [execute mode](https://ampcode.com/docs/cli/execute-mode), [mode dial](https://ampcode.com/news/build-your-own-dial), [own keys](https://ampcode.com/news/free-agent), [rebuild](https://ampcode.com/news/neo)

**Aider: not suitable**

- **What:** Apache-2.0. Last release v0.86.2 (12 Feb 2026) and last commit 22 May 2026, so it is effectively dormant.
- **Why not suitable:** it is not an MCP host and has no tool-calling loop. The model writes edits as text blocks that Aider applies. Headless with `--message` and `--yes`.
- [PyPI](https://pypi.org/project/aider-chat/), [scripting](https://aider.chat/docs/scripting.html)

### Candidates missing from the list

**OpenClaw: worth a deep profile, for its reach**

- **What:** MIT, v2026.9.6 (23 Sep 2026), 390k stars, 13.6M npm downloads a month. A self-hosted personal assistant used mostly outside coding.
- **MCP:** host over stdio, Streamable HTTP and SSE, with a tool filter per server. It also hosts skills.
- **Headless:** `openclaw agent --local --message … --json`.
- **Loop:** its own runtime. It used to embed pi ([Ronacher](https://lucumr.pocoo.org/2026/1/31/pi/)); only pi's terminal UI remains a dependency.
- **Caveats:**
  - Replacing the system prompt needs a plugin hook.
  - When it talks to OpenAI's official Responses API it may hand the run to the Codex harness, so profile it on Anthropic or a custom endpoint.
  - It releases several times a week.
- [MCP](https://docs.openclaw.ai/tools/mcp), [agent CLI](https://docs.openclaw.ai/cli/agent), [runtime](https://docs.openclaw.ai/agent-runtime-architecture)

**Hermes Agent (Nous Research): worth a deep profile, as the most checkable personal agent**

- **What:** MIT, v2026.9.24 (24 Sep 2026), 249k stars.
- **MCP:** host over stdio, HTTP and SSE, with include and exclude lists per server. It supports MCP sampling, resources and prompts, and parallel tool calls can be enabled per server.
- **Headless:** `hermes -z`, or `chat --oneshot -q --format stream-json`. `--usage-file` records tokens, API calls, exit reason and side calls to the model.
- **Loop:** native tool calls, with separate code paths for Anthropic, Chat Completions, Responses and Bedrock. The turn cap defaults to 500.
- **Checkability:** `--ignore-user-config --ignore-rules` isolates a run, and `--toolsets` limits tools. It has an OTLP exporter, but its spans don't follow the GenAI conventions.
- [CLI](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/reference/cli-commands.md), [MCP](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp)

**Pi (formerly badlogic/pi-mono): useful as secondary evidence only**

- MIT, v0.87.1 (22 Sep 2026), 109k stars, 9.4M npm downloads a month. It has no MCP support by design.
- It is easy to control: `--print`, JSON and RPC modes, `--system-prompt`, `--tools`, and a custom base URL for Anthropic, Responses or Chat Completions. Tool calls run in parallel.
- A good minimal baseline for testing skills without MCP. [CLI docs](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/cli.md)

**Others**

- **Antigravity CLI:** closed source, signs in through a browser, used by 6% of developers at work. Not suitable.
- **[Qwen Code](https://github.com/QwenLM/qwen-code):** Apache-2.0 fork of Gemini CLI that runs on OpenAI-compatible and Anthropic APIs. Secondary evidence.
- **[Factory Droid](https://docs.factory.ai/model-independence/byok):** proprietary, but accepts your own key with a custom base URL. Secondary evidence.
- **Kiro CLI:** closed source. Secondary evidence at most.
- Crush, Continue, Mistral Vibe, Kimi Code, Zed and Junie were not screened in depth; they have small usage or run only inside an editor.

### Ranked shortlist for deep profiles

OpenCode and GitHub Copilot CLI were profiled after this screen, above. OpenClaw and Hermes Agent weren't.

1. **OpenCode.** Open source and widely used: 7% of developers use it at work, and it made the most MCP tool calls in the only MCP-side usage data found. It takes a custom base URL on every provider path, and profiling it also covers Kilo.
2. **GitHub Copilot CLI, driven through its SDK.** 21% of developers use Copilot at work. The engine is closed, but own keys, base URL, offline mode, prompt replacement, tool filters and OTLP make it observable.
3. **OpenClaw.** The biggest open-source agent by stars and npm downloads, and the main MCP and skills host outside coding. It is harder to pin down than the others.
4. **Hermes Agent.** Take it instead of OpenClaw if checkability matters more than reach.

### Which harnesses end users of MCP servers mostly use in 2026

The MCP users anyone can measure are mostly developers working in coding harnesses. JetBrains' Developer Ecosystem Survey 2026 (more than 15,000 professional developers, May to July 2026) found these shares using each tool at work: Claude Code about 39%, GitHub Copilot 21% (down from 29%), Codex 16%, Cursor 12%, JetBrains AI about 9%, OpenCode 7% and Antigravity 6% ([JetBrains](https://blog.jetbrains.com/research/2026/08/ai-coding-agent-adoption-2026/)). The Pragmatic Engineer's survey (906 respondents, Jan to Feb 2026) ranks Claude Code first, then Copilot, Cursor, Codex, Gemini CLI and OpenCode ([Pragmatic Engineer](https://newsletter.pragmaticengineer.com/p/ai-tooling-2026)). OpenAI reported more than 5M weekly Codex users on 2 Jun 2026 ([Constellation Research](https://www.constellationr.com/insights/news/openai-touts-broadening-codex-usage-5-million-weekly-active-users)). Microsoft's 4.7M paid Copilot subscribers in January 2026 comes only from a secondary source ([AI Business](https://aibusiness.vc/b2b/github-copilot-2b-arr)). The only data from the MCP side is from one Playwright MCP server (1.42M tool calls, Nov 2025 to Jun 2026). By distinct users it ranks Claude Code 980, Cursor 601, Codex 316, VS Code 126 and claude.ai 104, while OpenCode made the most calls (558k from 358 users, which would rank it third by users). About half of the client IDs in that data are untagged. One server makes this indicative only ([qaby.ai](https://qaby.ai/blog/mcp-client-comparison-claude-code-vs-cursor)). npm downloads for 23 Aug to 21 Sep 2026 were: Codex 79.2M, Claude Code 59.0M, OpenClaw 13.6M, Copilot SDK 10.4M, Copilot CLI 10.0M, pi 9.4M, OpenCode 8.9M, Gemini CLI 1.4M, Cline 0.34M, Kilo CLI 0.14M and Amp 0.10M ([npm API](https://api.npmjs.org/downloads/point/last-month/@openai/codex)). Automated installs inflate these counts, and tools with their own installers are undercounted. Outside coding, MCP's steward lists built-in client support in ChatGPT, Claude, Cursor, Gemini, Microsoft Copilot and VS Code ([MCP blog, Dec 2025](https://blog.modelcontextprotocol.io/posts/2025-12-09-mcp-joins-agentic-ai-foundation/)). How much MCP traffic goes through chat apps and personal agents is unverified. OpenClaw's reported 3.2M active users appears only on third-party statistics sites and is unverified.
