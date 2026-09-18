# Decisions

Append-only. Newest last. Each entry: date, decision, why. Supersede by adding a new entry that names the old one.

## 2026-09-17 Rust

Rust, to stay close to the Apache Arrow and DataFusion ecosystem the analysis side will use.

## 2026-09-17 Own agent loop

Hand-rolled loop on provider HTTP APIs rather than wrapping an agent SDK. Instrumentation is the product; a black-box loop cannot be instrumented on our terms.

## 2026-09-17 Explicit architecture, UsefulBytes shape

Same rings, port and adapter conventions, and layer lint as UsefulBytes. Shared language across the two repositories is worth more than a flatter layout. Sized down: fewer crates, no config provenance machinery, lighter documentation.

## 2026-09-17 Multi-provider from day one

`ModelProvider` is a port with Anthropic and OpenAI-compatible adapters. A lablet must be able to run against Ollama and similar local models.

## 2026-09-17 OTLP is the primary telemetry output

OpenTelemetry traces and logs following the GenAI semantic conventions. JSONL is a local fallback, not a second format to maintain semantics for.

## 2026-09-17 Grading, containers, and orchestration are out of scope

A lablet is one loop in one process. A grader is another lablet. The larger framework owns everything else.

## 2026-09-17 `serde_json::Value` allowed in domain

Tool inputs and outputs are JSON by definition, so the domain needs a JSON value type. serde derives stay out of domain and application; wire forms belong to adapters and the composition root. Superseded on 2026-09-18 by "serde derives allowed in the domain".

## 2026-09-17 Documentation is three areas, four product files

`product/` holds brief, spec, build plan, and this log. `contributing/` holds one conventions document. `lablet/docs/` holds user-facing docs. No ADR folder, no templates, no metadata headers.

## 2026-09-18 One wide event per run

Every run ends with a single wide event carrying the whole run summary, emitted by every observer from one `RunSummary` the loop accumulates. Analysis over many runs should work from one row per run; spans are for drilling into a single run.

## 2026-09-18 Failure modes are distinct stop reasons

Truncated output, context exhaustion, and token budget are their own stop reasons rather than folded into `provider_error` or `completed`. They are the findings a benchmark exists to surface.

## 2026-09-18 Transcript is separate from telemetry

`run.transcript_path` writes the conversation as JSON independently of `capture_content`. A grader lablet needs the conversation; telemetry content capture is a separate, usually off, concern.

## 2026-09-18 Observers never fail or slow the run

Telemetry export is buffered and its failures go to the diagnostic log. Measured latency must reflect the agent, not the exporter.

## 2026-09-18 Contract-first telemetry with OpenTelemetry Weaver

Every attribute, span, event, and the wide event is declared in a Weaver registry before it is emitted. Rust constants and builders and the telemetry docs are generated from it, and emitted telemetry is validated against it in CI. The telemetry surface is a product contract, so it needs a source of truth that is not the code. This is an experiment with Weaver; if the tooling does not hold up, the registry stays and the codegen is replaced.

## 2026-09-18 The fake provider is a product feature

`provider-fake` ships as a supported adapter so users can test frameworks built on lablet without spending tokens. It is also what lablet's own smoke tests and docs run on, so it cannot rot.

## 2026-09-18 Quality bar is a document with gates behind it

`product/quality-bar.md` lists commitments that are either user-verifiable or enforced by `cargo xtask`. Anything that cannot be one or the other does not go on the page.

## 2026-09-18 serde derives allowed in the domain

Supersedes the 2026-09-17 entry. The conversation model is serialised by JSONL telemetry, the transcript writer, and the fake provider's scripts; banning derives would mean three hand-written mirrors of the same types. The domain derives once and every JSON surface reuses it. Provider wire formats stay as separate types in their adapters. The layer lint no longer lists `serde`.

## 2026-09-18 Retry budget is per provider call

`run.max_retries` counts attempts for one call and resets on success. A per-run budget made a long run lose to a few spread-out rate limits, which measures the provider's weather rather than the agent.

## 2026-09-18 MCP tool names are never prefixed by default

Tools keep the names their server reports so measurements reflect the server as-is and allow and deny lists stay stable when servers are added. A collision is a build error; `prefix_tools: true` on a server is the escape hatch.

## 2026-09-18 Stop policy has two evaluation points

