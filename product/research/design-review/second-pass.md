# Design review, second pass

Delivered 2026-09-24 by a workflow of eight area reviewers, three independent verifiers per finding (truth, decision, impact), a completeness critic with gap reviewers, and a synthesiser. Judged against four goals: representative of production agent loops, valid measurement, good implementation, lightweight. Kept as delivered. What the human decided on its eleven owner decisions is recorded in `decisions.md`, 2026-09-28, and the parity research in [../parity/](../parity/) later revised several of them.

---

## Summary

The core of lablet is sound. The typestate run, the contract-first telemetry registry, a single source of numbers, tool calls grouped in order and signed-thinking replay all hold up against production agent loops. The problems are mostly in defaults, spec text and records for phases that aren't built yet. Almost all of them are cheap to fix now and more expensive after phase 4 freezes the telemetry contract and phase 7 builds the adapters.

The things that matter most:

- Several harness defaults decide how runs end in ways production loops don't. These are the three-error tool cap, the 100 KB output cut that keeps only the start, the 4096 output-token default and caching only the prompt prefix. Results then describe lablet rather than the agent (G1).
- The records don't capture the variable under test. Tool descriptions, schemas, server versions and prompt contents never reach the digest or the wide event. The digest also changes with output paths and labels, so it can't group runs (G2).
- The OpenAI-compatible path is the least faithful. It drops reasoning between calls, its spec paragraph contradicts a recorded decision, Ollama silently truncates context, and its errors aren't classified.
- Usage can't tell zero from "the provider didn't report it". Billed failed attempts vanish from usage and cost, and the wide event reports settings the provider ignored (G2).
- A tool call that times out isn't required to stop, so it can keep writing after later calls start (G3).

## Decisions only the owner can make

Each of these findings conflicts with a recorded decision or with the brief's scope.

1. Per-call tool-error counting (decisions 2026-09-23). Trade-off: a simple guard against looping, against runs ended in ways no reference loop ends them. Recommendation: count per turn, count only harness failures, and turn the cap off by default.
2. Terminal output_truncated (decisions.md:43-45). Keep it terminal, because it's a legitimate finding about the model. Raise the 4096 default to 32k or more and state the difference from production.
3. task_complete ignores sibling calls (spec.md:50). Recommendation: answer a task_complete that arrives with other calls as malformed_input, so the model resends it on its own.
4. Chat Completions for OpenAI (brief; decisions.md:351 defers replay). Recommendation: use the Responses API with encrypted reasoning for OpenAI models. Keep Chat Completions, with reasoning_content replay, for other servers. The alternative is to declare the difference and refuse reasoning models.
5. Inlined skills (brief: "optional inlined skills"). Recommendation: load skills progressively by default, and keep inlining as a named option.
6. "A failed attempt reports no usage" (spec.md:99). Recommendation: give ProviderError an `Option<Usage>`.
7. Composer keys live only on the Resource (decisions.md:159), and the digest hashes the whole config (spec.md:693). Recommendation: declare a small set of per-run labels and leave sinks and labels out of the digest.
8. Settings a provider ignores are accepted silently (spec §6). Recommendation: reject them at build. This follows G4's "refuse rather than model".
9. MCP servers live as long as the Lablet (spec.md:558). Keep that. Add a liveness check or restart at run start, and record that servers are reused.
10. The prompt cache is shared across runs. That trades G1 against G2. Keep shared as the default and add a per-run scope.
11. Every chat record carries the full history, as the semantic conventions require ("semantic conventions first"). Keep that, and deliver the wide event separately from content.

## Decide by phase

Before phase 4:

- The tool-error counting rule (H1).
- The output cut rule and default (M1), and a cap in the port (M2).
- Optional usage counts (H5).
- Recording the tool surface and request parts (H7).
- The digest's scope, per-run labels and a run request type (H8).
- The abort-on-timeout contract (H9).
- Deadline clamping (M17).
- Explicit-mode reminders (M4), sibling calls (M5), usage and retry-after on ProviderError (M13, M14).
- The auth error kind (M23), the outcome version (M24), transcript identity (M22).
- The truncation check in the loop (H6).
- The requested and applied registry (M11), error strings (M20), wide-event delivery (M19).
- The per-turn offered set (M8).

