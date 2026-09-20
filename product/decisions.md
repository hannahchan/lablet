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

## 2026-09-19 dprint formats everything rustfmt doesn't

Adopted before phase 1 so the telemetry registry's YAML and the docs are formatted from the start. The config follows UsefulBytes (line width 100, Markdown wrapping left alone, no reordering of Cargo manifest keys) with a YAML plugin added and the vendored and generated trees excluded. It runs inside `cargo xtask fmt` and the pre-commit gate.

## 2026-09-19 Vale styles are synced, not vendored

Supersedes the vendoring clause of the phase 0 retrospective entry. Vale's guidance treats `StylesPath` as build output that git ignores and `vale sync` re-creates. `Packages` takes a release URL, which pins the version without committing the package, and the sync leaves our vocabulary alone. The gate passes `--no-global` so a developer's own Vale config can't change the result. The cost is that a fresh clone and CI need the network once.

## 2026-09-19 Adopted from UsefulBytes before phase 1

shellcheck as `cargo xtask lint-shell` in pre-commit, a Claude Code session-start hook that reports where a session stands against `main`, and editor recommendations under `.vscode/`. Link checking, dependency upgrade, SBOM, and benchmark report tasks wait until there's something for them to act on. A project permission allowlist and a commit skill were considered and not adopted for now.

## 2026-09-20 Phase 1 decisions made by the builder

- **The Weaver research was wrong on two points.** Vendoring lablet's own dependency paths isn't enough: the vendored GenAI manifest names core semconv by git URL, and Weaver clones every git path it meets. The vendoring script rewrites that one line and records the change in `SOURCES`; it's the only vendored byte that differs from upstream. And Weaver reads `.weaver.toml` sections by subcommand name, so the table is `[live-check]`, not `[live_check]`.
- **Pins live in `lablet/telemetry/vendor.sh` only**, by commit, fetched shallowly. Core semconv is pinned by the commit behind its tag, since a tag can move. `weaver vendor --check` fetches again and compares byte for byte, and a weekly workflow runs it. That replaces the check against git URLs that the plan first named.
- **Provider-failure and content records keep the GenAI event names**, declared in lablet's registry. The check passes and live-check matches them by name, so backends that know those names recognise them.
- **Join keys are required on every span and log record** through one internal attribute group: `gen_ai.conversation.id`, `session.id`, and `lablet.config.digest`.
- **Enums only for lablet's three closed sets**: stop reason, completion mode, and tool source. The conventions' open enums would add about sixty unused variants.
- **Per-signal key lists reference the attribute constants**, so a key without a constant doesn't compile.
- **The lablet policy has four rules**: justification note, `development` stability, nothing defined outside `lablet.*`, and an explicit requirement level other than `recommended` on every span and event attribute. The last two came from the phase review.
- **Live-check promotes an undefined enum value to a violation for `lablet.*` only.** The conventions' enums are open, so the fake provider's name stays information.
- **A tool span's `error.type`** is the `ToolErrorKind` when the executor failed and the MCP conventions' `tool_error` when the tool returned an error result.
- **Lablet owns its diagnostic templates for `weaver check`**, because Weaver's GitHub format prints policy violations only and its terminal format buries them.
- **`gen_ai.request.model` is on the root span**, as the conventions recommend. `gen_ai.agent.name` is always `lablet`.

## 2026-09-20 Phase 2 decisions made by the builder

- **Three stop points, not two.** The loop decides before a provider call, after a provider response, and after a tool phase. Each point has a documented precedence, so one state always gives one answer. The tool-error cap leads after a tool phase because it alone says the run was failing, not merely long. A response that finishes the task completes the run even when it also used up a limit.
- **A truncated `task_complete` is `output_truncated`, not `completed`.** A response cut off at `max_tokens` can end inside the call's arguments, and a run mustn't end on a partial result. This came from the phase review.
- **Every limit is met when the run reaches it.** `max_turns: 2` stops after turn 2's tool phase, elapsed equal to the timeout stops, and tokens equal to the budget stops.
- **`run.max_retries` counts retries, not attempts.** `3` allows four attempts and `0` never retries. The first build read it as attempts because of how scenario E2 was worded; the name now means what it says, and E2 is four errors.
- **Backoff is `base * 2^(n - 1)` capped at `max`, with no jitter**, from `run.retry_backoff_base` (500ms) and `run.retry_backoff_max` (30s). The loop checks the run timeout before each wait.
- **Durations are whole milliseconds in `u64` fields named `*_ms`.** The model holds no `Duration`; the loop converts once where it reads the clock, so no observer rounds for itself.
- **`RunSummary` doesn't repeat what `RunOutcome` holds.** Usage and the tool call total are read from the outcome inside it, so no two fields can disagree.
- **`RunContext` carries `capture_content`.** The loop reads it to fill content fields, and an observer reads it before emitting the result. This came from the phase review.
- **Pricing bills uncached input once.** `input_tokens` includes cached tokens, so the input rate applies to input minus cache reads and writes, and each cache count is billed at its own rate.
- **`ToolName` is 1 to 64 of `[a-zA-Z0-9_-]`**, what both provider APIs accept. An MCP tool name that breaks the rule is a build error. A model tool call that breaks the rule counts as malformed, not as an unknown tool.
- **Reading a message from its serde form doesn't validate it.** Adapters call `validate` and report `Malformed`.
- **Closed enum spellings are pinned to the registry in `lablet-conformance`**, the one crate that may depend on both, through exhaustive matches.
- **The outcome fixture is `lablet/tests/fixtures/outcome.json`**, read by a test in `lablet-model`.

