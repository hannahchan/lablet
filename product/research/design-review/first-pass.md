# Design review, first pass

Delivered 2026-09-24 by a workflow of four reviewers, one adversarial verifier per reviewer, and a synthesiser, judging against the spec and `decisions.md` as they stood after phase 3a. Findings a verifier found already decided were dropped, which is why the second pass judged against the goals instead. Kept as delivered; the report on the second pass supersedes its conclusions.

---

## Summary

The design is sound. The core loop is modelled carefully: typed run states, one stop reason per state, and totals computed from turns. The telemetry is contract-first, the output formats are standard, and the major trade-offs are recorded in `decisions.md`. After adversarial verification, none of the findings rated high. Eight medium findings remain, all cheap to fix now. Most are about measurement: the data doesn't yet say exactly what the model saw or what caching cost. Six of the eight should be settled before phase 4 freezes the telemetry registry, the documents, and the tool-executor port.

## Decide before phase 4 and phase 7

These get expensive once adapters and fixtures are built against them.

- Before phase 4:
  - Add digests of what the model actually received (the tool specs and the final system prompt), and take output sinks out of the config digest.
  - Add `max_output_bytes` to `ToolCall` so executors stop buffering past the cap.
  - Decide whether unreported token counts are `Option` or zero before the telemetry mapping always emits them.
  - Clamp provider and tool deadlines to the time left in `run.timeout`.
  - Say plainly that bash isn't a sandbox, and clear its environment.
  - Add `schema_version: 1` to the outcome when it moves to `lablet-documents`.
- Before phase 7: make prompt caching a mode that can cache the conversation, before the Anthropic adapter's wiremock fixtures and P6 lock in prefix-only caching.
- Before phase 9: rewrite the OpenAI adapter paragraph in the spec.

## Findings

All surviving findings are medium after verification. The finders rated several of them high, and the verifiers lowered them. They're ranked here by how much they affect what lablet measures.

### 1. The config digest doesn't identify experiment arms

- Severity: medium (merges two findings, both marked "partly" by the verifier)
- Area: fit-measurement, contracts
- Decide before: phase 4
- Evidence: spec.md:693 hashes the resolved config tree before `${VAR}` substitution, and C5 (acceptance.md:79) requires identical digests. The config includes `run.transcript_path`, `telemetry.file.path`, `telemetry.otlp.*` and `telemetry.resource` (spec.md:638, :682-690). The wide event's setup group carries only tool names and a count (spec.md:110).
- Problem: this fails in two directions.
  - Arms merge. Two MCP server versions launched by the same command (`npx -y pkg@${V}`) get the same digest. So do an edited `system_file` or skill, and a changed `${VAR}` URL. Nothing records which tool descriptions and schemas the model saw, and that's the variable the brief's example use case compares.
  - Groups split. A composer that sets a transcript or telemetry path per run gives every run its own digest, so the digest stops working as a grouping key.
- Why "partly": composers can label arms through `telemetry.resource`, which exists for that purpose, so arms can still be told apart by hand. The spec deliberately lets a change of default move the digest, and `gen_ai.agent.version` sits beside it. The finding's point about defaults was refuted.
- Recommendation:
  - Add `lablet.tools.digest`, computed over the tool specs as sent (names, descriptions, schemas, order).
  - Add `lablet.prompt.system_digest`, computed over the final system prompt including skills.
  - Put both on the wide event and the root span. They're additive.
  - Leave output and telemetry sinks out of `config.digest`, and keep the rest as it is.

### 2. Only the fixed prefix is cached, so cost and latency don't match production loops

- Severity: medium (the finder said high; the verifier said partly)
- Area: fit-measurement
- Decide before: phase 7
- Evidence: spec.md:540 puts `cache_control` on the system prompt and tool specs only (also spec.md:653 and P6 at acceptance.md:96). Production harnesses such as Claude Code cache the growing conversation.
- Problem: every turn re-sends the whole conversation, but only the fixed prefix is ever read from cache. On a 30-turn run, cache reads, cache writes, `cost_usd` and provider latency differ by multiples from what a production loop pays.
- Why "partly": `input_tokens`, turns and tool calls, the brief's core metrics, are unaffected. Changing the tool surface breaks the cached prefix in every mode, so that part isn't specific to this design. P6 isn't built yet, so the change is still cheap.
- Recommendation: make `model.cache` a mode, `none | prefix | conversation`, with `conversation` as the default. Report the mode on the wide event, using the `lablet.request.cache_control` attribute that catalogue.md:54 already proposes.

### 3. Run timeout can overrun by a whole tool phase

- Severity: medium (the finder said high; the verifier said partly)
- Area: loop-domain
- Decide before: phase 4
- Evidence: service.rs:336 and :510 pass fixed `provider_timeout` and `tool_timeout` values as deadlines. spec.md:63 promises an overrun of "at most one provider or tool call".
- Problem: tool groups run one after another. Ten exclusive calls with a 60-second tool timeout can run 10 minutes past `run.timeout`, so the spec's promise is already false.
- Why "partly": the Ctrl-C half, including a status for calls that never ran, is a known open question (spec §10, decisions 2026-09-23). Adding an enum value later is additive and doesn't raise the transcript version.
- Recommendation: clamp each deadline to the smaller of the per-call timeout and the time left in the run. Consider adding `ToolCallStatus::NotRun` when `Pending::answer` gets a stop check between groups.

### 4. bash is described as sandboxed but inherits lablet's environment