Before phase 7, with phases 5 and 6 included here:

- Config keys: root overlap (M21), completion_schema (M6), the parallel flag (M12), cache_scope (M16), provider_timeout semantics (M3).
- Rejecting ignored settings (M11).
- SIGTERM and exit codes (M18, M23).
- Cache breakpoints (H2).
- The builtin tool profile and bash semantics (M7).
- read_file paging (M1).
- Process groups (H9).
- Streaming or an idle timeout (M3).
- Invalid tool names (M13).
- The Thinking serde slot for OpenAI replay (H3).

Before phase 9, with phase 8 included here:

- MCP instructions (M9), server identity capture (H7), server restart (M10).
- OpenAI reasoning (H3), the stale provider-openai paragraph (H4), the error table (M15), Ollama num_ctx documentation (H6).

Later:

- The skills default (M8, phase 10).
- The low items below.

## Findings

### High

#### H1. The tool-error cap ends ordinary runs

High. G1, G2. Areas: fit, loop, tools (three findings merged). Decide the counting rule before phase 4 and bash's exit semantics before phase 7.

- Evidence: spec.md:635 sets the default to 3. transcript.rs:160-169 counts per call across turns. is_error includes a tool's own error result. The spec doesn't say whether a non-zero bash exit is an error (spec.md:552). No reference loop has this stop.
- Problem: three parallel read_file misses, or three failing test runs, end the run as a Failed class. Inside a group, the count depends on call order. Success rates then move with how much the model parallelises.
- Recommendation: count per turn, or count only unknown, malformed_input, failed and timeout. State that a non-zero bash exit is ordinary output. Turn the cap off by default, or record it as a deliberate difference.
- Votes (all three findings): truth yes, decision yes, impact yes.

#### H2. Only the system prompt and tools are cached

High. G1, G2. Area: first pass. Decide before phase 7.

- Evidence: spec.md:540 and P6 put cache_control on the prefix only.
- Problem: Claude Code, the Agent SDK and Codex put rolling breakpoints on the conversation. cost_usd, cache reads and writes, and latency therefore differ from production by multiples.
- Recommendation: add a rolling breakpoint on the last message, as the references do.
- Votes: first-pass verifier partly; rerate real, high (up from medium).

#### H3. OpenAI-compatible calls drop reasoning between tool calls

High. G1, G2. Areas: fit, provider (merged). Decide the phase 9 scope now, and the Thinking slot before phase 7.

- Evidence: spec.md:544 says "Thinking blocks are dropped on the way out". Build-plan phase 9 lists only Chat Completions. Codex and the Agents SDK replay encrypted reasoning items. DeepSeek and Kimi require reasoning_content to be sent back.
- Problem: OpenAI reasoning models re-reason every turn, so their numbers don't transfer. DeepSeek-style servers reject turn 2, which ends the run as provider_error. Cross-provider A/Bs change two variables at once.
- Recommendation: see decision 4. Record the API surface and whether reasoning was replayed on the wide event, and add a replay case to P3.
- Votes: truth yes, decision yes, impact yes (both findings).

#### H4. The provider-openai paragraph contradicts the unparsed-arguments decision

High. G1, G2. Area: first pass. Fix the spec now; it must be settled before phase 9.

- Evidence: spec.md:544 says bad arguments are "Malformed", which is retried. spec.md:59 and decisions 2026-09-20 say MalformedInput. Unknown fields go "to Opaque", but Opaque is a content block.
- Problem: the same model error counts as provider_retries on OpenAI and as a tool-surface signal on Anthropic.
- Recommendation: rewrite the paragraph to match the decision, and drop the Opaque mapping.
- Votes: first-pass verifier confirmed; rerate real, high (up from medium).

#### H5. Usage can't tell zero from "not reported"

High. G2. Area: first pass. Decide before phase 4.

- Evidence: spec.md:348 says cache fields are zero when the provider doesn't report them.
- Problem: vLLM, Ollama and llama.cpp share one provider name but differ in what they report. Averages then mix "none" with "unknown".
- Recommendation: make the cache and reasoning counts optional before the always-emit mapping is fixed.
- Votes: first-pass verifier partly; rerate real, high (up from medium).