## 2026-09-20 Domain revision after an independent design review

An independent review of the two domain crates, checked against the real Anthropic, MCP, and OpenTelemetry shapes, led to these changes. Several supersede entries from the phase 2 list above.

- **The finish reason is read before the tool calls.** A refusal or content filter stops the run as `refused`, a response cut off at `max_tokens` as `output_truncated`, and a context-window finish as `context_exhausted`, in both modes. A cut-off response never has a tool call executed, because a truncated tool input can parse as a valid partial object. This supersedes the narrower rule about a truncated `task_complete`, and the earlier rule that the tools run and the loop goes on. A refusal isn't a completed run, so `refused` exits with code 2.
- **`FinishReason` knows the providers' spellings.** Every Anthropic and OpenAI spelling maps to a variant, and only an unknown string is `Other`. An unknown reason with no tool calls still completes a natural-mode run, because from an OpenAI-compatible server it's usually a normal end, and the wide event carries the finish reasons.
- **`Thinking` is a sum type**: provider default, adaptive, a non-zero budget, or disabled. Effort is a separate request setting, since it isn't part of thinking on either provider.
- **Tool results are text.** No provider has a wire form for JSON tool results, so each adapter would turn the value into text differently and the same run would cost different tokens by provider. Image, audio, and binary content from an MCP tool isn't carried; one function renders the placeholder line that names the kind, the MIME type, and the byte count.
- **The run accumulates itself in the model**, as pure methods, and `RunSummary` is built only by its `finish`. It's the one place a duration becomes whole milliseconds, which supersedes the earlier rule that the loop converts once. It owns the running usage and the consecutive tool-error count.
- **`turns` counts model responses received** and is derived from the completions recorded, so a run whose first provider call fails has zero turns.
- **Per-tool statistics exist only for configured tool names.** A call to any other name counts in the totals and in `lablet.tool_calls.unknown`, so the per-tool attribute keys stay bounded however many names a model invents.
- **`RunContext` holds only what the composition root alone knows.** ProviderResponse mode, the turn cap, the timeout, and the request defaults moved to `RunSummary`, written from the loop's own policy, so the wide event can't report limits the loop didn't obey. `capture_content` stays on the context.
- **The stop policy takes inputs that can't contradict themselves**: a `Progress` before a call and after a tool phase, and the finish reason with a `Calls` value after a response. `Progress` lives in the model because the run produces it.
- **`RetryPolicy` takes `max_retries` directly**, zero included, and `StopPolicy::allows_wait` owns the rule that a backoff wait mustn't carry the run to its timeout.
- **`lablet.provider.retries` counts attempts begun beyond the first of a call**, so one fatal first call reports zero.
- **Messages, completions, and transcripts validate when read from their serde form**, through private raw types, and unknown fields are errors, so a mistyped key in a hand-written script can't read as a default. This supersedes the earlier rule that reading a message doesn't validate it.
- **`RunService::run` returns a `FinishedRun`** holding the summary and the transcript, and holds an optional `Pricing`.
- **The provider name isn't configurable.** The model name with `server.address` and `server.port` on the wide event tells one OpenAI-compatible server from another.

## 2026-09-20 The transcript is made of turns

A second design review, and a proposal from a parallel session, found that the model enforced rules for values by type but left aggregates to convention: the transcript paired a flat message list with a parallel record list, the pushes never validated, and no rule spanned messages. The transcript is now `{ system, turns }`, where a turn holds its input, its response, the record of the completion that produced it, and the outcome of every tool call it made. The pairing is a field rather than an index, so the role rules became unrepresentable and their errors are gone. `messages()` renders the flat form a provider call sends.

