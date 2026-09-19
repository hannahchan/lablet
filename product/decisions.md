# Decisions

Append-only. Newest last. Each entry: date, decision, why. Supersede by adding a new entry that names the old one.

## 2026-09-17 Rust

Rust, to stay close to the Apache Arrow and DataFusion ecosystem the analysis side will use.

## 2026-09-17 Own agent loop

Hand-rolled loop on provider HTTP APIs rather than wrapping an agent SDK. Instrumentation is the product; a black-box loop can't be instrumented on our terms.

## 2026-09-17 Explicit architecture, UsefulBytes shape

Same rings, port and adapter conventions, and layer lint as UsefulBytes. Shared language across the two repositories is worth more than a flatter layout. Sized down: fewer crates, no config provenance machinery, lighter documentation.

## 2026-09-17 Multi-provider from day one

`ModelProvider` is a port with Anthropic and OpenAI-compatible adapters. A lablet must be able to run against Ollama and similar local models.

## 2026-09-17 OTLP is the primary telemetry output

OpenTelemetry traces and logs following the GenAI semantic conventions. JSONL is a local fallback, not a second format to maintain semantics for.

## 2026-09-17 Grading, containers, and orchestration are out of scope

A lablet is one loop in one process. A grader is another lablet. The larger framework owns everything else.

## 2026-09-17 `serde_json::Value` allowed in domain

Tool inputs and outputs are JSON by definition, so the domain needs a JSON value type. serde derives stay out of domain and application; wire forms belong to adapters and the composition root. Superseded on 2026-09-18 by "serde derives allowed in the domain."

## 2026-09-17 Documentation is three areas, four product files

`product/` holds brief, spec, build plan, and this log. `contributing/` holds one conventions document. `lablet/docs/` holds user-facing docs. No ADR folder, no templates, no metadata headers.

## 2026-09-18 One wide event per run

Every run ends with a single wide event carrying the whole run summary, emitted by every observer from one `RunSummary` the loop accumulates. Analysis over many runs should work from one row per run; spans are for drilling into a single run.

## 2026-09-18 Failure modes are distinct stop reasons

Truncated output, context exhaustion, and token budget are their own stop reasons rather than folded into `provider_error` or `completed`. They're the findings a benchmark exists to surface.

## 2026-09-18 Transcript is separate from telemetry

`run.transcript_path` writes the conversation as JSON independently of `capture_content`. A grader lablet needs the conversation; telemetry content capture is a separate, usually off, concern.

## 2026-09-18 Observers never fail or slow the run

Telemetry export is buffered and its failures go to the diagnostic log. Measured latency must reflect the agent, not the exporter.

## 2026-09-18 Contract-first telemetry with OpenTelemetry Weaver

Every attribute, span, event, and the wide event is declared in a Weaver registry before it's emitted. Rust constants and builders and the telemetry docs are generated from it, and emitted telemetry is validated against it in CI. The telemetry surface is a product contract, so it needs a source of truth that's not the code. This is an experiment with Weaver; if the tooling doesn't hold up, the registry stays and the codegen is replaced.

## 2026-09-18 The fake provider is a product feature

`provider-fake` ships as a supported adapter so users can test frameworks built on lablet without spending tokens. It's also what lablet's own smoke tests and docs run on, so it can't rot.

## 2026-09-18 Quality bar is a document with gates behind it

`product/quality-bar.md` lists commitments that are either user-verifiable or enforced by `cargo xtask`. Anything that can't be one or the other doesn't go on the page.

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

A GenAI or core semantic-convention attribute is used wherever one exists. A `lablet.*` attribute is added only when the run can't be described without it, with a one-line justification in the registry. Extensions considered and deferred live in the research catalogue. This keeps the registry small and every extension a visible decision.

## 2026-09-18 Root-span usage totals reuse `gen_ai.usage.*`

The conventions allow the aggregate on `invoke_agent`, and Honeycomb and Datadog read it there. A query that sums `gen_ai.usage.*` over every span in a trace double counts; the docs say to filter on `gen_ai.operation.name`, and the wide event is the intended per-run source.

## 2026-09-18 `input_tokens` includes cached tokens

Matches the conventions and Harbor. Inspect-style consumers subtract `cache_read_tokens` and `cache_write_tokens`, which are reported beside it.

## 2026-09-18 Turn index, not turn span

The conventions define no turn span and the idiomatic tree is `invoke_agent` with `chat` and `execute_tool` directly beneath. `lablet.turn` on each child recovers per-turn analysis. A turn span is trivial to add later and is listed as an open question.

## 2026-09-18 Redacted content is omitted

Idiomatic OpenTelemetry omits an attribute it's not populating. Byte-count attributes are always present so dashboards keep a stable column.

## 2026-09-18 ATIF export in phase 7

Harbor's Agent Trajectory Interchange Format is the idiomatic trajectory format for eval frameworks, but it's not an OpenTelemetry concern and would grow phase 3a. The phase 1 transcript model is designed to map to it losslessly.