#### H6. Local servers truncate context silently

High. G1, G2. Areas: measurement, provider (merged). Add the check in phase 4 and the docs before phase 9 and P5.

- Evidence: context_exhausted comes only from a classified error (spec §1). Ollama's default num_ctx truncates the prompt without an API error, and /v1 can't set it. P5 signs phase 9 off on an Ollama run.
- Problem: the model loses the system prompt and the task partway through the run, with no record of it. Local-model comparisons are skewed.
- Recommendation: check in the loop that each turn's input_tokens don't fall below the previous turn's, and count violations on the wide event. Don't stop the run, because cache reuse can produce false positives. Document num_ctx in the example config, `lablet init` and P5.
- Votes: truth yes, decision yes, impact yes (both findings).

#### H7. The records don't say what the model saw

High. G1, G2. Areas: first pass, mcp-server-identity and token-attribution gaps (five findings merged). Decide before phase 4; capture server identity in phase 8.

- Evidence: the digest hashes config text (spec.md:693), so skill files, system_file contents and tool descriptions never enter it. The wide event has only `lablet.tools.names` and `lablet.tools.count`, and server names only. RequestBytes folds tool-spec bytes into one sum (service.rs:582-613). `gen_ai.tool.definitions` isn't emitted, although catalogue.md:51 says to adopt it.
- Problem: two A/B arms that differ in descriptions, schemas, server build or skill text produce identical setup records. A mislabelled arm merges silently. No record says how many bytes the tool surface takes.
- Recommendation: add a `lablet.tools.digest` (canonical offered specs), a prompt digest, `lablet.prompt.tools_bytes` and `skills_bytes`, and the MCP serverInfo name, version and protocol, as arrays aligned with `lablet.mcp.servers`. Split request bytes per call. Emit gated tool definitions, and put the specs in the transcript.
- Votes: truth, decision and impact yes on all five; one first-pass rerate real, high.

#### H8. The digest can't group runs, and runs can't be labelled

High. G2, G3. Areas: first pass, contracts, grader gap (three findings merged). Decide before phase 4.

- Evidence: the digest covers transcript_path, telemetry paths and telemetry.resource (spec.md:693). The Resource is fixed per Lablet (spec.md:591). The RunId is created inside `run` (spec.md:602-605).
- Problem: per-run paths or task ids give every run its own digest. A reused Lablet can't tag each run with a task, arm or trial. The mapping from task to run exists only after `run` returns.
- Recommendation: hash only the keys that affect behaviour. Take `run(RunRequest { prompt, run_id, labels })`. Declare `lablet.task.id`, `lablet.experiment.id` and `lablet.trial` on the wide event and spans, and echo them in the outcome.
- Votes: digest scope rerated real, high; the other two truth, decision and impact yes.

#### H9. A timed-out tool call isn't required to stop

High. G2, G3. Areas: tools, security (merged). Put the port contract in place before phase 4 and the bash implementation in phase 7.

- Evidence: spec.md:453 says only "adapters enforce their own per-call deadlines". Nothing requires killing the process group or sending MCP `notifications/cancelled`. tokio kills only the direct child.
- Problem: a timed-out exclusive bash call keeps writing after later calls start, which breaks the ordering promised at spec.md:25. Background grandchildren hold stdout open and hang the call past its deadline, and they outlive the run.
- Recommendation: `execute` returns Timeout only after the work has stopped. Run bash in its own process group, send SIGTERM and then SIGKILL, and stop reading at the deadline. Send the MCP cancel. Add a conformance case, and a `sleep 30 &` acceptance case.
- Votes: truth yes, decision yes, impact yes (both).

### Medium

#### M1. The output cap keeps only the start, at 100 KB, and read_file can't page

G1, G2. Areas: fit, tools, grader gap. Decide the cut rule before phase 4 and read_file's schema before phase 7. Votes: truth, decision and impact yes on all.

- Evidence: spec.md:62 and :680. mini-swe-agent and Codex keep the head and the tail, at about 10 KB.
- Problem: errors and test summaries at the end are lost, the model reruns commands with `tail`, and the tail of a large file can't be read.
- Recommendation: keep the head and the tail with an elision marker, set the default to about 10-30 KB, and give read_file offset and limit.