- Severity: medium (the finder said high; the verifier confirmed it at medium)
- Area: adapters-security
- Decide before: phase 4
- Evidence: quality-bar.md promises built-in tools "sandboxed to a root". Spec §6 gives bash only a working directory and a timeout (spec.md:552), and bash is on by default (spec.md:664). The API key and `${VAR}` secrets are in the process environment. The spec doesn't say whether stdio MCP servers inherit the parent environment (spec.md:670).
- Problem: a root check can't confine a shell. The model can run `env` and put the API key into the transcript, which is always written. The file tools don't handle symlinks that lead outside the root.
- Recommendation:
  - Change the wording: bash sets its working directory, and isolation comes from the environment lablet runs in.
  - Start bash and stdio MCP servers with a cleared environment plus an explicit allowlist.
  - Check file paths after canonicalising them.
- Why medium rather than high: lablet targets containers, and the fix stays cheap.

### 5. Tool output is fully buffered before the cap applies

- Severity: medium (confirmed)
- Area: adapters-security
- Decide before: phase 4
- Evidence: spec.md:554 says "Neither executor shortens output". `ToolCall` has no cap field (tool.rs:17-29), and the cut happens afterwards in `Answer::measured`.
- Problem: a command like `yes`, or a huge MCP result, is held whole in memory. Lablet can then be killed for running out of memory and leave no outcome or wide event.
- Recommendation: put `max_output_bytes` on `ToolCall`. Executors stop keeping bytes past the cap but keep counting, and report `original_bytes` so `truncated_from_bytes` stays exact. The loop keeps its single cut.

### 6. Usage can't tell "zero" from "not reported"

- Severity: medium (partly)
- Area: adapters-security
- Decide before: phase 4
- Evidence: usage.rs:19-25. Reasoning, cache read and cache write are plain `u64` values that default to zero. Ollama and vLLM often omit them (spec.md:544).
- Problem: when rows are aggregated across providers, "none" and "unknown" get mixed silently.
- Why "partly": this was decided (spec.md:348), and `gen_ai.provider.name` on each row lets a consumer tell the two cases apart. Server-side tool counts are already deferred as additive (decisions.md:351).
- Recommendation: make those three counts `Option`, and emit an attribute only when the provider reports it. Settle this before phase 4 fixes the always-emit mapping.

### 7. Outcome JSON has no schema version

- Severity: medium (partly)
- Area: contracts
- Decide before: phase 4
- Evidence: spec.md:727 defers the version. build-plan.md:64 moves the outcome into `lablet-documents` without adding one. The transcript precedent is at decisions.md:531.
- Problem: composers parse this document from stdout on every run. Without a version, they have to infer its form from which keys are present.
- Why "partly": the deferral is deliberate. The `deny_unknown_fields` concern is moot because the outcome's read path is being removed (decisions.md:579-589).
- Recommendation: add `schema_version: 1` in phase 4 and use the transcript's rule: adding a field isn't breaking; renaming or dropping one is.

### 8. The OpenAI adapter spec contradicts the unparsed-arguments decision

- Severity: medium (confirmed)
- Area: adapters-security
- Decide before: phase 9
- Evidence: spec.md:544 says arguments that aren't valid JSON are `Malformed` and unknown fields go to `Opaque`. spec.md:59 and decisions.md:336-340 say unparsed arguments become `ToolInput::Unparsed` and end as `malformed_input`.
- Problem: built as written, bad arguments from an OpenAI-compatible model would be retried and counted as provider retries. The same failure from Anthropic counts against the tool. `Opaque` is a content block, so it has no natural place in chat completions.
- Recommendation: rewrite the paragraph now. By default, drop unknown message fields and log a debug diagnostic.

## What the design gets right

- Tool calls run in shared and exclusive groups, following Claude Code's rule, so a turn's wall time matches a production harness.
- The Run, Final and Pending typestate makes it impossible to record the wrong number or order of tool outcomes.
- The finish reason is read before any tool runs, so a truncated or refused response never executes a tool. Every state has exactly one stop reason.
- Malformed tool arguments go back to the model instead of being retried, which keeps provider flakiness separate from tool-surface problems.
- The loop cuts all tool output in one place and records `truncated_from_bytes`, so truncation is measured the same way for every tool.
- Usage follows GenAI conventions, emits raw counts only, and reports pricing rates beside the cost so cost can be recomputed.
- Telemetry is contract-first: a Weaver registry, generated constants, a justification policy for `lablet.*` names, and live-check in CI.
- Join keys are on every record, and composer attributes sit on the Resource.
- File output is plain OTLP/JSON. The transcript has an explicit `schema_version` and a rule for when it rises.
- Secrets arrive only through `api_key_env` and `${VAR}` and stay out of the digest. Content capture is off by default.

## Dropped or refuted

- The 4096 output cap ends runs for a harness reason: terminal truncation is deliberate, and a default is a one-line change.
- No context management: compaction is planned for later, and the only fix asked for is documentation.
- The wide event can't split tool, backoff and overhead time: knowingly deferred (spec §10), and the attributes are additive.
- The transcript is both the record and the provider context: this was chosen on purpose, and it guards only against features nobody has planned.
- `task_complete` drops sibling calls, and an empty response completes: both are stated policy and cheap to change.
- No retry-after on rate limits: `retries_exhausted` is its own stop reason, the retry settings are configurable, and a field can be added later.
- `Final` is terminal and leaves no seam for a user who speaks again: out of scope, and the seam is already reserved.
- MCP servers outlive runs: this lifetime is deliberate, and "no state across runs" refers to aggregation state.
- Per-tool wide-event keys vary per run: this was decided, and they're bounded by the tool set.
- Upstream GenAI renames force major versions: under 0.x a break is a minor bump. The small leftover is setting `schema_url` on the instrumentation scope.
- Too much rigour spent on internal types: this is a process concern, not a contract choice, and its costs were accepted on purpose.