## 2026-09-18 Spec corrected to current semantic conventions

From the instrumentation research: `gen_ai.usage.cache_write.input_tokens` replaces the `cache_creation` spelling; provider failures are `gen_ai.client.operation.exception` log records rather than the deprecated `exception` span event; `mcp.*`, `jsonrpc.*`, and `network.transport` go on the `execute_tool` span with trace context injected into `params._meta`; `gen_ai.request.seed` and `gen_ai.request.reasoning.level` replace lablet-named equivalents; `gen_ai.tool.type` uses `function` and `extension`; `session.id` is emitted beside `gen_ai.conversation.id`.

## 2026-09-18 Dual licensed MIT OR Apache-2.0

The Rust ecosystem convention. Apache-2.0 brings the patent grant and contribution terms; MIT keeps GPLv2 compatibility and simplicity. Users pick either. Contributions are accepted under both without a separate agreement.

## 2026-09-18 Build plan restructured to twelve phases

The telemetry contract is its own phase so the Weaver templates can't stall the scaffold; the composition root is split into library and CLI; OpenTelemetry moves before the first real provider because a wrong span shape costs more to fix late than a wrong provider mapping; features and hardening are separate phases so "done" is unambiguous. Phase numbers in earlier entries refer to the previous numbering.

## 2026-09-18 Trace context reaches MCP through the observer

The OTel observer opens the tool span on `ToolCallStarted`; the loop then asks the observer for that span's W3C trace context and places it on the `ToolCall`, and the MCP adapter injects it into `params._meta`. This keeps OpenTelemetry types out of the application layer and lets JSONL-only runs answer `None`.

## 2026-09-18 Small pull requests, human merges

Work lands in small, reviewable PRs, each one logical unit with CI green, merged by a human. A phase is many PRs. The building agent may clarify the spec in a PR but must stop and ask before changing a recorded decision. Manual acceptance items are signed off by a human in the closing PR of the phase.

## 2026-09-18 Weaver approach is researched before phase 1

Weaver is experimental. A dedicated research document and spike under `product/research/weaver/` settles the registry syntax, vendoring, template starting point, and live-check usage before the telemetry contract phase begins; phase 1 follows it.

## 2026-09-19 Weaver approach fixed by the spike

Registry in v2 syntax with lablet-owned spans and events that reference `gen_ai.*` keys, dependencies and weaver-packages vendored under `deps/` with relative paths (Weaver has no dependency cache), constants-only Rust codegen from the spike templates, docs from the vendored markdown templates, and live-check run without `--v2` because the v2 index ignores dependency attributes. Weaver is pinned at v0.26.1 and upgraded only in a dedicated PR. Span names and required span attributes are held by unit tests because live-check doesn't match spans to definitions. The fallback keeps the registry and replaces codegen with an xtask step over the resolved JSON.

## 2026-09-19 Raw data, never reports

A lablet's telemetry is valuable in aggregate, so every record is shaped for aggregation by the composing system and lablet never aggregates, reports, or visualises. Consequences: one flat wide event per run; join keys on every record; the JSONL file is a flat rendering of the trace with registry attribute names and a schema URL, making it the fourth stable contract; raw counts rather than ratios; no `report` command, CSV, or cross-run state. The CLI keeps one human-readable summary line on stderr for the person at the keyboard.

## 2026-09-19 Parquet exporter considered, deferred

A Parquet or Arrow writer would help developers without a collector and fits as a pluggable exporter inside the OTel adapter, likely in the OTel-Arrow layout. Deferred because the file layout is a contract decision and the JSONL file already covers the lightweight case. Recorded as an open question in the spec.

## 2026-09-19 One telemetry observer, pluggable exporters, OTLP/JSON file

Supersedes the JSONL observer. `telemetry-otel` maps events to OTel spans and log records once; exporters below it are the SDK's pluggable traits. The file output is OTLP/JSON, the Collector's own file format, so the file and the network carry identical data including the full resource, the file can be replayed into a collector, and lablet maintains no envelope of its own. A flat lablet line format was rejected as a second rendering of the same contract. The Parquet exporter, if added, is a third exporter in the same slot. This withdraws the "fourth stable contract" clause of the raw-data entry: the file format is the Collector's, not lablet's, and the quality bar now reads "three contracts, one borrowed."

## 2026-09-19 Composer resource attributes stay on the Resource

`telemetry.resource` keys are chosen by the composer and can't be declared in the registry, and live-check reports undeclared span attributes as violations. They live on the OTel Resource only, which every export batch carries. The join keys duplicated onto spans and the wide event are `gen_ai.conversation.id`, `session.id`, and `lablet.config.digest`.

## 2026-09-19 `opentelemetry-semantic-conventions` isn't a dependency

The generated `telemetry-registry` crate already holds every `gen_ai.*` and core name at the vendored versions. A second semconv version in the workspace is the drift the registry exists to prevent.

## 2026-09-19 Delivery process for the build