#### M2. The output cap applies only after the executor has buffered all output

G2, G3. Area: first pass. Decide before phase 4. First-pass verifier confirmed; rerate real, medium.

- Evidence: spec.md:554 ("Neither executor shortens output"). ToolCall has no cap field.
- Problem: a runaway command can get lablet killed for running out of memory, leaving no outcome and no wide event.
- Recommendation: put the cap in the ToolCall port and count bytes while reading.

#### M3. Long outputs can't run: a 4096 default and a 120 s whole-call deadline (moved up from first-pass drops, merged with the streaming gap)

G1, G2. Decide the timeout semantics before phase 5 and streaming before phase 7. Votes: truth yes, decision yes, impact yes on both.

- Evidence: spec.md:648, :637 and :58, with streaming deferred at :718. Thinking counts against max_tokens.
- Problem: coding runs end as output_truncated. Raising the cap turns long generations into timeouts that are retried and billed each time.
- Recommendation: see decision 2. Stream on the wire behind the same port, with an idle timeout and a generous cap on the whole call.

#### M4. Explicit mode ends on the first text-only turn

G1, G2. Area: fit. Decide before phase 4. Votes: truth yes, decision yes, impact yes.

- Evidence: spec.md:50. mini-swe-agent, SWE-agent and Terminus send a format reminder back to the model.
- Problem: success rates measure slips in the completion protocol rather than whether the task got done.
- Recommendation: send back a configurable reminder, up to a cap of about 3, and count reminders. Turn.input is the seam.

#### M5. task_complete drops sibling calls, and an empty response completes the run (moved up from first-pass drops)

G1, G2. Area: loop. Decide before phase 4. Votes: truth yes, decision yes, impact yes.

- Evidence: spec.md:50. transcript.rs:198 drops blank text.
- Problem: write_file plus task_complete ends as completed, but the write never ran and nothing signals it.
- Recommendation: see decision 3. The empty end_turn case is minor.

#### M6. task_complete arguments aren't checked against completion_schema

G1, G2. Area: loop. Decide before phase 5. Votes: truth yes, decision yes, impact yes.

- Evidence: run.rs:448-460. Validation is deferred at spec.md:720.
- Problem: "completed" overstates success in the mode graders use.
- Recommendation: validate against a subset (type, required, properties) and answer failures as tool_error. The alternative is to reject completion_schema until validation exists.

#### M7. The builtin tools match no reference loop

G1, G3. Areas: fit, tools (merged). Decide before phase 7. Votes: truth yes, decision yes, impact yes.

- Evidence: spec.md:552 lists bash, read_file and write_file. There is no edit tool, and bash's statefulness and output format aren't specified.
- Problem: rewriting whole files inflates output tokens. A lost `cd` or `export` adds calls.
- Recommendation: pick one reference profile and record it. Either mini-swe-agent's stateless bash, or Anthropic's bash plus a str_replace edit tool. Snapshot-test the bash output format.

#### M8. No progressive disclosure: skills are inlined and the tool set is fixed

G1, G2. Areas: fit, domain (merged). Record the offered set before phase 4 and settle the skills default before phase 10. Votes: truth yes, decision yes and yes/low, impact yes.

- Evidence: spec.md:659. toolset.rs:127-188 builds specs once. TurnRecord holds no offered set.
- Problem: a skill A/B compares system prompts, not skills. Tool search and skills loaded through a tool can't be represented.
- Recommendation: record the offered set on each turn and read specs from run state. See decision 5.

#### M9. MCP server instructions are dropped

G1, G2. Area: tools. Decide before phase 8. Votes: truth yes, decision yes, impact yes.

- Evidence: the spec never reads InitializeResult `instructions`. Claude Code puts them in its system prompt.
- Problem: lablet understates how usable a server is.
- Recommendation: append them after the skills, count their bytes, and add a per-server switch.

#### M10. MCP servers outlive runs, and a dead server stays dead (moved up from first-pass drops)

G2. Area: first pass. Decide before phase 8. Votes: truth yes, decision yes, impact yes/low.