- **Each turn carries the user input that prompted it**, which leaves room for multi-turn runs without changing the document. The first turn's input is the task prompt; later inputs are empty today, because they follow a tool phase. The loop still takes one prompt and offers no multi-turn entry point. Input holds text only, so a tool block in user content is unrepresentable rather than validated.
- **Blank input is nothing from the user.** Text that's empty or whitespace only is dropped from input just as from a response, so a blank prompt is refused rather than becoming an empty text block the Anthropic API rejects.
- **The rules that remain** are enforced when a turn is recorded and when a transcript is read: a turn's outcomes answer exactly its response's tool uses, each once and in call order; only the last turn may have tool uses that nothing answered; every turn after the first either has input or follows answered calls; and a turn's calls are answered once.
- **The outcome is typed by `StopReason::class()`.** A failed run always carries an error message, a stopped or completed one never does, and a structured result exists only on a completed run. The outcome JSON is unchanged.
- **A tool call's outcome carries one status**, and `is_error` is derived from it when a tool result is rendered, so the two can't disagree. The status is a closed set declared in the registry as `lablet.tool.status` and pinned to the model by the conformance test, since `error.type` is an open set and a call that ended well hasn't got one.
- **Every turn records when it started, how long the successful attempt took, and how many attempts it took**, so a grader can see the shape of a run without reading telemetry.
- **Tool output is capped by a pure function in the model**, applied by the loop, so every tool is cut the same way at a character boundary with one marker naming both sizes.
- **Port data left the domain.** `McpCallMeta`, `TraceContext`, and `NetworkTransport` are types of the application layer. `Endpoint` stays, because the summary carries it for the wide event.
- **`lablet.provider.calls` is gone**, because turns counts model responses and the two were always equal.
- **Known limit:** a run that receives no response has no turns, so its transcript document doesn't hold the prompt, though the prompt's size still reaches the summary. Revisit when the ATIF export makes the missing user step visible.

## 2026-09-20 One fact, one place, before the loop is written

A review from a parallel session raised twelve modelling findings against the revised domain. Seven landed here, before phase 3, because each one is cheaper to change while the loop that reads these types doesn't yet exist.

- **A tool call's status says whether a tool ran.** `ToolCallStatus` is `Unknown` or `Ran { source, ended }`, so a source exists exactly when a tool ran. Before this, "the model called a tool the run doesn't have" was three facts that could disagree: a missing source, a status, and a name that `finish` looked up a second time in the run's tool list. The summary now reads the outcome's own status for both what it counts as unknown and which calls earn a per-tool entry. `as_str` still flattens to the five `lablet.tool.status` values, so the registry and the conformance test are untouched. The transcript document holds the two levels.
- **`Turn::calls(mode)` is the only way to read a response's tool calls.** The `Calls` type moved from the policy to the model to sit beside `Turn`. The loop can no longer classify a response differently from how the stop policy expects it.
- **The two counted caps are `NonZeroU32`.** `max_turns` and `max_consecutive_tool_errors` both meant one at zero, and each reached that by a different route: a `.max(1)` in one, an unread cap in the other. A zero timeout or token budget still stops a run at the first point A, because there it means something.
- **`FinishReason::Other` carries a private newtype**, so `From<String>` is the only way to build one. `Other("refusal")` used to typecheck, and a refusal that skipped normalisation reads at point R as an ordinary end: the run would have completed, and exited 0, on a response the model declined to give.
- **A cost is finite and never negative**, refused on both the code and the serde path, and `Pricing::cost` answers `Option<Cost>`. JSON has no infinity or NaN, so serde writes either as `null`, which is also how a run with no pricing configured writes the field. An overflowed cost would have been indistinguishable from one never asked for.
- **`RunSummary` and `FinishedRun` serialise and don't deserialise.** They hold the invariants that span fields, such as one finish reason per turn and per-tool keys among the tools, and only `Run::finish` establishes them. They were the last serde path with no rule on the way in; nothing reads either back, so the path is gone rather than guarded.
- **The outcome document refuses a field it doesn't know**, as every other raw type already did. It's the contract a composer parses, so a misspelt key is worth reporting.

Rejected, with the reasoning kept because it will come up again:

- **A typestate for the loop protocol** would delete two error variants and cost more than they do. Whether a response makes tool calls isn't known statically, so the transition would return a sum of two states that both need `finish`, and moving the run through a `loop` fights the borrow checker. `NothingFromTheUser` survives either way, because a whitespace-only prompt is a value, not a state. The serde path keeps both checks regardless.
- **Enforcing that the cache counts are a subset of `input_tokens`** would fail a real run because a provider's numbers didn't add up. Reporting what the provider said, and saturating where the subtraction would go negative, is the better trade for a tool whose job is faithful measurement.

Deferred: newtypes for `config_digest`, `ModelRef::name`, `Endpoint::host`, and an MCP server name; the shared validation of `RunId` and `ToolCallId`, whose provenance is opposite; and the seven `Run*` type names, which want the loop to exist before anything is renamed.

## 2026-09-20 What bounds the per-tool keys, now that the summary doesn't

Making the tool-call status the single authority moved an invariant without moving the sentence that asserted it. A second review caught the drift, and two defects with it.

- **The per-tool attribute keys are bounded by what the tool executor resolves**, not by `RunSetup::tools`. Before, `finish` gated the per-tool entry on the configured name list, which bounded the keys itself. It now reads the outcome's status, which is right, but the bound left with it. Re-adding the name check would reinstate the disagreement that change removed, so the bound is the port's obligation instead: a `ToolExecutor` resolves only names it offered in `specs()`, and `ToolSet` fixes that set when the run is built. Phase 8 is where this bites, because an MCP server may announce `notifications/tools/list_changed` mid-run; the new tool belongs to the next run. Spec §1, §3, and §5 now say so, and §5's `From<ToolErrorKind>` note is corrected: only `Unknown` maps to a status by itself, since the other two need the source of the tool that ran.
- **A refused turn leaves the caller's input alone.** `Transcript::record` promised it and `push` broke it: the blank-input drop ran before the refusal, so an all-blank input came back empty. The check now reads the input without touching it. The effect was benign, since both forms mean nothing from the user, but these comments are contracts and the loop will be written against them. A test holds both refusal paths now.
- **Every total in the summary is added the same way.** `add_turn` mixed plain `+=` with `saturating_add` field by field, against the discipline the rest of the model keeps. One `add` function raises them all.
- **A cost derived from a self-contradicting usage is reported, not withheld.** When a provider's cache counts exceed its own input count, the uncached part saturates to zero and the run prices low. Refusing to price the run is the worse failure for a tool whose job is to report what happened, and all four counts reach the wide event beside the cost, so a consumer can see the inconsistency. Written into `Pricing::cost` rather than left to be rediscovered.

Deferred: building `RunSummary` from an immutable `Totals` value with an `Add` impl, as `Usage` already has, instead of mutating twelve fields through `add_turn`. It's a fair point about where a forgotten field hides, and it isn't order-dependent with the loop.

## 2026-09-20 Four names, changed before the loop is written against them

A naming review from a parallel session ranked nine candidates by whether a name asserts something false rather than merely unhelpful. Four are done here, before phase 3, because one of them gets expensive afterwards and the rest were cheap to fold in. Entries above this one use the new names, so every name in this log is a live one.

- **`Completion` is `ProviderResponse`.** It collided with `CompletionMode` on two central types that mean unrelated things: one is a provider's reply, the chat-completions sense of the word, the other is how a run decides the agent is finished. Renaming the second would have moved the `lablet.run.completion_mode` wire value, so the first moved instead, which also matches the language the spec already used: the response, `Turn::response`, the model's response. `CompletionError` is `ResponseError`, `CompletionRequest` is `ProviderRequest`, and `Run::completion` is `Run::responded`, which reads as the event it records.
- **`RunResult` is `TaskResult`.** It was never the run's result, which is `RunOutcome`. It's `{ text, structured }`, where `structured` is the `task_complete` argument, so it's the task's answer. The field inside the outcome is still `result` and the outcome JSON is unchanged.
- **`RequestDefaults` is `RequestParams`.** Nothing overrides them per call, so they weren't defaults.
- **`RunTally` is `Run`.** It's the aggregate root: it owns the transcript, holds the input waiting for the next turn, and takes each event the loop reports. "Tally" named the least of that. `Run::start`, `run.responded`, `run.finish() -> FinishedRun` is how §1 already described a run in prose. It now lives in `run.rs`, and what was `run.rs` became `outcome.rs`, since it holds how a run ends and what it produces.

Declined, because vocabulary churn has its own cost and neither name asserts anything false: `StopClass`, and `ModelRef`. Deferred: the three nested near-synonyms in `ToolCallOutcome` / `ToolCallStatus` / `ToolCallEnd`, where the field names read well even though the type names repeat; and `RunContext`, which should be renamed if and when it leaves the domain, rather than twice.