Supersedes "Small pull requests, human merges." Branches, fast-forward to `main` and push when a logical piece lands, no pull requests for now. `cargo xtask pre-push` passes locally before every merge and CI is checked after. One phase per explicit go-ahead; the builder goes as far as it can and stops where a human is needed. Each phase ends with a multi-agent code review, a phase report, and a stop. The builder may make architectural decisions, recording each here and reporting them at the phase end. Expected to be adjusted as the process is learned.

## 2026-09-19 Phase 0 decisions made by the builder

- **Rust 1.98.1, MSRV 1.96.** The toolchain matches UsefulBytes and was already installed; `rust-version` is the pinned toolchain minus two minors, per spec §8.
- **`serde-saphyr` instead of `serde_yaml`.** `serde_yaml` is archived at `0.9.34+deprecated`. `serde-saphyr` is pure Rust and reads and writes externally tagged enums in map form, the same shape as the JSON serde form, so fake-provider scripts work in YAML or JSON with plain derives. The `serde_yaml` forks need `singleton_map_recursively` attributes on the model, which the domain may not carry. It has no `Value` type, so the raw config tree that `--set` edits is `serde_json::Value`.
- **OpenTelemetry 0.33.0**, published the day before pinning, because its SDK fixes a batch-processor race with `force_flush` that the "file is complete when `run` returns" guarantee depends on. Falling back to 0.32 changes no other pin.
- **The layer lint forbids more than the first draft listed**: `tonic`, `axum`, and `hyper` join the inward-forbidden set, names match by family, and no shipped crate may depend on a `tests/` crate. The spec and contributing tables now say so.
- **No pedantic relaxations.** UsefulBytes allows `missing_errors_doc` and `missing_panics_doc`; lablet doesn't, so `# Errors` and `# Panics` sections are gate-enforced.
- **One root `clippy.toml`** covers both workspaces and lets tests `unwrap`. Integration targets declare modules as `#[cfg(test)] mod name;` so helpers are covered.
- **mise backends.** Weaver and cargo-llvm-cov come from GitHub releases with checksums in `mise.lock`; the `ubi` backend the Weaver research used is deprecated. cargo-mutants is compiled from source because upstream ships no arm64 macOS binary and the x86_64 fallback can't link under Rosetta.
- **CI runs on every pushed branch** as a matrix of `ci`, `coverage`, and `mutants`, since there are no pull requests. On `main` the changelog base is the commit before the push; on branches it's the merge-base with `origin/main`.
- **`scripts/setup.sh`** is the one command for a fresh clone.

## 2026-09-19 The lints reject what lablet doesn't use

After the phase 0 review taught the xtask to model exotic Cargo features and grew it past 6,400 lines, it was reworked to refuse them instead: member globs, `.` and `..` or absolute member paths, `[workspace] exclude` and `default-members`, a root `[package]`, `[patch]`, `[replace]`, and cargo-config `patch`, `paths`, and `source` are each one diagnostic saying the lints don't support it. Every dependency in a member manifest is exactly `workspace = true`; versions, paths, sources, and renames live only in `[workspace.dependencies]`. This closes the name-collision, implicit-member, and git-pin holes with one rule. The xtask is about 4,500 lines, half of them tests. A lightweight project's gates should be small enough to read.

- **Why-comments live in `[workspace.dependencies]` only**, as a comment on the line directly above each entry. Member manifests need none. Trailing comments and group headers don't count.
- **`anyhow`, `eyre`, and mocking frameworks are banned in `lablet/deny.toml`**, not in xtask code. The ban is graph-wide, so an upstream crate that pulls one in must be listed under `wrappers` with a reason.
- **Unit tests live in sibling `tests.rs` files**, and coverage ignores them by file name rather than parsing Rust. Enforced in the three floor crates. A floor crate passes with nothing to measure only while it defines no function, so a floor crate's first function lands with its test.
- **One integration target per crate and no symlinked members are review conventions**, not gates.

## 2026-09-19 Review workflows judge worth, not only truth

The phase 0 review accepted any finding that could be reproduced, and the fix pass modelled every one. From phase 1 the review adds a judgement before fixing: is this worth handling in a lightweight project, or should the input be rejected, or the finding dropped.

## 2026-09-19 Phase 0 retrospective changes

- **Reviews are scaled to risk and budgeted.** The phase 0 review used 109 agents on scaffolding and cost about ten times the build, because every reproducible finding survived and was fixed. Reviews now cap findings, triage before verifying, verify cheaply, and carry a budget of about 10 to 15 percent of the build cost. The rules are in `contributing/README.md`.
- **Comments say why, and nothing else.** No narration, no facts that drift, no restating code or docs.
- **`cargo xtask` follows the developer's workflow**, grouped as UsefulBytes groups it, with the everyday tasks (`check`, `build`, `run`, `fix`, `setup`, `clean`) added, `fmt` formatting by default, cheap steps first in the gates, and a quiet green gate.
- **Prose is linted with Vale and the Microsoft writing style package**, vendored at a pinned version, gating on errors in pre-commit. Research inventories are excluded because they quote other projects' identifiers.