- Evidence: spec.md:558. There is no restart or fail-fast.
- Problem: runs after a crash end as tool_errors_exhausted, which looks like model failure.
- Recommendation: see decision 9.

#### M11. The wide event reports requested settings, not applied ones

G1, G2, G4. Areas: measurement, provider (merged). Decide before phase 4 and phase 5, and map reasoning_effort in phase 9. Votes: truth yes, decision yes, impact yes.

- Evidence: Anthropic ignores seed and OpenAI ignores effort, yet both are reported. The wide event has no response model, system_fingerprint or service tier.
- Problem: an effort A/B on OpenAI looks valid but has no effect.
- Recommendation: reject ignored settings at build and map effort to reasoning_effort. Add the set of response models and the OpenAI fingerprint and tier.

#### M12. The parallel-tool-call flag can't be set or recorded

G1, G2. Area: provider. Decide the config key in phase 5 and the mapping before phase 7. Votes: truth yes, decision yes, impact yes.

- Problem: single-action loops like SWE-agent can't be reproduced, and a change in the provider's default goes unrecorded.
- Recommendation: add `model.parallel_tool_calls`, map it for each provider, report it and include it in the digest.

#### M13. Billed failed attempts lose their tokens, and a bad tool name is retried

G1, G2. Areas: measurement, loop (merged). Decide before phase 4. Votes: truth yes, decision yes/low, impact yes.

- Evidence: provider.rs:43-49. service.rs:352 records only latency.
- Problem: cost and the token budget under-count exactly in the arms that fail more often. An invalid tool name is model behaviour that gets re-rolled.
- Recommendation: `Option<Usage>` on the error, with failed-attempt totals. Treat an invalid name as an unknown call answered with an error result.

#### M14. ProviderError has no retry-after (moved up from first-pass drops)

G2. Area: loop. Decide before phase 4. Votes: truth yes, decision yes, impact yes.

- Problem: under parallel load, 429s run through the retry ladder (about 3.5 s in total), and heavier arms lose more runs.
- Recommendation: add an optional retry-after hint that the retry policy honours, plus jitter.

#### M15. OpenAI-compatible error classification rests on "and equivalents"

G2, G3. Area: provider. Decide before phase 9. Votes: truth yes, decision yes, impact yes.

- Problem: vLLM's context overflow becomes provider_error, and insufficient_quota is retried as if the provider were flaky.
- Recommendation: write a classification table with wiremock cases, and say whether the max_completion_tokens fallback happens once per adapter or per call.

#### M16. The prompt cache is shared across runs

G2. Area: measurement. Decide before phase 7. Votes: truth yes, decision yes/low, impact yes.

- Problem: cost and first-call latency depend on how the composer schedules runs.
- Recommendation: see decision 10. Add `model.cache_scope: shared | run` and report it.

#### M17. Deadlines ignore the time left in the run

G2, G3. Areas: first pass, loop (merged). Decide before phase 4, and at the latest phase 7. Votes: truth yes, decision yes, impact yes/low.

- Evidence: service.rs:336 and :510. The spec's "at most one call" bound (spec.md:63) is false.
- Recommendation: use min(limit, time remaining) as each call's deadline and correct spec.md:63. A NotRun status can be added later without a breaking change.

#### M18. Only Ctrl-C is handled

G2, G3. Area: security. Decide before phase 5. Votes: truth yes, decision yes, impact yes.

- Problem: Docker and Kubernetes send SIGTERM. Lablet then dies with no outcome and no wide event, so long runs go missing from the data.
- Recommendation: treat SIGTERM like SIGINT. On a second signal or a short grace period, abort calls, then write and flush.

#### M19. Content capture can lose the wide event

G2. Areas: security, contracts (merged). Decide before phase 4 and verify in phase 6. Votes: truth yes, decision yes, impact yes.

- Problem: the full history grows quadratically, the collector rejects batches over 4 MiB, and the queue drops records silently.
- Recommendation: flush content first, then send the wide event in its own export. Set an attribute length limit and report a count of dropped records.

#### M20. Error strings bypass capture_content

G3. Area: security. Decide before phase 4. Votes: truth yes, decision yes, impact yes/low.

- Recommendation: cap messages, strip URL credentials and queries, and gate response bodies behind capture_content. Show config values before `${VAR}` substitution, and add a test that a substituted value never reaches stderr.