Before each provider call and after each tool phase. One evaluation per iteration let the tool-error cap, timeout, and cancellation fire one provider call late.

## 2026-09-18 Weaver generates constants, not builders

There are no upstream Rust builder templates for Weaver. Phase 0 generates attribute name constants and enums only; adapters compose spans and records from them. Builders can be added later if the constants prove insufficient.

## 2026-09-18 GenAI conventions are vendored at a pinned commit

The GenAI semantic conventions moved to their own repository with no tagged release and Development stability throughout. The registry vendors core and GenAI at pinned commits, the policy accepts Development for imported `gen_ai.*`, and upstream renames are treated as breaking changes to lablet's contract.

## 2026-09-18 Semantic conventions first, extensions last

A GenAI or core semantic-convention attribute is used wherever one exists. A `lablet.*` attribute is added only when the run cannot be described without it, with a one-line justification in the registry. Extensions considered and deferred live in the research catalogue. This keeps the registry small and every extension a visible decision.

## 2026-09-18 Root-span usage totals reuse `gen_ai.usage.*`

The conventions allow the aggregate on `invoke_agent`, and Honeycomb and Datadog read it there. A query that sums `gen_ai.usage.*` over every span in a trace double counts; the docs say to filter on `gen_ai.operation.name`, and the wide event is the intended per-run source.

## 2026-09-18 `input_tokens` includes cached tokens

Matches the conventions and Harbor. Inspect-style consumers subtract `cache_read_tokens` and `cache_write_tokens`, which are reported beside it.

## 2026-09-18 Turn index, not turn span

The conventions define no turn span and the idiomatic tree is `invoke_agent` with `chat` and `execute_tool` directly beneath. `lablet.turn` on each child recovers per-turn analysis. A turn span is trivial to add later and is listed as an open question.

## 2026-09-18 Redacted content is omitted

Idiomatic OpenTelemetry omits an attribute it is not populating. Byte-count attributes are always present so dashboards keep a stable column.

## 2026-09-18 ATIF export in phase 7

Harbor's Agent Trajectory Interchange Format is the idiomatic trajectory format for eval frameworks, but it is not an OpenTelemetry concern and would grow phase 3a. The phase 1 transcript model is designed to map to it losslessly.

## 2026-09-18 Spec corrected to current semantic conventions

From the instrumentation research: `gen_ai.usage.cache_write.input_tokens` replaces the `cache_creation` spelling; provider failures are `gen_ai.client.operation.exception` log records rather than the deprecated `exception` span event; `mcp.*`, `jsonrpc.*`, and `network.transport` go on the `execute_tool` span with trace context injected into `params._meta`; `gen_ai.request.seed` and `gen_ai.request.reasoning.level` replace lablet-named equivalents; `gen_ai.tool.type` uses `function` and `extension`; `session.id` is emitted beside `gen_ai.conversation.id`.

## 2026-09-18 Dual licensed MIT OR Apache-2.0

The Rust ecosystem convention. Apache-2.0 brings the patent grant and contribution terms; MIT keeps GPLv2 compatibility and simplicity. Users pick either. Contributions are accepted under both without a separate agreement.

## 2026-09-18 Build plan restructured to twelve phases

The telemetry contract is its own phase so the Weaver templates cannot stall the scaffold; the composition root is split into library and CLI; OpenTelemetry moves before the first real provider because a wrong span shape costs more to fix late than a wrong provider mapping; features and hardening are separate phases so "done" is unambiguous. Phase numbers in earlier entries refer to the previous numbering.

## 2026-09-18 Trace context reaches MCP through the observer

The OTel observer opens the tool span on `ToolCallStarted`; the loop then asks the observer for that span's W3C trace context and places it on the `ToolCall`, and the MCP adapter injects it into `params._meta`. This keeps OpenTelemetry types out of the application layer and lets JSONL-only runs answer `None`.

## 2026-09-18 Small pull requests, human merges

Work lands in small, reviewable PRs, each one logical unit with CI green, merged by a human. A phase is many PRs. The building agent may clarify the spec in a PR but must stop and ask before changing a recorded decision. Manual acceptance items are signed off by a human in the closing PR of the phase.

## 2026-09-18 Weaver approach is researched before phase 1

Weaver is experimental. A dedicated research document and spike under `product/research/weaver/` settles the registry syntax, vendoring, template starting point, and live-check usage before the telemetry contract phase begins; phase 1 follows it.