## 2026-09-20 A run reports what it spent on reasoning, and at what rates

Reopening phase 1. A prior-art comparison found reasoning tokens missing, and verifying its one unverified claim against Anthropic's API documentation found two more problems beside it. All three are about the same thing: the run invites you to configure reasoning and then can't tell you what it cost.

- **`Usage` carries `reasoning_output_tokens`.** Anthropic reports `usage.output_tokens_details.thinking_tokens` and OpenAI reports `completion_tokens_details.reasoning_tokens`; lablet discarded both. It's a part of `output_tokens`, never an addition to it, on the same subset discipline the cache counts already follow, and `gen_ai.usage.reasoning.output_tokens` was already in the vendored conventions. Pricing needs no new rate, because both providers bill reasoning at the output rate.
- **The two constructors take a named `TokenCounts`.** A fifth positional `u64` beside four others is the hazard that made the two prompts a struct; five adjacent counts deserve the same answer.
- **A run reports the rates it was priced at.** `lablet.pricing.*_usd_per_mtok` sits beside `lablet.run.cost_usd` on the wide event. A property pass showed `cost(a) + cost(b)` differing from `cost(a + b)` when a provider's cache counts exceed its own input count, because the uncached part saturates to zero; a composer could see the inconsistency in the four raw counts but couldn't recompute the cost, since the rates weren't emitted. That was the one derived number lablet published without the inputs to re-derive it, which is against its own raw-data rule. `Rates` is a model type, validated on both paths, because a run reports it; the arithmetic stays in the policy.
- **The thinking setting and the temperature reach the wide event.** `gen_ai.request.temperature` was on the chat span alone while its three siblings were on the event, and the thinking mode reached no attribute at all. A reasoning budget is the setting a run is most often varied by, and one-row analysis couldn't see it.

Recorded as found, to act on at the phase they bite:

- **`Thinking::Budget` is rejected by every current model.** `thinking: {type: "enabled", budget_tokens: N}` is deprecated on the 4.6 generation and returns a 400 on Opus 4.7, Opus 4.8, Opus 5, Sonnet 5, Fable 5.1, Mythos 5.1, Fable 5 and Mythos 5. Adaptive thinking replaced it, with depth set by `output_config.effort`, which is how lablet already models effort. The variant stays for the models that still take it and phase 7 must refuse it for the models that don't.
- **`display` defaults to `omitted`.** On the current generation a thinking block comes back with an empty `thinking` field and a signature identical to the one a summarised block carries. lablet had no way to ask for the text, so every stored reasoning block would be empty and phase 10's ATIF `Step.reasoning_content` always blank. `lablet.request.thinking` records whether the text was asked for; the request field follows at phase 7.

The general lesson is worth more than the three fixes: the domain modelled a provider API from memory, no adapter existed to contradict it, and no gate could. Phase 7 would have found all of it. Checking the documentation cost one page read.

## 2026-09-20 Three seams settled before the loop is written against them

- **The retry policy owns the whole retry rule.** `RetryPolicy::next(attempt, kind)` answers `None` at once for a failure another attempt can't change, where `delay(attempt)` measured a wait and left the loop deciding whether to use it. `ProviderErrorKind` is a payload-free domain enum beside the application's `ProviderError`, which keeps the message; that mirrors what tools already do, where `ToolErrorKind` maps onto a domain value the conformance test pins. The four spellings are the failed chat span's `error.type`; that attribute is an open set in the conventions, so a unit test pins them rather than a generated enum.
- **Tool arguments that aren't JSON are the model's problem to fix, not the provider's.** They were classified as a malformed response and retried, which re-rolls the same prompt against the same schema, burns the retry budget, ends the run `retries_exhausted`, and reports it in `lablet.provider.retries`. A model that reliably fails to serialise against an awkward MCP schema is exactly the signal "optimise this MCP server" exists to surface, and it pointed at the wrong component. `ToolInput` keeps what the model wrote, `ToolCallStatus::MalformedInput` is the outcome, and the call counts in the tool statistics. Three other frameworks landed here independently. The counter-argument is on record: a response the adapter can't map is a genuine provider failure that deserves a retry; what changed is that arguments never were that case.
- **A run takes one `Prompts`, not two strings.** `start(setup, system, prompt)` took them adjacent and swapping them ran the task as the system prompt with both reported sizes inverted. One value with named fields, for the reason five token counts became a `TokenCounts`.