#### M21. The default tool root contains lablet's own inputs and outputs

G2, G3. Area: security. Decide before phase 5. Votes: truth yes, decision yes, impact yes.

- Problem: the agent can read earlier runs and edit the config or telemetry.
- Recommendation: make the root required, and reject a root that overlaps the config, prompt, skill files, transcript or telemetry paths.

#### M22. The transcript has no run id, and a fixed path overwrites it

G2, G3. Area: contracts. Decide before phase 4. Votes: truth yes, decision yes, impact yes.

- Recommendation: add run_id, config_digest, version, started_at, the model and the offered tools. Default the path to `lablet-<run_id>.transcript.json`. Return the document from `run`.

#### M23. The exit-1 rule for rejected credentials can't be reached

G2, G3. Area: contracts. Decide before phase 5, with the error kind in phase 4. Votes: truth yes, decision yes, impact yes.

- Evidence: RunStarted is emitted unconditionally (service.rs:184-195), and auth errors fall into Fatal.
- Recommendation: `run` always prints an outcome. Add `ProviderErrorKind::Auth`. Keep exit 1 for failures before build returns.

#### M24. The outcome JSON has no schema_version

G3. Area: first pass. Decide before phase 4. First-pass verifier partly; rerate real, medium.

- Recommendation: add the version when phase 4 rebuilds the type, and let readers tolerate unknown fields.

## What the design gets right

- The Run, Final and Pending typestate makes "every call answered once, in call order" structural.
- Point R reads the finish reason before tool calls, so a truncated call never runs.
- Tool calls are grouped into shared and exclusive runs, with outcomes in call order. This matches Claude Code and keeps transcripts independent of timing.
- Malformed tool arguments go back to the model as text it can correct.
- Usage is normalised through named constructors, and pricing subtracts cache fields, so cached tokens can't be billed twice.
- One clock and one rounding function mean a total always equals the sum of its parts.
- Telemetry is contract-first: a Weaver registry, pinned conventions and live-check.
- RunSummary is computed from the transcript, so the outcome, spans and wide event can't drift apart.
- The digest is taken before `${VAR}` substitution, and observers can't fail or slow the run.
- The ATIF mapping is written out field by field ahead of phase 10.

## Refuted or low

- bash isn't a sandbox (survived as low): reword quality-bar to "rooted, not sandboxed". Production loops rely on containers too.
- Per-tool keys vary per run: refuted. Per-tool columns are what a tool A/B needs, and they're bounded.
- Semconv renames force a major version: low. The cost is overstated under 0.x. Set schema_url on the scope.
- Rigour spent on internal types: refuted. It's about process pace, not a design flaw.
- The transcript is both record and context: refuted. Compaction isn't on the roadmap, and adding it later is additive.
- Final is terminal, so a user can't speak again: refuted. Out of the brief's scope, and the seam is reserved.
- No context management: low. State it in the spec as a known difference.
- No time split on the wide event: refuted. Deliberately deferred, and additive.
- Cost from run totals: low. It only matters for tiered pricing; state the flat-rate limit.
- request.bytes counts signatures: low. A secondary metric; excluding signatures is cheap.
- A retry cut by the timeout drops the error: low. A one-line fix: attach the last error message.
- The token budget counts cache reads at full weight: low. Fix the "billed" wording, and consider a max_cost option.
- Spend comes only from turns: refuted. No side model calls are planned.
- Turn input has no author: refuted. No injected input is planned, and adding an author later is additive.
- Nested run identity: refuted. Out of scope, and `&mut self` already rules it out.
- Omitted images aren't counted: low. Add a counter and map resource_link in phase 8.
- Symlink escapes: low. Containers are the boundary; reword the promise.
- Two clocks and per-call truncation: low. The bias is at most 1 ms per call; sum durations, then round.
- `-` sends OTLP to stderr alongside diagnostics: low. Opt-in; drop `-` or make it imply quiet.
- No slot for time to first token: refuted. It needs streaming, and adding it later is additive.
- The grader claim reaches too far: low. Reword the brief.
- Bytes aren't tokens: refuted. Per-span token counts with `lablet.turn` already cover it.
