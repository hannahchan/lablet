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

**Partly superseded on 2026-09-20 by "A second document in `contributing/`":** `contributing/` holds two, `README.md` and `reviews.md`. The three areas, the four product files, and the ban on an ADR folder, templates, and metadata headers all hold.

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

**Partly superseded on 2026-09-20 by "Phase 2 decisions made by the builder":** `run.max_retries` counts retries, not attempts, so `3` allows four attempts and `0` never retries. The budget being per call, and resetting on success, still holds.

`run.max_retries` counts attempts for one call and resets on success. A per-run budget made a long run lose to a few spread-out rate limits, which measures the provider's weather rather than the agent.

## 2026-09-18 MCP tool names are never prefixed by default

Tools keep the names their server reports so measurements reflect the server as-is and allow and deny lists stay stable when servers are added. A collision is a build error; `prefix_tools: true` on a server is the escape hatch.

**Superseded on 2026-09-28 by "Three of the parity matrix's proposals, decided":** an MCP tool's name carries its server's by default, and a server may keep its own names.

## 2026-09-18 Stop policy has two evaluation points

**Superseded on 2026-09-20 by "Phase 2 decisions made by the builder":** there are three points, not two. A response is judged before its tool calls run, which is what lets a refusal or a truncated response stop the run without executing anything.

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
- **CI runs on every pushed branch** as a matrix of `ci`, `coverage`, and `mutants`, since there are no pull requests. On `main` the changelog base is the commit before the push; on branches it's the merge-base with `origin/main`. **Partly superseded on 2026-09-29 by "Mutation testing runs daily, and its floor is exact":** the matrix is `ci` and `coverage`, and `mutants` runs in the `Floors` workflow, daily and on demand. Running on every pushed branch and the changelog bases still hold.
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

- **Three stop points, not two.** The loop decides before a provider call, after a provider response, and after a tool phase. Each point has a documented precedence, so one state always gives one answer. The tool-error cap leads after a tool phase because it alone says the run was failing, not merely long. A response that finishes the task completes the run even when it also used up a limit. _Superseded in part on 2026-09-28: the cap that leads after a tool phase counts consecutive invalid turns, not tool errors._
- **A truncated `task_complete` is `output_truncated`, not `completed`.** A response cut off at `max_tokens` can end inside the call's arguments, and a run mustn't end on a partial result. This came from the phase review.
- **Every limit is met when the run reaches it.** `max_turns: 2` stops after turn 2's tool phase, elapsed equal to the timeout stops, and tokens equal to the budget stops.
- **`run.max_retries` counts retries, not attempts.** `3` allows four attempts and `0` never retries. The first build read it as attempts because of how scenario E2 was worded; the name now means what it says, and E2 is four errors.
- **Backoff is `base * 2^(n - 1)` capped at `max`, with no jitter**, from `run.retry_backoff_base` (500ms) and `run.retry_backoff_max` (30s). The loop checks the run timeout before each wait.
- **Durations are whole milliseconds in `u64` fields named `*_ms`.** ~~The loop converts once where it reads the clock~~, superseded later the same day by the domain revision, where the model converts as it records and holds the only `whole_ms`. The rule that no observer rounds for itself still holds, and is why the conversion has one home.
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
- **Per-tool statistics exist only for configured tool names.** A call to any other name counts in the totals and in `lablet.tool_calls.unknown`, so the per-tool attribute keys stay bounded however many names a model invents. **Superseded on 2026-09-20 by "What bounds the per-tool keys, now that the summary doesn't":** the bound is what the tool executor resolved, not the configured name list, and the two coincide only because the executor resolves nothing it didn't offer.
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
- **Known limit:** a run that receives no response has no turns, so its transcript document doesn't hold the prompt, though the prompt's size still reaches the summary. Revisit when the ATIF export makes the missing user step visible. **Superseded on 2026-09-30 by "Six decisions before phase 5":** phase 5 puts the task prompt in such a run's transcript, since a CLI run shows the gap before the ATIF export does.

## 2026-09-20 One fact, one place, before the loop is written

A review from a parallel session raised twelve modelling findings against the revised domain. Seven landed here, before phase 3, because each one is cheaper to change while the loop that reads these types doesn't yet exist.

- **A tool call's status says whether a tool ran.** `ToolCallStatus` is `Unknown` or `Ran { source, ended }`, so a source exists exactly when a tool ran. Before this, "the model called a tool the run doesn't have" was three facts that could disagree: a missing source, a status, and a name that `finish` looked up a second time in the run's tool list. The summary now reads the outcome's own status for both what it counts as unknown and which calls earn a per-tool entry. `as_str` still flattens to the five `lablet.tool.status` values, so the registry and the conformance test are untouched. The transcript document holds the two levels.
- **`Turn::calls(mode)` is the only way to read a response's tool calls.** The `Calls` type moved from the policy to the model to sit beside `Turn`. The loop can no longer classify a response differently from how the stop policy expects it.
- **The two counted caps are `NonZeroU32`.** `max_turns` and `max_consecutive_tool_errors` both meant one at zero, and each reached that by a different route: a `.max(1)` in one, an unread cap in the other. A zero timeout or token budget still stops a run at the first point A, because there it means something. _Superseded on 2026-09-28: the turn cap is optional, and the cap on tool errors gave way to a cap on consecutive invalid turns._
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

## 2026-09-20 The small calls, and what phase 3 inherits

Decisions taken rather than deferred again, plus the constraints phase 3 should build on rather than rediscover.

- **`gen_ai.request.seed` is an `i64` in the model too.** It was an `Option<u64>`, and a seed above `i64::MAX` would have reached telemetry as some other number. Unlike the token counts it isn't bounded in practice, so the domain now holds the type the wire can carry.
- **`lablet.run.transcript_path` is the path rendered as UTF-8**, replacing anything that isn't. A path lablet chose from a config a person wrote is UTF-8 in practice, and refusing a run over the rendering of a telemetry attribute would be the wrong trade.
- **No string newtypes yet.** `config_digest`, `ModelRef::name`, `Endpoint::host` and an MCP server name stay `String`. `config_digest` has exactly one producer, in the composition root, which phase 6 writes; the others are pass-through values whose shape lablet doesn't get to decide. Revisit `ConfigDigest` when that producer exists. **Partly superseded on 2026-09-30 by "Six decisions before phase 5":** phase 4 wrote the producer, and `ConfigDigest` becomes a newtype in phase 5, in the change that moves the digest. The other three still stay `String`.
- **No further renames.** The four that landed were the names asserting something false. `StopClass`, `ModelRef`, the remaining `Run*` types and the nested `ToolCall*` types don't, and vocabulary churn has its own cost.
- **The fake provider emits `gen_ai.provider.name: fake`.** The attribute is a closed member list of sixteen real providers in the conventions and `fake` isn't one, yet the fake-provider config is the only thing `weaver live-check` runs against. The alternative is reporting some other provider's name for a run no provider served, which would be a lie in the one field that says who answered. Phase 6 adds a documented finding filter for it if live-check grades an undeclared member as a violation rather than an advisory; that's the phase where it can first be observed.
- **Room left for two additive changes, not built.** `Thinking { text, signature }` has one replay slot where OpenAI's Responses API needs two, the reasoning item id and its encrypted content, and it carries no provider tag though `Opaque` does; the error about a reasoning item provided without its required following item recurs across four SDKs' issue trackers. And `ToolSource` can't express a provider-native server-side tool, so Anthropic's web search would land in `Opaque` and count as zero tool calls, which makes "does their search beat my MCP server" unanswerable. Both are additive and belong to phases 9 and 8.

Validated rather than changed, because it turned out to be load-bearing:

- **lablet already satisfies Anthropic's preserved-thinking prefix binding.** A thinking block's signature is bound to the system prompt, the tools array and every preceding message, and editing any earlier message invalidates it. The system prompt and the tool list are fixed for the run, the transcript is append-only, and the blank-text drop happens once before storage and is replayed identically on every request, so two consecutive requests never differ in their prefix. The decision that a tool appearing mid-run belongs to the next run is what keeps the tools array fixed, which is why it's recorded here as a constraint rather than a convenience.

Phase 3 inherits two things worth stating before it starts:

- **`ToolSet` should make the executor's obligation structural.** The per-tool attribute keys are bounded by what the executor resolves, which is a contract an adapter has to honour. If resolving a name through the tool set is the only way to obtain a `ToolSource`, a tool appearing mid-run can't produce a `Ran` outcome at all, and the bound stops depending on behaviour.
- **`lablet-run`'s floors measure nothing until `RunService` exists.** A first commit of port traits alone reports "nothing to measure yet" and passes both floors while asserting nothing, so the first commit there carries enough of the loop to be measured.

## 2026-09-20 Six laws, because examples can't state them

`proptest` joins the workspace as a dev-dependency of the two domain crates, with default features off so the fork-based runner and its process-isolation trees don't come with it; lablet's properties are pure.

Every test until now was an example. Examples say what one value does; these say what every value does, which is what a composer summing ten thousand runs actually relies on. The laws:

- **Cost is additive over usages a provider could report.** Summing each run's cost gives the same answer as pricing the summed usage. This is the guarantee the product's stated purpose depends on, and it only holds when the cache counts are a part of the input count, which is why the rates now reach the wide event: a consumer can tell when a provider's arithmetic didn't agree with itself.
- **Cost never falls as tokens rise.**
- **Summing usage is a commutative monoid.** A run's totals don't depend on the order its turns are added in, and an empty run adds nothing. Saturating addition stays associative because both groupings reach the same ceiling, which was worth checking rather than assuming.
- **Neither total double-counts the part it holds.**
- **Normalising a finish reason is idempotent**, so a reason that round-trips through a document can't drift.
- **A transcript read back is the transcript that was written.** Asserted on a transcript the model built, because blank text is dropped once on the way in, so an arbitrary document normalises on its first read rather than being a fixpoint immediately.

## 2026-09-20 The summary's totals are grouped at phase 4, with the mapping that needs them

The review raised three findings against `RunSummary`, which are one shape seen three ways: twelve bare `u64` fields, `finish` initialising twenty zeros and mutating them, and a 22-field struct. Grouping the totals into value types with `Add` impls, mirroring `Usage`, was going to land before phase 3.

It moves to phase 4 instead, paired with the change that actually prevents the bug.

The risk is the wide-event mapping: twenty assignments between same-typed values, where the compiler sees one type, the registry declares every attribute `int`, and the summary's JSON test checks how the summary serialises rather than how it reaches telemetry. Grouping the fields makes that mapping read better and doesn't make a swap impossible; only holding the mapping to the generated key list does. So the two land together, and the mapping gets written first, because it shows which groups it wants rather than leaving the grouping to a guess.

Two things argued against doing the grouping alone and early. The wide event is flat by rule, since the aggregatability rules forbid nested maps, so a flat summary mirrors what it becomes and nesting it only to flatten it again adds a step that can itself be wrong. And the forgotten-field risk in `finish` is covered today: every field is asserted, so one left unwritten fails now. The exposure is to fields added later, which is worth fixing and isn't urgent.

Newtypes for the units instead, `Bytes`, `Millis` and `Count`, were considered and declined. It targets the swap directly and would hold across the whole model, but it touches every arithmetic site and buys nothing at the telemetry boundary, where every attribute is an `int` whatever the domain called it.

## 2026-09-20 One module per idea, not one per stage of a run

Two independent module-organisation reviews, run without contact, reached the same shortlist: `provider.rs` and `outcome.rs` had become catch-alls, and `whole_ms` was in the wrong file. The second named the cause the first only described: the modules were named along two axes at once, some for a kind of thing (`id`, `message`, `tool`, `provider`) and some for a stage of a run (`run`, `transcript`, `outcome`), so anything fitting neither landed in the nearest stage-module.

`lablet-model` is now eleven modules with one idea each. `price.rs` takes `Cost` and `Rates`, which were in `provider.rs` though they're neither what a run asks a provider nor what one answers; the giveaway was that the request parameters had been pushed below them, so the file's declaration order contradicted its own doc. `usage.rs` takes `Usage` and `TokenCounts`, which every other module reads. `stop.rs` takes the vocabulary of how a run ends, `CompletionMode`, `StopReason`, `StopClass` and `Calls`; `outcome.rs` keeps the document; `summary.rs` takes the two halves of the wide event. `whole_ms` moved to `lib.rs`, so `tool.rs` no longer says `use crate::outcome::whole_ms` and implies that tools depend on outcomes.

Three things moved to sit beside what they belong to. `add_turn` is now `RunSummary::add_turn`, next to the doc claiming `Run::finish` establishes the summary's invariants rather than a file away from it. The output cap, `ToolResultContent::capped`, moved from `message.rs` to `tool.rs`, because a cap is a property of a tool call and its only caller was there. And `Pricing::new` takes a `Rates` and is infallible: it had been re-declaring `Rates::new`'s four-`f64` signature and its `# Errors` section only to call it, and had no caller outside its own tests.

This dissolves both import cycles the second review found: `message` and `provider` each imported the other, as did `transcript` and `outcome`. Legal in Rust, and the usual sign of a boundary in the wrong place.

Declined: splitting `message.rs` into owned storage and the borrowed wire view. `Message<'a>` borrows from the owned types, so a module line between them would make the lifetime harder to follow, not easier. Also declined, for now: splitting the three largest test files, which is navigability alone.

The timing is the same argument that made moving `Calls` free: pure code movement is cheap while `lablet-run` doesn't exist and expensive once the loop is written against these paths.

**Partly superseded on 2026-09-20 by "`ProviderKind` is a leaf module, which the cycle claim needed":** the refactor dissolved one of the two import cycles, not both. Everything else in this entry holds.

## 2026-09-20 Phase 3 decisions made by the builder

- **Superseded decisions are now marked where they sit.** Four entries said something a later entry reversed, with nothing at the old entry to say so: the stop policy's two evaluation points became three, `max_retries` changed from counting attempts to counting retries, the duration conversion moved from the loop to the model, and the per-tool bound moved from the configured name list to what the executor resolved. This log is used to refuse review findings that have already been decided, so an entry that reads as current when it isn't can mislead in the one direction that matters.
- **`RunEvent` carries the run id once, not on every kind.** The spec's sketch said every variant carries `run_id` and gave it to none. It's `RunEvent { run_id, kind }`, because the id is a join key an observer needs on every signal, and one field is one place to read it rather than eight match arms.
- **`RunService::run` takes `&mut self`.** The spec says one service runs one run at a time and that concurrent calls are rejected, but the application ring is forbidden tokio by the layer lint, so there's no async mutex, and `run` returns a bare `FinishedRun` with nowhere to report a refusal. An exclusive borrow makes a concurrent call a compile error rather than a runtime one, which is the answer this domain gives everywhere else. Sequential runs, which is what the phase 4 library offers, are unaffected.
- **`ProviderError` is a struct, not an enum with a message in each variant.** `ToolError` was already a struct with a `kind`, and `ProviderErrorKind` exists so the retry policy can read the class; one shape for both errors means the policy takes one field rather than matching four variants.
- **`ToolSet` is built whole in this phase, and its filtering is unit-tested here.** The build plan's phase 3 line asks for allow and deny filtering and duplicate-name rejection, but the scenarios that exercise those are assigned to phases 4 and 8, where real executors exist. Half a type is worse than none, so it's built and unit-tested now; the named scenarios land where acceptance.md puts them.

## 2026-09-20 A second document in `contributing/`

Amends the 2026-09-17 entry "Documentation is three areas, four product files," which said `contributing/` holds one conventions document. It now holds two: `README.md` for the rules, and `reviews.md` for what to look for in a review.

The split is along the line that already ran through the page. Every rule in `README.md` is enforced by a gate or labelled a review convention, and the Reviews section described how a review is staffed and budgeted but never what it should find. That knowledge existed only in commit messages and in this log, where it was recoverable but not reachable: a reviewer would have had to read fifteen commits to assemble it.

`reviews.md` assembles it, each item drawn from a defect that reached `main` in the two domain crates and was caught by a later review, with the fixing commit cited so the full reasoning stays recoverable. Nothing in it's a gate, and an item that becomes mechanical moves to `README.md` and becomes one.

This isn't an ADR folder, templates, or metadata headers, which the original entry ruled out and which stay ruled out.

## 2026-09-20 `ProviderKind` is a leaf module, which the cycle claim needed

Corrects the entry above, "One module per idea, not one per stage of a run," which says the refactor dissolved both import cycles in `lablet-model`. It dissolved one. `transcript` and `outcome` no longer import each other, and `message` and `provider` still did: `message` needs `ProviderKind` for `ContentBlock::Opaque`, and `provider` needs `ContentBlock` for a response.

`ProviderKind` moves to its own module, which breaks the cycle at the only edge that could move. `provider` depends on `message` for content whatever happens, so the provider family was the half to go, and it goes down rather than sideways: a leaf that names which provider serves a run, imported by the content that tags an opaque block and by the `ModelRef` that names a model. `Usage` set the pattern one commit earlier, when it left `provider.rs` while `ProviderResponse` went on holding one.

`lablet-model` is twelve modules, and its module graph has no cycle.

## 2026-09-21 Phase 3, finished

- **`RunService` holds a `ToolSet`, not an `Arc<dyn ToolExecutor>`.** The loop has to learn where a tool came from, and the port can't say: `specs()` lists them but nothing maps a name to its source. Holding the composite concretely is what makes the executor's obligation structural, since `ToolSet::source` answers from a map fixed when the run was built.
- **Which stop reason a provider failure becomes lives in the loop.** It depends on context the policy can't see: a retryable or malformed failure is `retries_exhausted` only once the budget is spent, where a fatal one is `provider_error` at once and a context-exhausted one is itself. `ProviderErrorKind` stays a domain type; the mapping doesn't.
- **A call whose arguments didn't parse never reaches an executor.** The loop resolves the name first, so a name the run doesn't offer is `unknown` whether or not its arguments parsed, and only a real name with bad arguments is `malformed_input`. No `ToolCall` is built for one, so it has no latency of its own.
- **The fakes live under `src/tests/`.** The gate forbids `#[cfg(test)]` on anything but `mod tests;`, and coverage tells test code from production by filename, so a `src/fakes.rs` would have been measured as production and failed the attribute check. One `#[cfg(test)] mod tests;` in `lib.rs`, everything else below it.
- **`ToolName::task_complete()` is infallible.** An error path for a name that can't be refused would be an unreachable branch, and an unreachable branch can't be covered, so the floor would have refused it. The floors shaped the design rather than only checking it.

Known limit, carried forward: one mutant survives in `lablet-run`, replacing the default `RunObserver::trace_context` with `None`. The default body is already `None`, so it's an equivalent mutant and no test can kill it.

## 2026-09-21 What the phase 3 review changed

Eleven findings survived verification, three of them one defect found by three reviewers. The four that changed a decision rather than a line:

- **A run keeps one `CompletionMode`, on the `ToolSet`.** It was on `StopPolicy` as well, and `RunService::new` takes the built set and the policy as independent arguments, so a set built `Natural` beside a policy set `Explicit` produced a run that reported `completion_mode: explicit`, never offered `task_complete`, and ended `ended_without_completion` on turn 1 with nothing to diagnose it by. The set is the holder because it's the one value whose shape depends on the mode: a set built for the wrong mode is a set with the wrong tools in it. `StopPolicy::after_response` takes the mode as an argument, like every other thing it's handed.
- **A blank task prompt is refused by `Prompts::new`.** `Prompts` was two public `String`s, and the transcript refuses a first turn whose input is blank, but only after the provider call. So an empty task bought a real call, threw the answer away, and reported `turns: 0`, `usage: 0` and `cost: null` for tokens that were spent, while blaming lablet for a defect of its own. Private fields and a checked constructor put the refusal where it costs nothing. An empty system prompt is still a run with no system prompt.
- **`EventKind::ProviderCallFailed` carries one `retry: Option<Duration>`.** It was `will_retry: bool` beside `backoff: Option<Duration>`, one fact stored twice, so `{ will_retry: true, backoff: None }` typechecked, and `wait.unwrap_or_default()` turned the state that can't happen into a reported backoff of zero. `lablet.retry.will_retry` and `lablet.retry.backoff_ms` are both read from the one field, so the registry is unchanged.
- **`task_complete` is claimed before any executor is asked.** The duplicate check guarded executors against each other and the built-in spec was pushed afterwards, so an executor serving `task_complete` in explicit mode got the name offered twice. Every provider rejects that with a 400, so every call of the run would have failed, with the cause nowhere in the telemetry, and the executor's own tool was unreachable behind the interception. It's now `ToolSetError::DuplicateName`, which is what that error exists for.

Also fixed, without a decision to record: every successful provider call was recorded with zero latency and a start offset equal to the instant the response arrived, so `lablet.provider.latency_ms.total` counted only failed attempts and a clean run published zero for it; and the loop dropped every executor's `McpCallMeta` at `settle`, which left six declared `mcp.*` and `jsonrpc.*` span attributes with no path to being set. Both are now held by tests.

## 2026-09-21 `RunService::run` keeps `&mut self`

Signed off by the human after the phase 3 review raised it. Two runs on one service is a compile error rather than a race. The spec asks for a concurrent call to be rejected, and this ring has no runtime to reject one at: `run` returns a bare `FinishedRun` with nowhere to report a refusal, so an exclusive borrow is the same answer the domain gives everywhere else.

The cost lands in phase 4, where `Lablet` runs more than once: it holds the service mutably, or builds one per run. Neither is a race.

## 2026-09-21 Two phase 3 sign-offs, and what checking them found

Both decided by the human; the first needed the build plan's phase 3 acceptance prose narrowed, so verifying it was worth doing properly.

- **The build plan's phase 3 prose now matches `acceptance.md`.** It asked for allow and deny filtering and duplicate-name rejection end to end, where acceptance.md puts those scenarios (T1, T2, T3) in phases 4 and 8, which is where real executors exist. `ToolSet` is built and unit-tested whole in phase 3, as the 2026-09-21 entry above says; the prose now says that rather than claiming the scenarios.
- **L9's exit code belongs to phase 5.** Spec §2 makes exit 2 the CLI's mapping of any stop reason but `completed`. Phase 3 has no CLI, so it asserts the stop reason and that it isn't `completed`; the code itself is asserted where `main.rs` exists.

Checking the claim scenario by scenario found nine of the twenty-one phase 3 scenarios under-asserted: the loop behaved correctly in every case, but fifteen clauses of `acceptance.md` had no assertion that would fail if the behaviour broke. The pattern was consistent: a test asserted the stop reason and stopped there, where the scenario also asks for no further provider call, no `ToolCallStarted`, or the wide event written all the same. Two were load-bearing: nothing asserted that a cancelled run's in-flight tool call runs to its end and comes back on the transcript, and nothing asserted that a tool call outcome holds its own start offset, which is the same timing chain the zero-latency defect broke. All fifteen are now asserted.

A stop reason is the cheapest thing to assert and the least of what a scenario says. That's the review item: assert the clause, not the outcome it implies.

## 2026-09-21 Coverage judges regions as well as lines

`cargo xtask coverage` held each floor crate to a line floor alone, though cargo-llvm-cov reports regions in the same JSON and the gate parsed them away. A region is a span of source with its own counter, so each match arm, each `else` a line never spells out, and the far side of a `&&` are counted apart, where a line counts as covered when any region touching it ran. Regions are therefore the stricter measure and 100% regions implies 100% lines; the gap between the two is where a line hides an arm nothing took.

The floor is 90%, the same as lines, because the number is what the existing floors already commit to, and raising it needs a decision of its own. Each crate now gets two report lines, lines then regions, so the gap reads off adjacent rows. `conclude` counts floors rather than crates, since a crate is now held to more than one.

Branch coverage was considered and left out. It's finer again in the other direction, because a condition that's never false has no uncovered region and only branch coverage sees it, but `-Z coverage-options=branch` needs nightly and `rust-toolchain.toml` pins stable 1.98.1. Measured on nightly before this change, it found nothing region coverage doesn't already flag: `lablet-policy` is 18 of 18 branches, and `lablet-run`'s two missed branches are both also missed regions. That's a fact about today's code rather than a property, and it's worth measuring again when the loop grows.

**Partly superseded on 2026-09-29 by "Mutation testing runs daily, and its floor is exact":** branch coverage is measured on the nightly that `xtask/src/coverage.rs` pins and held to 100% in the daily `Floors` run. That only a nightly toolchain measures branches, and that the workspace stays on its pinned stable one, still hold.

What stands between the domain and application crates and 100% regions, as of this commit, is one testable gap and a set of defensive branches:

- `transcript.rs`, the `else` of `if let Some(turn) = self.turns.last_mut()`, never taken in 1,680 calls.
- `service.rs`, `Stopped::defect` and both its call sites: the transcript refusing a sequence the loop can't produce.
- `service.rs`, the `unreachable!` in `settle`'s `let ... else`.
- `service.rs`, `ToolInput::Unparsed` in `task_complete_argument`, and `tool.rs`, `ToolErrorKind::Unknown` in `ended`. The first is reachable and untested; the second can't be reached through `settle`, which resolves the name before it calls an executor.

All but one are a branch for a state the caller has already excluded, which is the reading the human gave this gate: coverage falling in these layers is a design signal. Raising the floor to 100% would mean making those states unrepresentable rather than guarded, and is left as its own decision.

## 2026-09-21 The domain crates are held to every line and every region

A trial, asked for after the region floor landed: remove the branches nothing can reach, then raise the floors to 100 and see what holds. `lablet-model` and `lablet-policy` now hold at 100% lines and 100% regions. `lablet-run` reached 98.4% regions and is held at 98: a ratchet just under where it stands rather than a round number, since it can't reach 100. The margin is two regions, so anything new that no test can reach fails the gate, where a floor at 90 would have left eight points of silent drift.

Eighteen uncovered regions went in, seven of them design and four of them tests nobody had written:

- `Transcript::answer` asked for the last turn twice, once to check and once to write, and the second ask couldn't fail. It takes the turn once now, mutably, and decides every refusal against it. The `None` arm that's left is reachable, because outcomes can arrive with no turn to answer, and it's tested.
- `RunService::settle` took the call's arguments as an `Option` its caller had already matched out of `ToolInput`, so it had to unwrap what couldn't be absent and guard the rest with `unreachable!`. It reads `call.input` itself now and the arm is gone.
- No scenario ran a tool call with `capture_content` on. Line coverage couldn't see it: the fields were on covered lines and only the closures behind `capture.then(..)` were cold. Two of the loop's four content fields had never once been exercised.
- Nothing called `ToolErrorKind::Unknown.ended()`, or completed a run with a `task_complete` call whose arguments didn't parse.

The eleven that are left are the loop's two `Stopped::defect` arms and the function they call: `Run::responded` and `Run::tool_calls` returning a `TranscriptError` the loop can't provoke, because the policy stops a turn that called no tool before the next response is recorded.

Those stay, and the reason is worth stating. `Transcript` is read back with `#[serde(try_from = "RawTranscript")]`, and that conversion walks the raw turns through the same `push` and `answer` the run writes through. One rule set, enforced on both paths, which is the convention this repo already holds itself to. Making the loop's calls infallible would mean a second copy of those rules for the read path, and a second copy is the drift the convention exists to prevent. Eleven uncovered regions is the cheaper side of that trade.

So the hypothesis behind the floor holds, with a boundary: coverage falling in these layers is a design signal, except where the uncovered code is one caller's handling of a failure another caller can genuinely cause. That case is a shared contract, not a guard.

No floor is 100% mutants. An equivalent mutant can't be killed by any test, `lablet-run` carries one already (the default `RunObserver::trace_context` returning `None`, replaced by `None`), and whether a mutant is equivalent isn't decidable, so the number would be a promise about future code that nobody can keep.

**Partly superseded on 2026-09-29 by "Mutation testing runs daily, and its floor is exact":** the mutation floor is exact. Every viable mutant is caught except the ones a list names, each with its reason, and this equivalent mutant is the list's one entry. That whether a mutant is equivalent isn't decidable still holds, and it's why a person judges each one once and the gate doesn't.

## 2026-09-21 Vendor and solution agnostic, written down

A commitment the human has been holding since before the spec and never wrote: no model provider, eval framework, trajectory format, or observability product is privileged. It's in `brief.md` now, beside "raw data, never reports," because it has been deciding things unwritten and nothing in the repo let a reader reconstruct it.

It surfaced while weighing whether `Transcript` should carry its own serde form, and it turned that question around. The argument for the split had been the domain's freedom to change. The argument that actually holds is this one: the transcript's serde form is lablet's native JSON, so the domain type **is** one of the formats, and being one of the formats is what a neutral model can't be.

Two pieces of evidence, both already in the repo:

- Phase 2's line in `build-plan.md` obliged the model to ATIF: "The transcript shape must map losslessly onto an ATIF v1.8 trajectory ... so the phase 10 export needs no model change." That's Harbor's format setting the domain's shape, as a standing requirement.
- It doesn't hold anyway. Spec §3's own mapping table sends the outcome's `status`, `started_ms`, `latency_ms` and `truncated_from_bytes` to `ObservationResult.extra`, because ATIF has no slot for an error, a time, or a truncation. The escape hatch was there before anyone claimed field-for-field.

So the model is shaped by format two and is format one, and neither is the run.

What follows, and what doesn't:

- **The obligation on the model is lifted.** `build-plan.md` is amended in this commit: the transcript maps onto ATIF, which is why phase 10 exports rather than reconstructs, but the shape is the run's and the export carries what ATIF has no slot for. Nothing built in phase 2 changes; what changes is which way the constraint points.
- **Giving the transcript its own document type isn't decided here.** It's the open question this commitment gives a reason to ask, and it needs its own go-ahead: it would delete the transcript's read path, which nothing uses, and supersede the 2026-09-21 entry that priced eleven uncovered regions as the cost of one rule set serving two paths. The read path has no second caller, so that price was paid for nobody.
- **The port isn't the answer**, and the distinction is worth keeping. One kind of thing maps the run to a format; a port is a trait the application ring depends on. The loop neither renders nor writes, and a failed disk write isn't a stop reason. A format writer is an adapter the composition root picks between; `run.transcript_format` already names two.
- **Spec §3's "field for field" wants revisiting** when the transcript question is settled, since the table below it already says otherwise.

This doesn't reopen the provider adapters or the telemetry conventions, which were agnostic by construction: provider wire formats are separate types in their own adapters, and OTLP reaches any collector.

## 2026-09-21 The transcript has no read path

Follows the vendor-agnostic commitment above, and supersedes the 2026-09-21 entry "The domain crates are held to every line and every region" on the one point it got wrong.

That entry priced eleven uncovered regions in `lablet-run` as the cost of one rule set serving two paths, the loop's and the serde reader's. The premise was checked and the conclusion wasn't: the reader has no caller. Nothing in lablet reads a `Transcript` back, and `brief.md` puts the consuming side outside lablet twice over, in "analysis belongs to the larger framework that composes lablets," and in "writing Arrow or Parquet directly: the collector side does that." A reader in `lablet-model` was lablet doing the composing framework's job. It's gone, along with `RawTranscript`, `RawTurn`, the `try_from` attribute, and `TranscriptError::Response`, whose only producer was that conversion. "Reading back what it emitted" is now in the brief's out-of-scope list so the question doesn't come back.

`TranscriptDocument` takes the published form's place. It borrows a `Transcript`, adds `schema_version`, and serialises without reading back, which is the posture `RunSummary` and `FinishedRun` already had. The version had nowhere to live while the domain type was the wire form, since a transcript in memory has no version, and it had to land now rather than later: `RawTranscript` carried `deny_unknown_fields`, so a version key added afterwards would have been a break lablet's own reader enforced against lablet's own older documents. `TranscriptDocument::of` takes `Transcript` apart by pattern rather than through its reader methods, so a field added to the run's transcript is a compile error until this says whether it's published. That's why the module is a child of `transcript` and not a sibling.

The eleven regions stay, and they needed a new justification because the old one is now false. They're a third kind of uncovered code, and the earlier entry's two categories don't hold them:

- A guard against a state the caller has already ruled out should go, and the type should rule it out instead.
- A shared contract, where a second caller can genuinely fail the check, stays, and that caller's tests cover it.
- These are neither. No caller can reach them. What they buy is that a defect in the loop ends the run with a reported `provider_error` instead of publishing a transcript that lies about what happened. For a tool whose whole value is being trustworthy about numbers, never silently wrong is worth more than two points of coverage.

So `lablet-run` stays at 98 and the floor's recorded reason is rewritten rather than the number moved. `lablet-model` is unchanged at 100% lines and regions: deleting the reader took its tests with it, and the document arrived with its own.

Not done here, and worth naming so it isn't lost. `RunFinished` is emitted inside `RunService::run` while `Lablet::run` writes the file afterwards, so `lablet.run.transcript_path` is published before the file exists, and whether or not the write succeeds. Phase 4 either writes before the wide event or makes the attribute conditional. `TranscriptError::AlreadyAnswered` is also unreachable from production on what's now the only path, and its only producer is a unit test.

## 2026-09-21 The domain is independent of the report, and serde isn't the axis

Restores the 2026-09-17 entry "`serde_json::Value` allowed in domain," whose second sentence read: serde derives stay out of domain and application, and wire forms belong to adapters and the composition root. It was superseded the next day and it was right. It's restored with a scope, and the 2026-09-18 entry "serde derives allowed in the domain" keeps its first sentence and loses its conclusion.

The axis was wrong for four days, in both entries and in the arguing about them. serde isn't a format: one `Serialize` impl serves JSON, MessagePack, CBOR and anything else, so a derive in the domain is no more a JSON dependency than `Display` is a terminal dependency. And `serde_json::Value` genuinely belongs in the domain, for the reason the 2026-09-17 entry gave and which still holds: tool arguments, a tool's input schema, an opaque provider block, and the `task_complete` argument are JSON because the provider APIs define them that way. Modelling them as a JSON value is the domain being accurate about the world. Replacing `Value` with a hand-rolled enum of the same shape moves a dependency and keeps the coupling, pays a recursive conversion in every adapter on every call, and puts number precision and object ordering at risk, which the preserved-thinking prefix binding recorded on 2026-09-21 depends on.

What a derive on a domain type actually costs is the shape. Field names, nesting and enum representation become the published form, in every format rendered from that type. So the live problem was never that the domain depends on JSON, which is false, but that the domain isn't independent of the report, which is true and is what the vendor-agnostic commitment above already asks for.

The proof is already in the tree. `TranscriptDocument` exists because a `schema_version` belongs to the document and means nothing to a run in memory, so the moment the published shape had to differ from the internal one, a separate type was forced. It didn't go far enough: it lives in `lablet-model` and nests `Turn` directly, so the leaves still publish their field names from the domain ring.

Three surfaces carry the domain's shape, which is the count the 2026-09-18 entry named and got right. What it got wrong was concluding the domain should therefore own the shape, and the reason it gave, three hand-written mirrors of the same types, is dissolved by a shared adapter kernel: `crates/adapters/secondary/shared/` already exists and already holds `telemetry-registry`, so one document crate serves sibling adapters without either depending on the other.

- The transcript document, written out. Unwritten, since `apps/lablet` is two lines of module doc.
- The fake provider's script, read in. Unwritten too, and spec section 6 calls it the domain model's serde form.
- The outcome document, written out. Landed, and the only one that's governed: a checked-in fixture and a changelog gate.

Two of the three are free, so phase 4 takes them and the build plan is amended in this commit. Document shapes live in `lablet-documents` under the shared adapter ring; `Transcript`, `Turn` and the leaves they nest lose their derives; the transcript writer is an adapter and the write stays in `Lablet::run`, where effects belong.

The script keeps its validating read, and the asymmetry is the point rather than an inconsistency. The transcript's reader was deleted yesterday because nothing reads a transcript. A human hand-writes a script, which is a writer nothing vouches for, so the 2026-09-20 rule stands there: a mistyped key that read as a default would be a script that lies about what the model said.

Not decided, and deliberately left open: the outcome document. It's the most report-like of the three, since section 2 makes it what a run prints, so the principle bites hardest there. It's also the only one already landed, and a fixture and a gate already protect it from silent drift. The move is bounded work, around three types plus `serialize_with` helpers for the string-valued enums, and it reopens phase 2. Either it follows, or it stays as the documented exception on the strength of its governance. It shouldn't drift into being decided by whoever touches it next.

One thing is lost and worth naming, because it was built on purpose the day before. `TranscriptDocument::of` takes `Transcript` apart by pattern, which compiles only because the module is a child of `transcript` and can see private fields, so a field added to the run's transcript is a compile error until the document says whether it's published. A mapping in another crate must build from the reader methods instead, and a new field beside a new reader compiles silently. That guarantee becomes a test: the transcript needs the checked-in fixture and the changelog-gate entry the outcome already has, and phase 4 owes them whether or not anything else here happens.

## 2026-09-21 The loop's floor is 99, with no headroom

`Stopped::defect` is a pure function, and the sentence it builds is what an operator reads when a defect fires: that lablet is at fault rather than the provider, which of its own rules it broke, and that the run still came back with an outcome. Nothing tested it. Two tests in `src/service/tests.rs` do now, and they lift `lablet-run` from 98.4% regions to 99.1%, and from 98.6% lines to 99.6%.

That's as far as it goes. What's left is six regions in two arms, the ones that take a `TranscriptError` from `Run::responded` and from `Run::tool_calls`. Neither can be reached through `RunService::run`. `responded` refuses only when the turn before it went unanswered, and the policy stops a turn that called no tool before the next response is recorded, since `after_response` returns a reason for `Calls::None` in both completion modes. `tool_calls` refuses only a mismatch of ids or count, and the loop builds one outcome for each call, in call order, from the turn's own `tool_uses`.

A typestate was considered, which would have deleted the runtime check and made the sequence a compile error instead, a stronger guarantee than the one being kept. It doesn't work in safe Rust: the correspondence between outcomes and calls is a length equality, and the only ways to make that infallible are silent truncation or a panic. Both are worse than the arm.

So the floor is 99 rather than 98, and it has no headroom in the regions at all. 99% of 692 is 686 and there are 686. One new region no test can reach turns the gate red on the commit that adds it, which is the intent: a check of the third kind recorded above should be argued for when it's written, not found later by a coverage report. Lines have three of slack, because 522 of them round more kindly.

The risk this takes on is worth stating. A legitimate third-kind check added in phase 4 fails the gate the moment it lands, and whoever hits it has to either cover it, argue it, or move the number. That's the cost of a floor with no slack, and it's accepted on purpose.

## 2026-09-21 Domain and application are in-memory models

The general form of the three entries above, and it settles the outcome question the last one left open. Nothing below the adapters carries a published shape. A run's transcript, its outcome and everything they hold are values; collapsing one into bytes happens once, at the boundary, in an adapter. It's in `brief.md` as a design commitment.

The application ring already satisfies it and always did. `lablet-run` has no serde derive at all, and `lablet-policy` has no serde anywhere. What the ring holds of JSON is `serde_json::Value` in five positions, all of them values that are JSON by the provider APIs' definition: a tool call's arguments, a tool's input schema, an opaque provider block, the completion schema, and the `task_complete` argument. So this is a domain-crate question, and the cost lands in `lablet-model` alone.

**The outcome moves, with the transcript, in one change.** The previous entry left this open. The same facts decide it: nothing in production reads a `RunOutcome` back, since every `from_value::<RunOutcome>` in the workspace is in `outcome/tests.rs`, and spec section 10 still defers a schema version for the outcome JSON, which has nowhere to live while the domain type is the wire form. That's the wall the transcript hit, and it's what turned a tidy idea into a necessary one.

Two things make the outcome easier than it looked. `RawOutcome` isn't only a read target: it's `pub(crate)` and doubles as the parts `Run::finish` fills in, so removing the read path doesn't delete it. It becomes the construction parts, `RunOutcome::closing` goes public as a checked constructor, and the rules stay in the domain enforced once. And `RunSummary` and `FinishedRun` derive `Serialize` for nothing at all, since the wide event is a field mapping against the generated key list rather than a serialisation, so those two and the six types reachable only through them lose their derives with no replacement anywhere.

They move together rather than in sequence, and that matters. Both documents land in the same adapter crate, both need `Usage`, and both want the same mapping convention and the same fixture pattern. Doing the transcript in phase 4 and the outcome afterwards means building that crate and then reopening it, which is the whole of the objection to moving the outcome at all.

**The one exception, stated so it isn't discovered later.** `Message` and `ToolResult` keep `Serialize`, and the rule is about published shapes rather than about the derive. `RequestBytes` sizes a provider request by rendering the conversation, so every provider reports growth the same way, and those bytes are counted and discarded. No consumer ever sees that encoding, and no field name in it reaches a contract. Measuring with a canonical rendering isn't publishing one; moving it to the adapters would make the number per-provider, which is the opposite of what it's for.

**What phase 4 owes, which the build plan is amended with in this commit.** The transcript needs a checked-in fixture and a changelog-gate entry. That was recorded as an obligation when `TranscriptDocument::of` lost its compile-time drift guard, and the obligation never reached the plan work is actually done from. The outcome already has both, and its gate path doesn't change when the type moves.

Nothing here reopens the provider adapters, which were never coupled: a provider's wire format is its own type in its own adapter, and that's been true since 2026-09-18.

## 2026-09-21 `TranscriptError::AlreadyAnswered` is gone

The variant and the check that produced it. `Transcript::answer` had four refusals and now has three, and `Run::tool_calls`'s contract is one sentence shorter.

It went because the rule it held is nearly held by the rule beside it. Outcomes are matched against the call ids of the response, and those don't change when a turn is answered, so a second set naming other calls is refused as `OutcomesDontAnswerCalls` exactly as the first would have been. `AlreadyAnswered` only ever caught one case the other misses: a second set naming the same calls with different content, which silently replaces the first.

That narrowing is the honest cost and it's smaller than it sounds, but it isn't nothing, and it sits against the rule recorded the same day: a check no caller can reach stays when what it buys is that a defect surfaces rather than corrupting data. The two `Stopped::defect` arms in the loop were kept on exactly that reasoning. This one goes because its remaining case needs a caller that answers the same turn twice, with the same call ids, carrying different outcomes, and `Run::tool_calls` has one caller, which builds its outcomes from the turn's own `tool_uses` and calls it once. Held against the other two, which guard a sequence the loop could get wrong in more than one way, this is a guard for a single contrived shape.

Two stale lines in spec section 3 went with it: the `TranscriptError` listing still carried `AlreadyAnswered`, and also `Response(ResponseError)`, which was deleted with the transcript's read path a few entries above and never removed from the spec.

`lablet-model` holds at 100% lines and regions on a smaller denominator, 768 and 917, since the deleted code was covered by the test that has gone with it.

## 2026-09-23 A run's states are types, and a turn's tool calls run together

Supersedes the 2026-09-21 entry "The loop's floor is 99, with no headroom" on its one argument, and closes the open question "Parallel tool execution within a turn" in spec section 10. Held to a review before any code: it changes one rule in `contributing/README.md`, and that change is argued below.

**The typestate was refused for a reason that doesn't hold.** That entry said the correspondence between outcomes and calls is a length equality, and that the only ways to make it infallible are silent truncation or a panic. That's true only while the loop builds the list and hands it over. Turn it around and the equality holds by construction: the domain walks the calls of its own turn, asks the loop for one answer to each, and puts the call id on each answer itself. The loop never holds a list whose length could be wrong.

The other two refusals go with it once the run's states are values. `Run::responded` consumes the run and returns a `Responded`, which is `Final` when the response called no tool and `Pending` when it called at least one. `Final` can only finish. `Pending` can finish, at point R, or be answered, which gives back a `Run` ready for the next call. So nothing records a response while calls wait for answers, and nothing records one straight after a response that called no tool, because neither sequence can be written. The policy already stops every turn that calls no tool, and now the type says so too. A later feature that lets the user speak again gives the run new input, which is the other way a `Run` is made.

Point R splits along the same line, or the loop would trade two unreachable arms for a third. Matching `Responded` after `after_response` returned `None` leaves a `Final` arm that can't run, because the policy stops every response that called no tool. So the policy is asked inside each arm. `StopPolicy::after_final` takes a response that called no tool and returns a bare `StopReason`, since such a response always stops the run. `after_response` takes the rest, and `Calls` loses its `None` variant. `Pending::calls` replaces `Turn::calls` as its only producer.

`TranscriptError` is deleted, `Run::responded` stops returning `Result`, `Run::tool_calls` is replaced by `Pending::answer`, and `Stopped::defect` goes with the two arms that called it. The transcript's checks are deleted rather than left unreachable, so `lablet-model` holds at 100%, and `lablet-run`'s floor rises to 100. The "third kind" of uncovered code, recorded in the entry "The transcript has no read path," has no member left. It stays a category a later check may argue for.

**Tool calls in one turn run together, because that's what the field does.** Both provider APIs return several tool calls in one response by default: Anthropic has `disable_parallel_tool_use` to turn it off and OpenAI has `parallel_tool_calls`. The common harnesses run those calls concurrently. A measuring loop that runs them one at a time reports a turn's wall time as the sum of its calls where a production harness would report the longest, so lablet would measure something no one ships.

Running every call at once is wrong for tools with side effects. So the rule is the one Claude Code uses. Each tool is `Shared` or `Exclusive`. A run of consecutive `Shared` calls forms a group that runs concurrently, up to `tools.max_concurrent_calls`. Each `Exclusive` call runs alone. Groups run in call order. Reordering, to run every `Shared` call first, was rejected: a read the model placed after a write would run before it.

- **Classification is on `ToolSpec`**, as `concurrency: ToolConcurrency`, whose default is `Exclusive`, so a tool nobody classified keeps today's behaviour. `read_file` is `Shared`, and `bash` and `write_file` are `Exclusive`. An MCP tool is `Shared` exactly when the server annotates it `readOnlyHint: true`. That's the server's word, trusted as its tool names already are. A call the loop answers itself, to a name no tool has or with arguments that didn't parse, reaches no executor and counts as `Shared`.
- **The cap defaults to 10, and 1 runs every call alone**, which is today's loop. That's the escape hatch for a script whose event order a test asserts exactly, and for a server that can't take concurrent requests.
- **Outcomes stay in call order**, so the transcript, the rendered results and the consecutive-tool-error count read exactly as before. Events of calls in one group interleave, and observers already key them by `call_id`.
- **`lablet.tool_calls.latency_ms.total` sums call time.** Once calls overlap, that can exceed the run's wall time. It already meant the sum, and the spec now says so, so nobody reads it as the time the tool phase took.
- **`ToolExecutor::execute` may be called concurrently** for `Shared` tools. That's a new port obligation and a new case in the conformance set.

**Why the domain composes futures, and the rule that changes.** Chosen by the human over two alternatives that kept the domain synchronous: tool calls answered one at a time, or a concurrent loop handing the domain a finished list and a count check that can't fail. Three properties are wanted, and any two are easy:

1. The domain is synchronous, as `contributing/README.md` requires: "Anything async is application or outward."
2. Calls in one turn run concurrently.
3. There's no branch that no test can reach.

Sequential and synchronous works: the domain hands out one linear token per call, each answered in turn. Concurrent and synchronous works if the domain keeps a count check, which is where the loop is today. Concurrent with no unreachable branch needs whatever knows the calls to also collect the answers, because tokens that leave the domain and come back through a join the domain doesn't own can always come back short. Rust has no linear types to stop a token being dropped.

So `Pending::answer` takes an `AsyncFn(&ToolUse) -> Answer` and returns a future. It composes rather than acts: it creates no future of its own, reads no clock, starts no task and names no runtime. It schedules futures its caller made, in the order the calls came and in groups the concurrency rule sets. The rule the domain was synchronous for is testability, and that's what the rule in `contributing/README.md` now states. A domain crate has no runtime, no I/O, no clock, no port and no threads, and a plain `#[test]` can drive every function in it deterministically. An async function meets that when its tests poll it to completion with `std::task::Waker::noop()`, stable since Rust 1.85, and hand it futures the test controls. A future that reports "not ready" once and records when it starts and ends is enough to show that shared calls overlap, exclusive calls don't, and outcomes come back in call order, on one thread and with no dev-dependency. Anything that needs a runtime to test stays in the application or further out. `lablet-policy` stays synchronous because nothing in it waits.

What the change costs, stated so it isn't found later:

- **The domain schedules the tool phase.** Control flips: the domain calls back into the loop, and the grouping, the order and the cap are the domain's.
- **The run lives inside the future while tools run.** Anything that abandons that future part-way, such as a timeout around it or a drop on cancellation, loses the run and its transcript with it. So interrupting a tool phase, still an open question, has to be built into `Pending::answer` rather than wrapped around it.
- **`futures-util` is new to the domain,** below.

The join is `futures-util`'s `StreamExt::buffered`, with default features off. It preserves order and bounds concurrency, which is exactly the group rule. It's new to the dependency graph, isn't on the domain's forbidden list, and replaces the other option: a hand-written, waker-correct, bounded, ordered join is code a lightweight project shouldn't own.

`AsyncFn` rather than `AsyncFnMut`, because calls in one group borrow the closure at the same time. The one risk is `Send`. On stable Rust, a bound can't say that the future an async closure returns is `Send`, so the future `answer` returns is `Send` only because auto traits leak through opaque types at its one concrete call site, in `RunService::run`. That's the first thing the implementation checks. If it fails, the fallback is `Fn(ToolUse) -> Fut` with an owned `ToolUse`, which is the clone the loop already makes today.

**What the loop needs from an answer.** `ToolCallFinished` reports the capped sizes, and the cap is applied when an outcome is built, so the loop has to see a measured answer before it hands it back. `Answer::measured` takes what `ToolCallOutcome::measured` took, less the call id: the status, the content, the cap, the start and the latency. It exposes what the event reads. The domain turns it into a `ToolCallOutcome` by adding the id of the call it asked about, and the outcome's fields don't change.

**Not changed, and worth saying.** Cancellation is still polled at points A and B, and a turn's tool phase still runs to its end once it starts. Stopping between groups is now possible in one place, since the domain owns the phase. It needs a status for a call that never ran, because the transcript accepts all of a turn's outcomes or none, so it stays open. A turn's `task_complete` interception and point R's order are untouched. **Partly superseded on 2026-09-30 by "Six decisions before phase 5":** the status exists as `NotRun`, and phase 5 stops calls in flight on cancellation. Until then a tool phase still runs to its end once it starts.

## 2026-09-23 Two fixes from the review of the loop

- **Cancellation is polled before each retry**, after the backoff and before the next attempt. Spec section 1 says a run never makes a provider call after the condition that should have stopped it, and a retry is a provider call. Without the check, Ctrl-C during a run of provider failures paid for every retry left and slept through each backoff.
- **A filter name that nothing serves is refused**, as `ToolSetError::UnknownFilterName`. A misspelt `deny` entry left the tool it meant to deny on offer, so a safety control could fail open without a word, and a misspelt `allow` entry offered nothing. Names are checked against what the executors serve. So in explicit mode, `task_complete` in either list is refused as well, since the filter doesn't apply to it and naming it can only be a mistake.

## 2026-09-23 `Pending::answer` takes `Fn(ToolUse) -> Fut`

The phase 3a spike took the fallback recorded in "A run's states are types, and a turn's tool calls run together." With `answer: impl AsyncFn(&ToolUse) -> Answer`, the future `RunService::run` returns isn't `Send`. The compiler reports that `Send` is "not general enough": the future an async closure returns for a borrowed argument is higher-ranked over the borrow, and a generic function that holds several at once can't prove them `Send` for every lifetime. It fails the same way through `StreamExt::buffered` and through a hand-driven `FuturesOrdered`, so the join isn't the cause. With `answer: impl Fn(ToolUse) -> Fut` and `Fut: Future<Output = Answer>`, the future is `Send`, a spawned task on a multi-threaded runtime runs it, and a turn's groups overlap and finish in call order as designed. The price is one clone of each `ToolUse`, which the loop already made.

The same spike found what the testability rule needs from its tests. A future that returns `Pending` without waking its task is never polled again by `FuturesOrdered`, so a test that drives `answer` with `Waker::noop()` and such a future hangs rather than fails. The domain's test helper polls a bounded number of times and panics past it, and the futures a test hands in wake themselves before they return `Pending`, as any correct future does.

## 2026-09-24 A `task_complete` call whose arguments didn't parse completes nothing

It used to complete an explicit run with a null structured result. In explicit mode that result is what the run is for, so a run reporting `completed` with nothing in it looks like success to a grader and holds none. It was also the one call with bad arguments that the model never saw an error for. Now `Pending::calls` reads such a response as `Tools`: the `task_complete` call is answered `malformed_input`, the response's other calls run, and the model can call it again. The result counts as a tool error, so a model that keeps failing reaches `tool_errors_exhausted`, and `max_turns` bounds it too.

`ToolSet::source` answers for `task_complete` now, because it's offered: a real name with bad arguments is `malformed_input`, not `unknown`. When validating the argument against `run.completion_schema` lands, an argument that doesn't fit takes the same path.

**Partly superseded on 2026-09-28 by "Three of the parity matrix's proposals, decided" and "What the review of the plan changed":** no count of tool errors ends a run, and `tool_errors_exhausted` is gone. The call is an invalid one, since it reaches no tool, so a model that keeps making it with no call beside it that reaches a tool stops the run with `invalid_calls_exhausted` after `run.max_consecutive_invalid_turns` such turns in a row (3 by default). Everything else in this entry holds.

## 2026-09-24 A group of tool calls is a pool, not an ordered join

Supersedes two sentences of "A run's states are types, and a turn's tool calls run together": that `Pending::answer` creates no future of its own, and that the join is `StreamExt::buffered`. The phase 3a review found both untrue of what was built, and found that an ordered join is the wrong one.

An ordered join, `buffered` or `FuturesOrdered`, only starts a call when the earliest call still running ends. With a cap of 2 and a group of three reads taking 5s, 10ms and 10ms, the third waits the whole 5s for the first while a slot sits empty. The cap was meant as a pool, and lablet reports a turn's wall time as the thing a production harness would show. So a group now runs on `FuturesUnordered`: each answer carries its place in the group, a new call starts whenever any call ends, and the answers are sorted back into call order before they're recorded. The loop that fills the pool is about ten lines, which is less than the objection to a hand-written join supposed, because `FuturesUnordered` does the part that's hard to get right. The only future `answer` makes is the one that pairs a call's answer with its id and place.

## 2026-09-24 What the phase 3a review changed

Two reviewers, six findings, all medium, all confirmed and fixed. The pool that replaced the ordered join has its own entry above. The one that changed a contract rather than a line:

- **`retry: Some(wait)` is the decision made when an attempt fails, not a promise.** Polling cancellation after the backoff means a run can report a retry and then end without it. Emitting the failure after the sleep would put its time in the wrong place, so the contract changes instead: the event's field, spec section 5, and `lablet.retry.will_retry` now say a run cancelled during the wait ends before the retry, and the root span's stop reason says so.

The rest brought code and log back into agreement: a call whose arguments didn't parse counts as `Shared`, as the design entry says, so `ToolSet::concurrency` takes the whole call; `ToolSet::is_task_complete` had lost its last production caller to `Pending::completed_with` and is gone; and three tests claimed more than they held, the equal-transcript one because the fakes share one clock, so it now compares outcomes and says why.

## 2026-09-25 Lablet's behaviour follows named reference loops

Decided by the human. Until now, lablet aimed to be "representative" of production agent loops without naming any, so no decision could be checked against that aim. Two design-review passes on 2026-09-24 reached different verdicts on the same findings because they used different yardsticks. A harness's choices also change what it measures: one study that ran the same tasks and models through three harnesses found about a 40-fold difference in tokens per solved task between two of them ([2607.22585](https://arxiv.org/abs/2607.22585)). So lablet names the loops it aims to behave like, pins their versions, and takes its defaults from them.

- **References.** The primary reference is Claude Code, run through the Claude Agent SDK at 2.1.281. Codex CLI rust-v0.156.1 is the secondary, for OpenAI models; parity with Codex is partial, because its shipped configuration has current models call tools by writing JavaScript. mini-swe-agent v2.4.6 is a minimal baseline for bash-only model comparisons. OpenCode, Copilot CLI, Goose, OpenHands and Gemini CLI break ties where the references are silent or split. They were chosen from a survey of eight loops profiled in depth and thirteen screened, on three grounds: how widely each is used, whether each can be run and observed in a comparable configuration, and which provider path each covers.
- **Profiles.** A profile is a named set of setting values that reproduces a reference, and the default profile follows the primary. A setting exists only where a reference behaves differently, or where an experiment needs that factor isolated.
- **The profile doesn't change with the model.** A model comparison changes only the model. Most multi-provider harnesses tune their loop for each model, and lablet deliberately doesn't. A user who wants each model run the way its vendor's harness would run it chooses that profile explicitly. The default is a choice of loop behaviour, made on usage evidence. It doesn't privilege any provider's API or format, since the same loop runs every model and the domain model stays neutral.
- **Checking.** Parity is checked three ways: behaviour documented from sources, captured requests compared with lablet's, and outcome distributions compared over repeated runs. Claude Code is captured with its own raw-body logging, because routing it through a proxy switches off some of its defaults.
- **Where it lives.** The matrix and the reference documents live in `product/research/parity/`, which leaves "three areas, four product files" as it stands.

This entry settles the references and the principle, and nothing else. The matrix's proposals change recorded behaviour: MCP tool names, truncation, server lifetime, tool search, output limits and retries. They're decided separately, after the captures, together with the design-review decisions still to be recorded. The brief gains the principle when those land.

**Partly superseded on 2026-09-28 by "Two rules bound what lablet copies from its references":** mini-swe-agent is evidence and not a baseline profile, a profile sets values only, and the brief has the principle. The primary, the secondary and the rule that the profile doesn't change with the model all hold.

## 2026-09-28 What the human decided from the design review

The second design review of 2026-09-24 (`product/research/design-review/second-pass.md`) put eleven decisions to the human, each of which conflicted with a recorded decision or with the brief's scope. The human took the recommended option on all eleven the same day. They're recorded here, late, because the parity survey that followed (`product/research/parity/`) was allowed to revise them first. Each entry says what the survey found. The spec, the brief and the build plan aren't changed by this entry: those changes follow the captures the parity matrix calls for, so they're written once.

1. **Tool errors never end a run by default.** `run.max_consecutive_tool_errors` is off unless set, and when set only harness errors count: an unknown name, arguments that didn't parse, a timeout or an executor failure. A tool's own error result never counts. No reference loop ends a run over tool errors, and recovering from one is behaviour a tool-surface comparison exists to measure. _The parity matrix (row B9) refines this:_ Claude Code does end a run after repeated calls whose arguments don't parse, so lablet should too, once row C2 settles how such a call is answered.
2. **A response cut off at `max_tokens` ends the run.** `output_truncated` stays a terminal stop, `model.max_tokens` defaults to 32,000, and `run.provider_timeout` rises to about ten minutes so a long response can complete. _Contradicted by the survey (row B3):_ Claude Code sends a hidden resume message up to three times before failing, and no reference ends the run at the first cut-off. The matrix proposes resuming as the primary does, with stopping at the first cut-off as a setting. To be settled after the capture.
3. **`task_complete` beside other calls is refused.** When a response calls `task_complete` together with other tools, the other calls run and `task_complete` is answered with an error asking the model to call it alone once they've finished. This supersedes the spec section 1 rule that other tool calls in the same response are ignored, which reported a run as completed when the work it claimed never ran. _The survey found no direct evidence either way (row B2);_ what Claude Code's structured-output tool does with sibling calls needs capture.
4. **OpenAI models use the Responses API.** The OpenAI adapter drives OpenAI's own models through Responses, with encrypted reasoning sent back on each call and nothing stored server-side, and every other OpenAI-compatible server through Chat Completions with a per-server setting for sending `reasoning_content` back. The wide event records which API was used and whether reasoning was replayed. Chat Completions can't return OpenAI reasoning, so a loop built on it alone re-reasons every turn, which no production harness does. The brief's wording widens from anything speaking the OpenAI chat completions API to OpenAI's API and anything speaking chat completions. _Confirmed by the survey:_ Codex, the OpenAI-path reference, uses Responses only.
5. **Skills load through a tool.** A built-in skill tool lists each skill's name and description in the system prompt and returns a skill's body when called; inlining every skill into the system prompt stays as a named mode. Telemetry records which skills were loaded and when. Inlining measures a system-prompt change, not a skill: it can't measure whether the skill's description gets it loaded at the right moment, which is what a skill author tunes. _Confirmed by the survey:_ every reference and tie-breaking loop that supports skills loads them this way (row G1).
6. **Billed failed attempts are recorded.** `ProviderError` carries an optional `Usage`, failed-attempt totals go on the wide event, and they count in the cost and the token budget. Per-turn usage stays the usage of the successful call. The spec section 1 rule that a failed attempt reports no usage under-counted exactly in the runs that fail most.
7. **Runs can be grouped and labelled.** `lablet.config.digest` hashes only the settings that affect behaviour: output paths, telemetry settings and `telemetry.resource` leave it. `Lablet::run` takes a request holding the prompt, an optional run id and labels, and the declared labels `lablet.task.id`, `lablet.experiment.id` and `lablet.trial` go on the wide event, every span and the outcome. This narrows "Composer resource attributes stay on the Resource": arbitrary composer keys still live there, and these three are declared so live-check accepts them. A per-run transcript path gave every run its own digest, and a reused `Lablet` couldn't say which run belonged to which task.
8. **A setting the provider can't apply fails the build.** `seed` on Anthropic, `thinking` on Chat Completions and the like are refused at build, naming the setting and the provider, and a setting that can be mapped is mapped. An A/B on a setting the provider ignores looks valid and measures nothing.
9. **MCP servers live across runs, and each run checks them.** Servers keep the lifetime `Lablet::build` to `Lablet::shutdown`. Each run starts by checking every server and restarting any that died; a failed restart stops the run at once with its own stop reason; the wide event records how many runs each server has served. _Contradicted by the survey (row F1):_ every MCP host surveyed starts servers for each run or session. The matrix proposes fresh servers per run by default, with reuse as a setting. To be settled with the other matrix proposals.
10. **The prompt cache is shared across runs by default.** `model.cache_scope: shared | run`, default `shared`; `run` puts a short per-run marker at the start of the system prompt so no two runs share a prefix. The scope is reported. Sharing is what production agents do, and isolation is for a composer who needs cost and first-call latency independent of scheduling. _Confirmed by the survey (row A5):_ Claude Code builds its prefix to match across runs.
11. **Content telemetry keeps the conventions, and the wide event is protected.** Content records carry the full input history per call, as the GenAI conventions require. Content is flushed first and the wide event goes in its own export, attributes get a length limit, and a count of dropped records is reported. The history grows with the square of the turn count, and the wide event, emitted last, was the record most likely to be dropped.

**Context management, decided the same day.** A new phase 3b, before phase 4, builds masking of old tool outputs and every seam later context management needs: the transcript is the record and the context sent is a view of it; `run.context: full | mask_old_outputs { keep_last: N }`; `request_bytes` measured per call as sent; chat spans that carry a purpose, so a provider call needn't be a turn and the `turns + retries` identity goes; room in the transcript document for a compaction entry; and a `Run -> compact -> Run` transition. Summarisation was deferred to phase 10 on those seams. _The survey confirmed masking first (rows E1 and E2):_ Claude Code clears old tool outputs before it summarises, and so does Gemini CLI. It also found that every production harness summarises, so the matrix proposes planning summarisation rather than leaving it unscheduled. The phase 3b entry in the build plan follows once the matrix's proposals are settled, so it's written against the final scope.

**Partly superseded on 2026-09-28 by "Three of the parity matrix's proposals, decided" and "Two phases before phase 4, and one after the release":** decision 1 gains a cap on invalid calls, decision 9 is reversed, the spec, the brief and the build plan change now and not after the captures, and summarisation moves from phase 10 to phase 12.

## 2026-09-28 Two rules bound what lablet copies from its references

Decided by the human. The parity matrix proposed following the primary reference in almost every row. Followed to the letter, that makes lablet a copy of a product that ships almost daily, and a copy can't be complete or kept current. It's also more than lablet's users need. They compare variants: two tool surfaces, two skills, two prompts, two models. What has to carry over to real use is which variant wins and by roughly how much, and that needs lablet to match only the behaviours that bear on the thing being varied.

- **Rule 1: what lablet builds.** A behaviour of a reference is built only when all four hold. It changes what the model is sent or how a run proceeds. It bears on something a user varies. It's a pattern, found in the primary and at least one other reference, or in most of the loops surveyed. It can be specified from public evidence and checked against a recorded request.
- **Rule 2: a profile sets values.** A profile configures behaviours lablet has for other reasons. It's never the reason to add one.
- **A harness answers questions about itself.** Lablet answers which variant is better and where the tokens went, under one controlled loop. What a given harness does with a given server or skill is answered by running that harness. The reference configurations in `product/research/parity/` are kept for that, as well as for calibrating lablet's defaults.
- **Transfer checks decide where fidelity is added.** The same comparison of two variants is run in lablet and in a reference. Where the direction and rough size of the effect agree, no more fidelity is added for that kind of question. Where they disagree, the behaviour responsible is found and put to rule 1. The matrix is a list of suspects, not a list of work.
- **Known differences are written down** and published with lablet, so a user knows when to reach for the real harness. Today they're permissions, hooks, agents that start other agents, memory and project files; context a harness injects; prompts, tools and limits tuned for each model; Codex's code mode; switching models after a refusal; moving a timed-out command to the background; a tool list that changes during a run; and keys in a tool's `_meta` that belong to one vendor.
- **mini-swe-agent is evidence, not a profile.** It can only offer a bash tool, its way of completing a run is its own, and anyone can run it directly. Reproducing it would break rule 2. A bash-only preset may exist as a set of values, and doesn't claim to reproduce it.
- **Tool search is inside the line, and later.** The three most used harnesses hide MCP tool schemas until the model searches for them, which decides whether a tool is found at all. It passes rule 1 and it's expensive, so it's scheduled as its own feature. Until then it's the largest known difference.

The matrix's proposals are revised to match, and each is labelled keep, default, build, setting, later or difference. Of its 36 rows, 16 now ask for work before release, 5 are later features, and 15 ask for none. They're still proposals: each change of recorded behaviour is decided on its own.

## 2026-09-28 Three of the parity matrix's proposals, decided

Decided by the human. Each reverses or refines an earlier decision, so none was the builder's to make.

- **MCP servers start for each run.** Reverses decision 9 of "What the human decided from the design review." Every MCP host surveyed starts its servers for each run or session (matrix row F1). So a server's caches, sessions and files never carry from one run into the next, and a server that died in one run can't fail the runs after it. `tools.mcp_lifetime` is `run` by default. `build` starts the servers, because it has to list their tools to check the config, and those servers serve the first run; every later run on the same `Lablet` stops them and starts them again first. `lablet` keeps servers from `build` to `shutdown`, for a library user who'd rather not pay a server's startup on every run, and there a server found dead at the start of a run is started again. The wide event reports the lifetime. The count of runs a server has served, which decision 9 asked for, isn't kept: the lifetime says whether state could have carried over. How a run ends when its servers can't be started is left to phase 8's design.
- **An MCP tool's name carries its server's.** Supersedes "MCP tool names are never prefixed by default" (2026-09-18). That entry kept the server's own names so that measurements reflect the server unchanged. But every harness that documents its naming adds the server's name (matrix row A3), so in real use no model sees the bare name, and a name tuned in lablet would be tuned for a form nothing sends. The default is `mcp__<server>__<tool>`, the primary reference's form, and `names: own` on a server keeps its own names. A server's name in the config is held to the characters a tool name allows, and a name too long once prefixed is a build error, as an invalid name already is. Allow and deny lists name tools as the model sees them.
- **Repeated invalid calls end a run, and error results never do.** Refines decision 1, which turned the cap off. An invalid call is one that reached no tool, so lablet answered it: the name is one no tool has, the arguments didn't parse, or the call was rejected. The primary reference and mini-swe-agent both end a run after repeated calls they can't parse (matrix row B9), and a model that can only produce such calls is a run that has failed, not one that's slow. `run.max_consecutive_invalid_calls` is 3 by default and `null` turns it off. A call that reaches a tool resets the count whatever the tool returns, so an error result, a timeout or a dead server never ends a run. The stop reason `tool_errors_exhausted` becomes `invalid_calls_exhausted`, since the old name would now say the wrong thing. The review of the plan, recorded below, made the count one of turns and not of calls, and the setting `run.max_consecutive_invalid_turns`.

## 2026-09-28 What the builder took from the design review

The second design review left findings that needed no decision from the human. `contributing/README.md` has the builder sort a review's findings before anything is fixed, and drop what isn't worth handling in a lightweight project. This is that sort, together with the matrix's proposals that are the builder's to settle. Seven findings aren't in it because the human decided them: H1 is decision 1, H3 decision 4, H8 decision 7, M3 decision 2, M5 decision 3, M16 decision 10 and M19 decision 11. Finding numbers are the review's (`product/research/design-review/second-pass.md`), and row letters are the matrix's (`product/research/parity/matrix.md`). The phase named is the one that builds it.

**Taken.**

- **A count the provider didn't report is absent, not zero** (H5). `Usage` holds its reasoning, cache-read and cache-write counts as options, and telemetry leaves out an attribute it has no count for. Several servers answer to one provider name and report different things, so the name can't tell a consumer which zeros are real. A sum is absent only when no call reported the count. Phase 3b.
- **A call that timed out has stopped** (H9). `ToolExecutor::execute` returns `Timeout` only once the work has stopped: a process it started has exited, killed if need be, and a request it sent has been cancelled. Otherwise an exclusive call can still be writing while the next call runs, which breaks the order spec section 1 promises. The contract is stated in phase 3b. The conformance case and the kill of bash's process group are phase 4, and the cancellation notice to an MCP server is phase 8.
- **A call's deadline is never later than the run's, and the timeouts are the executors'** (M17, C4). The loop hands each call the time the run has left, and an executor applies the shorter of that and its own limit: 120 seconds for the built-in tools, as the primary has for bash, and 300 for an MCP call, as Codex and OpenHands have. `run.tool_timeout` goes, since one limit for every tool was the outlier. A provider call gets the shorter of `run.provider_timeout` and the time left. So a run overruns `run.timeout` only by the time a call takes to stop, where spec section 1 promised one call and delivered a whole tool phase. Phase 3b for the loop, phases 4 and 8 for the executors.
- **One output cap, three ways to cut** (M1, M2, C5). `tools.output_cut` keeps the start (`head`), both ends (`head_tail`), or a short preview (`preview`), each with a line that says what was left out. The default is a preview of 2,000 bytes once a result passes 50,000, which is what the primary sends for an MCP result. Saving the full output for the model to open isn't copied: with only the tools under test offered, the model has nothing to open it with. An executor keeps no more of a call's output than the cut can use and counts the rest, so a command that never stops writing can't take lablet's memory and the run's record with it. Phase 3b for the model and the port, phases 4 and 8 for the executors.
- **Retries follow the server's hint** (M14, D1, D2). `ProviderError` carries the wait the server asked for, and the wait is the longer of that and the backoff. A hint longer than `run.retry_hint_max`, 60 seconds by default, ends the retries, as it does in the primary. The defaults become 10 retries backing off from 500 milliseconds to 32 seconds, the primary's, where 3 retries in about 3.5 seconds was the outlier and reported a throttled provider as a failed run. A wait gains jitter of up to a quarter, derived from the run id, the turn and the attempt: two runs don't retry in step when many lablets share one API, and one run's waits are still the same every time. `run.retry_jitter: 0` turns it off. Phase 3b.
- **A rejected key is its own kind of failure, and it's a run** (M23). `ProviderErrorKind::Auth` isn't retried and is the chat span's `error.type`. Spec section 1 gave exit code 1 to credentials rejected before any `RunStarted` event, which can't happen, because the event is emitted before the first call. So such a run ends as `provider_error`, prints its outcome and exits 2, and exit code 1 is kept for what fails before `build` returns. Phase 3b for the kind, phase 5 for the exit code.
- **Error text is bounded and carries no credentials** (M20). A port's error message is cut to a fixed length and holds no URL credentials or query string, and a response body reaches telemetry only under `capture_content`. The CLI prints a config value as written, before `${VAR}` substitution. The contract is stated in phase 3b, and the adapters of phases 7, 8 and 9 are held to it by their tests.
- **The record says what the model was shown** (H7). The loop takes a digest of the tool specs as offered and a digest of the system prompt as sent, and the wide event gains both with the byte size of the tool specs. Two variants that differ in a tool's description, a server's build or a skill's text then differ in the record, where today their setup looks the same. The transcript document holds the tool specs, and the specs reach telemetry as `gen_ai.tool.definitions` under `capture_content`. Phase 3b for the digests, phase 4 for the rest. A server's own name and version join them in phase 8.
- **Both documents name their run** (M22, M24). The outcome gains `schema_version`, and the transcript document gains the run id, the config digest, lablet's version, the model, and when the run started. `run.transcript_path` may hold `{run_id}`, so a `Lablet` that runs twice doesn't overwrite its first transcript. Phase 4.
- **Bash has stated semantics** (H1, M7). It starts a new process for each command, which is what every reference does, its result is what the command wrote and then its exit code, and a non-zero exit is an ordinary result. `read_file` takes an offset and a limit, so a file longer than the output cap can be read in parts (M1). Phase 4.
- **The built-in tools are off unless asked for, and their root is never lablet's own files** (M21). `tools.builtin.enabled` is empty by default, since a run that measures an MCP server has no use for a shell. With a built-in enabled, `tools.builtin.root` has no default and `build` refuses a root that holds the config, a prompt or skill file, the transcript or the telemetry file. Phase 4.
- **A child process gets none of lablet's secrets** (first review, finding 4). Bash and a stdio MCP server start with a short list of environment variables and whatever the config adds, so the model can't read the API key with `env`. File tools check a path after resolving symbolic links. The quality bar stops calling bash sandboxed: its isolation is the environment lablet runs in. Phases 4 and 8. **Partly superseded on 2026-09-30 by "A command inherits lablet's environment, less lablet's secrets":** `bash` inherits lablet's environment less the variables lablet reads its own secrets from, and the values of those secrets are cut out of every tool result. A stdio MCP server's short list waits for phase 8. The check of a path after its links are resolved, and bash's isolation being the environment lablet runs in, still hold.
- **No turn cap by default** (B6). A cap of 30 ended tasks the references would finish. The run timeout and the token budget bound a run, and `run.max_turns` stays for a user who wants one. Phase 3b for the policy, phase 4 for the default.
- **The cache covers the conversation** (H2, A5). Breakpoints go on the system prompt, the tool specs and the latest messages, placed as the capture shows. `model.cache` stays a switch: no reference caches the prefix alone, so there's no third mode to offer. Phase 7.
- **The OpenAI adapter's section is rewritten, with a table of errors** (H4, M15). Spec section 6 now describes both APIs and names each error it classifies, where "and equivalents" left a server's overflow to be reported as a provider failure. Phase 9 builds it.
- **Server instructions reach the model, and long descriptions are cut** (M9, A4). Instructions follow the system prompt, and they and each tool's description are cut at 2,048 characters as the primary cuts them, with `tools.max_description_chars: null` to lift the cap. Phase 8.
- **A setting says which part of an MCP result the model sees** (C7). The references send `structuredContent` when a result has it, and lablet sent `content`. `tools.mcp_result: structured | content`, default `structured`. A resource link is sent as text. Phase 8.
- **The `task_complete` argument is checked against its schema** (M6, B2). The check covers the schema keywords a result needs: `type`, `required`, `properties`, `items` and `enum`. An argument that fails is a rejected call, answered with what failed, and the cap on invalid turns ends a model that can't get it right. So no second retry limit exists beside the first. Phase 10.
- **Small ones.** `SIGTERM` cancels a run as Ctrl-C does, since that's what a container sends (M18, phase 5). A telemetry file path of `-` implies `--quiet`, so OTLP and diagnostics don't share stderr (phase 5). The instrumentation scope carries the registry's schema URL (phase 4). The token budget is described as counting tokens, not billed tokens, since a cached token counts whole (phase 3b). The docs and the Ollama example say how to set its context size on the server, because Ollama's default cuts a prompt short without an error and its API has no parameter for the size (H6, phases 9 and 11). Streaming stays an open question in spec section 10 (M3).

**After the release, in phase 12.** Each passes the first rule and is its own feature, so the order is set by what the transfer checks show.

- Tool search (A2), with the tools offered to each turn recorded (M8), since the set then changes during a run.
- Summarising the conversation, and compacting when a request is too long (E2, D3).
- Resuming after a response is cut off at `max_tokens` (B3).
- Reconnecting a remote MCP server (F2).
- Passing an image in a tool result through to the model (C7).

**Dropped.**

- **A check for a prompt that shrank between turns** (H6). Masking will shrink a prompt on purpose, and the check would flag every run that uses it.
- **A reminder when explicit mode ends on text** (M4). `ended_without_completion` reports what happened. Whether the primary nudges the model first is a question for the capture, and spec section 10 keeps it open.
- **An edit tool, or any harness's tool suite** (M7). The second rule: the built-ins are a convenience, and a suite worth measuring can be served over MCP. What M7 said of bash is taken, above.
- **A provider's fingerprint and service tier on the wide event** (M11). The chat span already names the model that answered, and the rest of M11 is decision 8.
- **A setting for the parallel tool-call flag** (M12). It waits until an experiment needs it.
- **An invalid tool name answered as an unknown call** (M13). It's rare, and it would undo the rule that a `ToolName` is always one a provider accepts.
- **Restarting a dead MCP server during a run** (M10). Servers now start for each run, so a dead one harms only its own.
- **Four small ones.** Summing durations before rounding them saves at most a millisecond a call. Request bytes keep counting thinking signatures, which are sent. An omitted image needs no counter, since the transcript shows it. A run stopped by its timeout during a backoff keeps a null `error`, because the failed attempts are already in telemetry.

## 2026-09-28 Two phases before phase 4, and one after the release

The design review's decisions change crates that are already built, and phase 4 builds on them: the observer reads `RunSummary`, the documents render `RunOutcome` and the transcript, the fake provider plays `ProviderError`, and the built-in tools answer to the `ToolExecutor` port. So the changes come first, as phase 3a's did.

- **Phase 3b settles the domain and the loop after the design review.** It holds every change above that names phase 3b, the human's decision on invalid calls, and the human's earlier decisions 3, 6 and 7 where they touch the domain. The other two decisions the human made from the matrix are about MCP, and phase 8 builds them. Spec sections 1, 3, 4 and 5 describe its design.
- **Phase 3c is context management**, until the entry of 2026-09-29 moves it to phase 7a. The human decided its scope on 2026-09-24 as phase 3b; it becomes 3c so that the two phases are reviewed apart. Its design commit opens the phase and edits the spec, as phase 3a's did, because the trigger and the placeholder of masking come from the first capture when that has been run. `run.context` is `full` by default until a capture shows what the primary does in a run with no one at the keyboard. Its other value is `mask`, which takes a trigger as well as the count the earlier entry gave `mask_old_outputs`: masking a result changes a message the cache already holds, so it has to happen in steps and not on every turn.
- **Phase 12 holds the features that wait for the release.** The entry above this one lists them. At the release each is a known difference from the references, published with the docs.
- **A capture is an acceptance item.** Phases 7, 8 and 9 each close with the human running the reference in its stated configuration and the builder comparing what it sent with what lablet sends. The values a capture settles are defaults, so a phase before it can be built on a provisional one.
- **The release is checked for transfer.** The release checklist gains one comparison of two variants, run in lablet and in the primary reference.

## 2026-09-28 What the review of the plan changed

Three reviewers read the plan before the human did: one for the spec's agreement with itself, one for the path from each decision to a phase and a scenario, and one for the design of phase 3b against the code. They found no decision missing and no leftover of the behaviour the decisions replaced. They found the design unfinished in the places below, which are fixed in the spec, the build plan and the scenarios.

One change rewords a decision the human made, so it's the human's to confirm.

- **The cap counts invalid turns, not invalid calls.** The human decided that three invalid calls in a row end a run. Counted as calls, one response that makes three such calls at once ends the run before the model has been told of its mistake once, and the same calls in another order don't: a good call last resets the count and a good call first doesn't. The references that stop for this count responses. So an invalid turn is one whose calls all reached no tool, the cap is three such turns in a row, and the setting is `run.max_consecutive_invalid_turns`. A turn in which any call reached a tool ends the count, and a call that was never run isn't an invalid call, so a turn the run's timeout cut short never counts.

The rest are the builder's.

- **Point A is asked before each retry.** A failed attempt now counts toward the token budget, and the run's deadline can now cut a provider call short. Without the check, a call could pass the budget by ten billed attempts, and a call the deadline cut short would stop as `timeout` with a retry left and as `retries_exhausted` without one. One state gives one reason.
- **A call that's never run has a status.** Once no call's deadline is later than the run's, a tool phase can run out of time between its groups. `StopPolicy::time_left` is the one place a deadline is taken from the run's timeout. When it has nothing left before a tool phase, the run stops with `timeout` and the turn's calls stay unanswered. When it runs out inside a phase, each call whose turn comes later is answered `not_run`: nothing is started for it, it's in the transcript, and it's in no total, no event and no span. Spec section 10 had asked for this status for cancellation, which can now use it.
- **An executor feeds its output to the model's accumulator.** `KeptOutput` keeps the first bytes up to the cap, counts what follows, and holds the end when the cut sends one. The first draft had an executor keep only what the cut sends, which for a preview is 2,000 bytes, so an output of 30,000 would have arrived cut and been sent as if whole. `OutputCap::new` refuses a preview longer than the cap.
- **One reading of which call completes a run.** `CompletionMode::intercepts` is asked by `Pending::completed_with` and by the loop. The first draft had the loop match on the name, which would have rejected a tool an executor serves as `task_complete` in natural mode. Two `task_complete` calls in one response are both rejected.
- **The digests are the loop's.** It takes them from the system prompt and the tool specs it sends, so a caller of the loop can't pair a digest with a prompt it wasn't taken from. `RunContext` holds neither.
- **Isolating the cache is the adapter's work, and it's partial on Anthropic.** With `model.cache_scope: run` a request carries the run id as a cache key. Anthropic gets it as a system block ahead of the system prompt, and OpenAI as `prompt_cache_key`. It's no part of the system prompt in the transcript, its size or its digest. Anthropic caches the tool specs ahead of the system prompt, so they stay shared between runs: the human's decision 10 holds with that limit, which the spec states.
- **The refusal of a setting is about what a config states.** `cache: true` is a default, and refusing it for every provider but Anthropic would have refused every other config. A default the provider can't apply is left out of the resolved config.
- **A profile is a config file, not a config key.** The defaults are the primary reference's, and lablet ships an example config for the secondary. This follows from the second rule, that a profile sets values.
- **The skill tool is described, and skills get a digest.** The tool is `skill`, the loop answers it, and the wide event lists the skills that were loaded. A skill loaded through the tool puts only its name and description in the system prompt, so `lablet.skills.digest` covers the files. Phase 10.
- **Every new attribute of the wide event has a source.** `ModelRef` holds the API and whether reasoning is sent back, `RequestParams` the cache scope, and `RunContext` the MCP lifetime, the servers' versions and the time the run started, which the transcript document and the ATIF export both need. The count of dropped records is the observer's own.
- **What failed attempts reported is absent when none reported any,** and its cache counts reach the wide event, so the cost can be worked out again from the record.
- **Small ones.** The CLI gets `--run-id`, `--task`, `--experiment` and `--trial`, so it can label a run as the library can. `Lablet::run` takes `&mut self`, as `RunService::run` does, and a blank prompt is refused when the request is made. A server's name may not hold `__`, so a prefixed tool name reads one way only. The tool set is fixed when the `Lablet` is built, so a server that comes back offering other tools has failed to start. `Lablet::run` flushes the wide event last, from a processor of its own. The parity matrix marks what's decided: 17 of its rows are decided, 5 wait for phase 12, 4 are differences, and 10 are kept as they were. The jitter of a retry is added to a wait that came from a hint too, and its hash is FNV-1a, written out in the model, since the standard library's own may change from one release of Rust to the next.

## 2026-09-29 Context management follows the first capture, and tool search has a checkpoint

Decided by the human, on the builder's advice. The loop has been designed three times and nothing has yet run end to end: no run has produced a real trace, and the observer that the telemetry contract exists for isn't built. So the risk has moved from the design to the delay, and the plan is changed to reach the first traced run sooner.

- **Context management moves from phase 3c to phase 7a.** It was placed before phase 4 because it changes what phase 4 publishes. Designing it showed that everything it adds to a published contract is an addition: a field on a tool call's outcome, and a request size that can fall as well as rise. Its trigger and its placeholder come from the capture of the primary reference, which phase 7 records, and masking was to be off by default until then. So the phase follows phase 7, and the capture settles its default with the rest. One part stays ahead of it: a chat span carries its purpose from phase 4, with `turn` its one value, so a consumer that counts chat spans as turns is right from the first release and stays right when a later phase adds a provider call that isn't a turn.
- **Tool search has a checkpoint after phase 8.** The primary reference keeps MCP tool schemas behind a search tool by default, and lablet sends them all. That's the largest known difference for the use the brief names first, and it was to wait for phase 12. Phase 8 is the first point where lablet runs an MCP server against a real model, so the first transfer check is there: one comparison of two tool surfaces, in lablet and in the reference as it ships. When the two disagree on which surface did better and tool search is why, tool search gets a phase of its own before phase 9.
- **A capture stays out of the repository.** It can hold text that isn't lablet's to publish, such as the wording of a reference's own tools, and paths from the machine it was made on, and the repository is public. What a capture settles goes into `product/research/parity/`, and any test of those facts is built from lablet's own content.
- **The plan says how far the build has come.** The build plan opens with a status table, which the builder updates when a phase closes, and the spec opens by naming the sections that are ahead of the code. Neither document said which phases were done, and the spec now describes phase 3b before the code does.

## 2026-09-29 Phase 3b, closed

Seven items landed in the plan's order, each with its gates green and its own commit: usage whose counts can be absent, provider failures with the retry policy, the stop policy, `task_complete` alone, the output cap, deadlines, and what the record names. `lablet-model`, `lablet-policy` and `lablet-run` hold 100% of lines and regions, and the one mutant that survives is the equivalent one already recorded.

What the builder decided on the way:

- **The name `task_complete` is spelled where the rule for a tool name is.** `CompletionMode::intercepts` takes a `ToolName`, and `ToolName::task_complete` read the mode's constant, so the two modules imported each other. `ToolName::TASK_COMPLETE` owns the spelling and the mode's constant is that one, which keeps `lablet-model`'s module graph without a cycle.
- **`lablet-run` takes `sha2`.** The digests are the loop's, taken from what it sends, so the crate that computes them is the application's. They're SHA-256 in lower-case hex, and the tool specs are hashed as compact JSON in the order offered with nothing between them.
- **A tool error's message is bounded as a provider error's is.** The item named the provider's, and spec section 5's sentence names both.
- **A cut consumes what an executor kept.** Nothing builds a `KeptOutput` from content, so an output that was cut can't be cut again, where the spec had only said the loop applies the cap once.

What the review changed. Three reviewers read the phase, one for the loop, one for the domain's invariants, and one for whether each scenario's clauses are asserted. They found no defect of high severity, and the third found the pattern that phase 3 had, a test that asserts the stop reason and stops there, in none of the fourteen rows but one clause of E12.

- **No provider attempt is made when the run's time has gone.** The attempt was still made, with a deadline of zero. Spec section 1 says a run makes no provider call after the condition that should have stopped it, the tool path answers the same state `not_run`, and an adapter that read zero as no timeout would have run unbounded. The run stops with `timeout` before the attempt is announced.
- **A failed attempt's timing reaches the observer.** `ProviderCallFailed` carried none, so the summary's provider latencies counted attempts that no event reported, and the observer phase 4 builds couldn't have reconciled the wide event with the steps before it. `Run::failed_attempt` gives the timing back in the milliseconds the totals count, so the event can't differ from them.
- **A call's start and latency are its own.** The clock was read before the observer was told of the call, so both counted the observer's time. It's read a second time after, and the call's deadline is what was left less what the observer took.
- **A server's name and version are one value, and the lifetime is held with the servers.** Two lists paired by their order and a lifetime beside them could state two servers with one version, or a lifetime for a run with no servers. `McpServers` is never empty, and `RunContext` holds one or none.
- **A model's provider is read from its API.** `ModelRef` held both, and eight of the twelve pairs were ones no adapter produces.

Dismissed, each with its reason: `ToolCallFinished` can hold `not_run` by its type, but the loop returns before any event for such a call; only a failure another attempt could answer is held to point A, which is what spec section 1 says; an executor that keeps less than its call asked still reports a true size; the jitter stays within its bound for every salt; and a run whose provider reported no cache counts is priced on its whole input, once. The pairing of a digest with its prompt is held for a caller of the loop and not for a caller of `Run::start`, and the documents now say so.

The build took about 2.24 million tokens and the fix pass 0.33 million. The review took 0.45 million against a budget of 0.22 to 0.34 million, so it ran a third over: the domain reviewer read more than its lens needed. Phase 4's reviewers are given the files to read, not the diff.

## 2026-09-29 The transcript document is part of the public contract

Decided by the human. The public contract was the config schema, the outcome JSON and the telemetry registry, and the transcript was left out of it while spec section 1 called it what a grader or a composing framework reads. It's the fourth part now, and a breaking change to it needs a major bump like a breaking change to any of the other three.

Nothing had to be built for this. The transcript document got its `schema_version` on 2026-09-21, and its fixture joined the changelog gate's watched list with the documents crate, so it was already held the way the outcome is. What was missing was the sentence that says a reader may rely on it.

## 2026-09-29 Phase 4, closed

Six items landed, each with its gates green and its own commit: the documents crate with the transcript writer, the scripted provider, the observer with its file exporter and reader, the wide-event mapping with the summary's totals grouped, the built-in tools, and the library. A run built from a config string with the scripted provider and the built-in tools leaves its spans, its log records, its wide event and its transcript. `lablet-model`, `lablet-policy` and `lablet-run` hold 100% of lines and regions, and the one mutant that survives is the equivalent one already recorded.

The items landed in another order than the plan's. The plan put the library before the built-in tools, and the tools were built first, so `build` was written once with every adapter it composes.

What the builder decided on the way:

- **Spans are built as finished data.** The loop measures when a call started and how long it took, and the event that ends a span carries both. So the observer makes each span whole when it ends and never opens one on a clock of its own, and it needs no tracer.
- **The wide event is made when the observer is flushed.** It counts the records the exporter lost, and the count is whole only once everything else of the run has been exported. It goes in an export of its own, so it's the last line of its run.
- **The observer fills the wide event in one match over a generated enum.** The registry crate gains an enum of the wide event's keys and one of its template keys. The match has no wildcard arm, so a key the registry gains doesn't build until the observer says what it holds.
- **`lablet.mcp.servers` is required only of a run that has servers.** It was always required, and a run without servers would have written an empty list beside no lifetime.
- **`prost-derive` is a wrapper of the banned `anyhow`.** `deny.toml` lists it, which is the procedure the file documents for a dependency that brings a banned crate and can't be built without it.
- **A killed group that hasn't gone in 5 seconds ends the call as `failed`.** `timeout` says the work has stopped, and a call that can't say so doesn't say `timeout`. `nix` is taken for the signal, since unsafe code is forbidden.
- **A tool's description leaves out the root's path.** Otherwise `lablet.tools.digest` would differ between two machines that offer the same tools.
- **The resource leaves the run's context.** `telemetry.resource` goes to the observer when it's built, where the resource is fixed. The field in `RunContext` was read by nothing, and a second copy could have differed from the first.
- **A run id can be made from a ULID's value, and a checked task put under a system prompt.** `RunId::ulid` and `Prompts::with_system` leave `Lablet::run` no error arm that nothing could reach. The domain still reads no clock and draws no random number: the value is the caller's to make.
- **`build_observed` takes a caller's observers.** Spec section 7 offered no way to register the observer that section 5's fan-out assumes.
- **`build` refuses a stated setting the provider can't apply.** The plan gave this to phase 5. The resolved config needed the same table to leave out the defaults a provider can't apply, so the refusal came with it.
- **A relative path in a config starts at the working directory.** The spec didn't say.

What the review changed. Four reviewers read the phase, one each for the built-in tools, the library, the telemetry, and the documents with the scripted provider and the conformance cases. Each was given the files to read and not the diff. They found one defect of high severity.

- **The exit code survives every cut.** `bash` wrote its exit code as the last line of its output, and the `head` and `preview` cuts send only the start. With the defaults, a command that wrote more than the cap reached the model with no exit code, so the model couldn't tell whether a long test run had passed. `KeptOutput` gains a closing line, a short line about the whole output that every cut sends last. No more than 256 bytes of it are kept, so an executor can't use it to get round the cap.
- **The wait for a killed group is held by a test.** Nothing failed when the wait was removed, because the shell's own exit usually took long enough. The wait takes its probe as a parameter and is tested under a paused clock, the branch that answers `failed` included.
- **Two configs with the same effect share a digest.** `tools.allow` and `tools.deny` were hashed in the order they were written, and `tools.output_preview_bytes` was hashed whatever the cut. **Partly superseded on 2026-09-30 by "Six decisions before phase 5":** the promise covers the settings the resolved config marks as not applied, and any other setting changes the digest whether or not it has an effect. Both fixes still hold.
- **A stated setting with no effect is refused.** `model.base_url` with the scripted provider, and a `tools.max_description_chars` other than the default, which phase 8 delivers.
- **The value of `model.api_key_env` is in no message.** A key pasted there by mistake was echoed by the refusal. The value must be the name of a variable, and every refusal names the key alone. **Partly superseded on 2026-09-30 by "Six decisions before phase 5":** a message shows the value when it's written in capital letters, digits and `_`, as environment variables conventionally are. That the value must be the name of a variable, and that a message shows no other value, still hold.
- **A telemetry write that failed partway doesn't corrupt the next line,** and a file that was moved between two runs isn't written to: the file is opened at the start of each run.
- **A transcript is written whole or not at all, into a directory that's made.** A path with `{run_id}` in a directory lost every transcript, since no such directory can exist before the run, and a write that failed left a partial file where the last transcript was.
- **Two conformance cases hold what they name.** The output case asserted sizes and not text, and the observer case compared the wide event's token counts with the spans', which the same observer wrote.

Two claims were narrowed and nothing was built for them. A command can't read lablet's environment with `env`, but it runs as the user lablet runs as and can read it from the process table. And a process that leaves its group, as one started with `setsid` does, is beyond the kill. Both are the environment's to prevent, as spec section 6 says of `bash` as a whole.

Dismissed, each with its reason. A panic in a caller's observer is a defect in the caller's code, and an observer returns nothing, so it has no way to fail a run. `lablet.result.text` is an empty text for a run that has none, which is what the outcome writes, and `lablet.result.text_bytes` agrees. An executor that answers shared calls one at a time is slow and not wrong, since the loop owns concurrency. And nothing in lablet reads an outcome document, so a missing key that the read accepts is met by tests only.

Left open:

- **Where lablet is a container's first process with no init, a timed-out command that had children reports `failed` after 5 seconds.** The children become lablet's to take from the process table, and until they're taken the group still answers. It's unverified, since no Linux host was at hand. `docker run --init` avoids it, and taking the group's members in the wait would fix it.
- **A flush that times out would leak into the next run's count of lost records.** It can't happen with the file exporter alone. Phase 6, which adds the network exporter, holds the count for each run.
- **The library has no way to cancel a run.** `build` wires a cancellation that never fires, and phase 5's CLI needs a way in.
- **Phase 5's `main.rs` should consider clearing the process's `PR_SET_DUMPABLE` flag on Linux,** which stops a command reading lablet's environment from `/proc`. It's a setting of the whole process, so it's the binary's to make and not the library's. Decided on 2026-09-30 in "Six decisions before phase 5": it clears it.

The build took about 3.02 million tokens and the fix pass 0.47 million. The review took 0.60 million against a budget of 0.30 to 0.45 million, so it ran a third over for the second phase in a row. Giving reviewers the files held each one to its area, and the two that read the largest areas still took 0.18 million each.

## 2026-09-29 Mutation testing runs daily, and its floor is exact

Decided by the human. Mutation testing leaves the push path for a workflow of its own, `Floors`, which runs every day and on demand. Its floor changes from 80% of mutants caught to an exact one: every viable mutant of `lablet-model`, `lablet-policy` and `lablet-run` is caught, except the ones `EQUIVALENT_MUTANTS` in `xtask/src/floors.rs` names, each with the reason no test can catch it. The same workflow holds the three crates to every branch.

- **Line and region coverage stay on every push.** The CI leg takes under a minute, which a push can wait for.
- **Mutation testing runs daily.** It took 10 to 14 minutes of every push whatever the push changed, and its time grows with the code: 9 minutes at 338 mutants, 13 at 482. A scheduled run skips it when nothing that feeds the three crates has changed since the workflow's last green run on `main`: their sources, the manifests, the lockfile and cargo's configuration, the toolchain and tool pins, xtask, and the workflow itself. A run on demand always tests.
- **The floor is exact, because no percentage is right.** At 100% a percentage fails `lablet-run` on its one equivalent mutant, since 60 of 61 is 98.4%, and anything lower leaves room on a larger crate that real gaps hide in. Whether a mutant is equivalent still isn't decidable by a tool, so each survivor is judged once by a person and written down. An entry names the package, the file and what the mutant does, never the line or column, which move with every edit. It excuses one mutant that the tests ran against and missed, so a timeout is never excused. The named mutants are still generated and tested, so the list can't rot: a full run fails on an entry that matches no missed mutant, because the code it named is gone or a test now catches it. The list starts with one entry, `RunObserver::trace_context` replaced by `None`, which is already its default body, and 367 of 368 viable mutants are caught.
- **Branch coverage is held to 100%, in the daily run only.** The compiler measures branches only on a nightly toolchain, so `cargo xtask coverage --branch` runs on a nightly pinned by date in `xtask/src/coverage.rs`, and the workspace stays on its pinned stable toolchain. The workflow installs the same nightly, and a test holds the two in step. On 2026-09-21 branch coverage was measured once and left out, to be measured again when the loop grew. Measured again on 2026-09-29, every branch of the three crates is taken, 74 in `lablet-model`, 22 in `lablet-policy` and 46 in `lablet-run`, so the floor starts where the code already is. The job takes about a minute and runs every day, changed or not.
- **A builder gets a quick answer.** `cargo xtask mutants --changed` tests only the mutants in what differs from the merge-base with `origin/main`: what's committed on the branch, in the index and in the working tree, and untracked files, since a new module is untracked until it's committed. Every mutant it tests is caught or named. It can't say whether the list is current or whether a crate was measured at all, so the full run judges both.

The costs. A gap in the mutation floor can sit on `main` for up to a day before the scheduled run finds it, and for as long as the workflow stays disabled if GitHub turns it off after 60 days without activity, which is why the latest run's date is read too (see Gates in `contributing/README.md`). A failed scheduled run isn't attached to a push, so no push waits for it and no builder's gate turns red: the latest `Floors` run is read before work starts and before a phase closes, which is a convention and not a gate. A failed mutation run leaves the whole workflow run failed, so the next day's run doesn't take it for the last green one and skip the mutants that were missed.

What the builder decided on the way:

- **A listing that fails counts as no green run.** When `gh` can't list the workflow's runs, the scheduled run tests the mutants, since a run too many costs minutes and a run too few hides a gap.
- **Without `origin/main`, the scoped run starts from the merge-base with `main`,** as the changelog gate does.
- **Nothing but the list excuses a mutant.** `cargo xtask mutants` passes cargo-mutants `--no-config`, so a `.cargo/mutants.toml` can't stop a mutant from being made, and a production file of a floor crate may not carry `#[mutants::skip]`. Either would take a survivor out of the run without a name or a reason, which is the gap an exact floor exists to close.
- **A mutant with no verdict counts as not caught.** A mutant is excused from the count only when it didn't build. One whose run ended any other way, as `Failure` does when cargo is killed, or with a summary a later cargo-mutants adds, fails the floor and is listed with its summary, since nothing showed that a test would have failed.
- **The nightly runs through `rustup run`, in a target directory of its own.** A nested `cargo +toolchain` under `cargo xtask` can reach the real cargo rather than the `rustup` proxy, and a directory of its own keeps the nightly and stable builds from rebuilding each other's artefacts.

It supersedes in part three earlier decisions: the 2026-09-19 bullet on the CI matrix, the 2026-09-21 paragraph that no floor is 100% mutants, and the 2026-09-21 decision to leave branch coverage out.

## 2026-09-29 Phase 4, signed off

Decided by the human, at the review of phase 4.

- **The rule on serde says where the published shapes live.** The architecture rule said the domain model is the one serde form of the conversation, which stopped being true when the transcript and outcome documents moved to `lablet-documents`. It says now that the domain and the application hold in-memory models and that the published document shapes live in `lablet-documents`. Which crates may use `serde` is unchanged.
- **The cap on invalid turns is confirmed.** The cap was decided as three invalid calls in a row and built as three invalid turns in a row, for the reasons the 2026-09-28 entry "What the review of the plan changed" gives. The human confirmed turns.
- **A review's budget depends on what the phase adds.** About 20 percent of the build's token cost for a phase that adds a security boundary or a public contract, and 10 to 15 percent otherwise. The reviews of phases 3b and 4 each ran a third over the old ceiling, and phase 4's found ten real defects, one of them of high severity, so the budget was too small rather than the reviews too large.
- **The remaining speed-up proposals wait.** Building adapters side by side in separate working trees, splitting the spec, letting builders edit the spec sections their item covers, and feeding review findings into `contributing/reviews.md` were proposed and deferred.

## 2026-09-30 What the review of the phase 4 fixes changed

Two reviewers read the fix pass, one for the closing line and `bash` and one for the writers, the config and the conformance cases, and a sceptic tried to refute each finding. Seven findings held, none of high severity, and five changes answer them. The review found no fault in the other fixes: the refusals of `model.base_url` with the scripted provider and of `tools.max_description_chars`, the key variable's value kept out of every message, the telemetry file opened at the start of each run, and the two conformance cases.

- **The closing line isn't counted against the cap.** An output whose text fitted the cap was cut when its exit code took it over: it was sent whole with a line that said nothing or 0 bytes were left out, and `lablet.tool_calls.truncated` counted a call that lost nothing. The closing line is sent on top of the cap now, as the line that says what was left out is. Under the default `preview`, an output in that band of a few bytes is sent whole where it was previewed.
- **The wait for a killed group is held by a test of the group itself.** The tests drove the function the wait was moved to, so `Group::went` could answer without waiting and nothing failed. A test now starts a real process in a group of its own and waits for it on the paused clock, before the kill and after it.
- **Telemetry to standard error begins a line of its own after a failed write.** Only the file destination remembered that a write had left part of a line, so standard error on a full disk or a non-blocking pipe joined the next export to the fragment. Both destinations write through one function. Standard error is one stream across runs, so its flag isn't reset when a run starts.
- **Writing the transcript through a temporary file costs nothing that writing it in place didn't.** Its name, `.lablet-<pid>-<n>.tmp`, holds nothing of the target's, so a name near the file system's limit is still writable. It's made with `create_new`, so no link at the name is followed and two writers can't share one file, and a name that's taken is passed over, up to 100 of them. It takes the permissions of the file the path held, so a private transcript stays private. It's synced before the rename, so a crash can't leave the path naming text that never reached the disk, and an error the file system would report only at close is seen. A link at the path is still replaced.
- **A setting that nothing applies is left out of the resolved config.** `tools.output_cut` and `tools.output_preview_bytes` do nothing without `tools.max_output_bytes`, the MCP settings nothing without a server, and the `root`, `timeout` and `env` of `tools.builtin` nothing when no built-in tool is enabled, and each still changed the digest, against spec section 7's rule that configs with the same effect share one. No config this version builds has an MCP server, so every digest changes, and the changelog says so. `model.api_key_env` with the scripted provider stays in: the spec allows it, and the variable's name is digested for every provider.

What the builder decided on the way:

- **A link at the transcript's path gives the new file the permissions of the file it led to.** That's what reading the path showed, and a link's own permissions mean nothing.
- **The temporary file is made no more open than the file it replaces,** and its mode is then set to that file's exactly. So nobody the old file kept out can open it while it's written, which setting the mode after the file was made would have allowed for the moment between the two.
- **The resolved built-in section is a type of its own.** `ResolvedBuiltin` holds its settings as `Applied`, as `ResolvedModel` does, and `Builtin` is left as the config reads it.

## 2026-09-30 A push tests the mutants of what it changes

Decided by the human. `pre-push` ends with `cargo xtask mutants --changed`, so a push that changes `lablet-model`, `lablet-policy` or `lablet-run` is refused until every mutant in what it changes is caught or named in `EQUIVALENT_MUTANTS`. A push fast-forwards `main`, so what differs from `origin/main` at the push is what the push lands.

It costs little because it tests little. Over the 17 pushes of phases 3b and 4, seven changed no floor crate and would have passed the step at once, and the rest had 4 to 74 mutants in what they changed, where the full run has 482. At the full run's rate of about a second and a half a mutant, that's between 20 seconds and 2 minutes.

What it leaves to the daily run: a test that was weakened or removed, since only changed production code is mutated, and an entry in the list that went stale. `ci` runs the same steps, and on `main` the merge-base is the pushed commit, so there it finds nothing to test.

## 2026-09-30 A command inherits lablet's environment, less lablet's secrets

Decided by the human. A survey of eight agent harnesses found that every one passes its environment to the commands its shell tool runs. None of them keeps its model's API key out by default.

- **`bash` inherits lablet's environment, less the variables lablet reads its own secrets from.** What a framework prepared for its commands, such as `JAVA_HOME`, a proxy or a virtualenv, has to reach them, and the short list kept it out. Today the one such variable is the one `model.api_key_env` names, or `ANTHROPIC_API_KEY` for `anthropic` when it names none. The composition root builds the withheld set in one function and hands it to the executor, so a variable a later phase reads a secret from is one more entry there. `tools.builtin.env` is added on top and may name a withheld variable, which passes it on: a task that needs the key gets it only when the config says so. Withholding the key is a deliberate difference from every harness surveyed, taken for safety, as the run timeout is.
- **lablet's own secret values are cut out of every tool result.** Withholding a variable doesn't keep a key from a command, which can read it from a file or from lablet's environment through the process table. So each value is replaced by `[secret withheld]`, which names nothing of it, before the result reaches the model, the transcript or telemetry. Only a value of at least 16 bytes is cut, the length the survey proposed: a key is far longer, and a shorter value could be ordinary text, which a cut would mangle, and where the cuts fell would show the model what the value is. The cut comes before the `KeptOutput`, so the sizes and the output cap count what's sent, and a value that two pieces of a command's output split is cut whole.
- **The framework's other secrets are the framework's.** A `GITHUB_TOKEN` a framework sets reaches commands, and no pattern of names is filtered. The framework chose the environment it started lablet in, so it's the one to leave out what its commands shouldn't have, and a pattern would both miss secrets and withhold what isn't one.

Stdio MCP servers aren't decided. Phase 8 decides between the short list and this rule, after a probe of the primary reference that the human runs.

The costs. A framework's other secrets reach commands. The cut takes only the exact values lablet knows: a copy that a command encodes or changes isn't cut, nor is a secret shorter than 16 bytes.

What the builder decided on the way:

- **The cut is in the domain.** `Secrets` and `RedactedOutput` are in `lablet-model`, in front of `KeptOutput`. The cut needs only values and text, so a plain test drives it, and every executor makes it with the same words, as `ToolResultContent::omitted` is worded once. Phase 8's MCP executor needs it too, and one adapter can't use the code of another. It's held to the model's floors, which suits a boundary of this kind. The names of the variables stay out of the domain: which ones are withheld is the composition root's to say and the adapter's to apply.
- **The values are read only when a built-in tool is enabled.** Nothing else holds a key's value yet, and reading it for an executor that doesn't exist would hold it in lablet's memory for nothing. It's in no error, no `Debug` form and no resolved config: `Secrets` shows only how many values it holds.
- **Whitespace around a value is no part of it.** A variable set from a file often ends with a newline that no command prints beside the key.
- **Where two values begin at one place the longer is cut, and copies that overlap are one cut,** as a cut of the whole text would. A value that begins inside a cut stretches it to its own end, since a copy that began a byte into the first would otherwise show all of itself but that byte. Adjacent copies are two cuts. What may begin a value at the end of a piece is held back, and nothing else, so the executor holds less than the longest value beyond what it held before.
- **A value that isn't UTF-8 is read as the tools read output,** with U+FFFD for what's no character, so it matches the output of the same bytes.
- **`bash`'s description no longer says the environment holds a short list.** The environment is the ordinary one now, and the marker says what it stands for. The digest of the tool specs changes.
- **A `ToolError`'s message isn't cut.** It holds lablet's own words, the model's arguments and what the system said, and never what a tool wrote or read.

## 2026-09-30 Six decisions before phase 5

Decided by the human. An audit of phases 0 to 4, made before phase 5 starts, found six things that were the human's to settle, and the human took the builder's recommendation on each.

- **A message names the key's variable when it's written as a variable's name is.** The phase 4 review kept `model.api_key_env`'s value out of every message, in case a key was pasted there, and scenario C4 wants the message for an unset variable to name it. A message now shows the value when it's written in capital letters, digits and `_`, the way environment variables are conventionally written and few key formats are, so `ANTHROPIC_API_KEY` is named. Any other value is "the variable it names," as before, and a value that isn't a variable's name is still refused without being shown. One function in the composition root holds the rule, and every message that names the variable goes through it.
- **Phase 5's binary clears `PR_SET_DUMPABLE` on Linux.** A process with the flag cleared keeps its `/proc` entries from other processes of the same user, so a command can't read lablet's environment there and use the key or send it on, and no core dump that would hold the key is written. It's a setting of the whole process, so it's the binary's to make and not the library's, whose caller owns the process. That makes it the one thing lablet does in `main.rs` alone, and quality-bar item 6 says so. It closes lablet's own entries in `/proc` and a same-user `ptrace`, and no more: on macOS a command can still read lablet's environment through the process table, and on any system it can read the key wherever else it's held, in a file or in the environment of the process that started lablet. So cutting the key's value out of tool results still matters. `apps/lablet` takes a Linux-only `nix` dependency naming its `process` feature, which holds `prctl`, since unsafe code is forbidden; the `signal` feature the workspace pins turns it on already.
- **The digest promise is narrowed.** Spec section 7 said two configs with the same effect share a digest. The answer to the review of the phase 4 fixes took the settings it found that nothing applies out of the resolved config, and more of that kind remain: the retry settings when `run.max_retries` is 0, and `prompt.skills_mode` with no skills. The class has no clear end, since whether a setting has an effect can turn on another setting's value. So the promise covers the settings the resolved config marks as not applied, which the spec lists, and any other setting changes the digest whether or not it has an effect. Erring that way is safe: a composer can join two groups that the digest split, and no composer could split one that it joined.
- **The rule on attribute names is a review convention.** `contributing/README.md` said not to write an attribute name as a string literal in an adapter, and spec section 6 called one a lint failure, but no gate checks it. Quality-bar item 10 wants every rule in `contributing/` held by a gate or labelled a convention, so it's labelled, and the spec says so. A gate would have to tell an attribute name from every other string an adapter writes, and the code keeps to the rule today. The human approved this one edit to `contributing/`.
- **The changes that landed after the phase 4 sign-off are accepted.** `3727ebf`, the answer to the review of the phase 4 fixes, landed after the human signed phase 4 off, and it changed behaviour: the closing line isn't counted against the cap, and every config digest changed. The human has seen them, with the three decisions the builder made on the way, and accepts them.
- **The latest `Floors` run is read before work starts and before a phase closes, by the delivery rules.** `contributing/README.md` asked for it as a review convention, and the audit found that nothing had read it before phase 5 was planned. `CLAUDE.md` now says it beside the rule on a red `main`: the run should be from the last day or two, and a red one is fixed before anything else lands.

Phase 5's plan gains what the audit found it has to design for, each item with the decision or scenario it comes from. Five of them decide something, and are the builder's:

- **With a telemetry file path of `-`, the diagnostic log is off unless `RUST_LOG` is set.** The 2026-09-28 decision that `-` implies `--quiet` gave as its reason that OTLP and diagnostics don't share standard error, and spec section 7 kept only the summary line off it. A warning from the library would land between two OTLP lines and break a reader of the stream. A `RUST_LOG` that's set asks for the log, and whoever set it can route it.
- **Cancellation stops calls in flight, within the time a container's stop allows.** Once phase 5 wires `SIGTERM`, `docker stop` sends `SIGKILL` 10 seconds later by default, before a `bash` call of up to 120 seconds or a provider call of up to 10 minutes returns, so the run's outcome, transcript and wide event would go with the process. The 2026-09-23 entry kept stopping early open for want of a status for a call that never ran, and `NotRun` is that status now.
- **A `--run-id` that isn't one component of a path is refused before the run, with exit 1,** and the library refuses such an id in a run request too, so the two agree. An id that holds a `/` or a NUL loses the run's telemetry file and its `{run_id}` transcript with only a warning, and `--run-id` makes the id user input.
- **A run with no response writes a transcript that holds its task prompt.** The 2026-09-20 known limit waited for the ATIF export, and phase 5 shows the gap sooner: a CLI run whose key is rejected would leave a transcript that doesn't say what was asked. The prompt goes under a key that's added, so `schema_version` stays 1.
- **`ConfigDigest` becomes a newtype in the change that moves the digest.** The 2026-09-20 entry waited for the digest's producer, which phase 4 wrote, and phase 5 changes how the digest is taken, so the type changes with it.

## 2026-09-30 What the audit of the tests changed

An audit of the suite at `b56d30d` read about 1,050 of its 1,240 tests. It found them good, and very good in the three floor crates, whose floors had already removed what a mutant can find. What was left was what no mutant proposes: names that promised a clause nothing checked, contracts the spec states that no test put the code in a position to break, fakes more forgiving than their ports, and gate decisions in xtask that no test reached. Four of its six recommendations were taken.

Decided by the human:

- **An assertion that a call succeeded or failed, and nothing more, is refused.** clippy's `assertions_on_result_states` is on in both workspaces. It found 25 sites, all in tests; the four that expected an error now say which.
- **The gaps where a defect passed are closed.** Among them: cancellation meeting the turn cap, the token budget and a zero timeout in one turn, where moving or deleting the loop's poll after the tool phase passed every test; a tool's source reaching the record, which every helper had fixed to `Builtin`; the first bad rate being the one reported; and tool spans exported in the order their calls ended.
- **Every gate's decision is a function a test calls.** The seven xtask entry points that run a tool and decide pass or fail had their decisions moved out beside `changelog::resolve_base`, so a verdict mapped the wrong way, or an exit code accepted that shouldn't be, fails a test. xtask isn't a floor crate, and a one-off mutation run over it caught 463 of 539 viable mutants; the survivors that matter are left open below.
- **Four questions join `contributing/reviews.md`,** as items 29 to 32, and the items after them are renumbered. This lifts, for these four, the deferral of feeding review findings into that page, which the entry "Phase 4, signed off" records.
- **No test reads `product/spec.md`.** The audit proposed reading spec §7's config block from the spec, so the defaults test couldn't drift from it. The spec is prose and can drift, so a test's fixture is its source, the spec quotes the fixture, and a quote that drifted is a documentation defect for a review to find.

What the builders decided on the way:

- **A call that reaches its deadline has timed out.** The port's documents said a call past its limit times out, and the loop's fake timed a call out only past it, while the built-in executor kills a command at its deadline. Every limit is met when it's reached, as the entry of 2026-09-19 says, so the fake and the documents say that now.
- **The observer's shutdown has one bound in all.** It waited five seconds for each of its three queues in turn, so a stuck exporter held it for fifteen. Shutdown runs on a thread of its own and the caller waits the bound, five seconds by default and set with `shutdown_timeout`. What's still being exported when the bound is reached is left to finish, since nothing can take it back.
- **A diff a gate runs never writes the index.** `git diff` against a commit refreshes the index's stat data, which `--no-optional-locks` doesn't stop, so the scoped mutation run and the changelog gate rewrote the index of the repository they only read. Both pass `-c diff.autoRefreshIndex=false`, and a test that changes one file's timestamp holds them to it.
- **An executor's shared calls each get their own answer, and needn't overlap.** The conformance case was named for shared calls answered together, and it passed an executor that answered one at a time. The loop decides what runs together, so the case is named for what an executor owes it.

Left open:

- **Mutants xtask's tests don't catch.** Among them: the exit code of a failed gate (`main.rs`), a failure to remove the last report being ignored, a coverage run that judges no crate, and the refusal of a tool that isn't pinned. Each has a test that would catch it, named in the one-off run's report; none is a floor.
- **Every-variant lists written by hand, and helpers copied between crates,** the audit's other two recommendations, wait.

## 2026-09-30 The tests' copies become one, and their lists can't miss a variant

Decided by the human: the rest of what the audit of the tests left open is closed before phase 5. That's the xtask mutants that matter, the helpers copied between crates, and the every-variant lists written by hand.

- **A failed gate can't exit as a success.** A mutation run over xtask left 76 survivors, and 50 of them could turn a gate's answer or the command it runs. Each has a test now: the exit code of a gate's result, the removal of the last report, a coverage run that judged no floor, the pinned-tool refusal, and the git and cargo glue, tested in scratch repositories whose settings change what git does by default. A test runs the binary itself and reads its exit code. The 26 left change only what's printed, never whether a gate passes. xtask still has no mutation floor.
- **A report that judged no floor fails.** `floors::conclude` passed a run in which no floor crate was found at all, so a coverage run that measured nothing passed. It fails now.
- **The adapters' tests share one crate, `lablet-test-support`,** in `lablet/tests/test-support`. It holds one `Scratch` in place of seven, the clock and the never-cancelled port in place of four copies each, and a run builder in place of four hand-built runs. The directory is `test-support` and not `support`, because a directory `foo/bar/` is package `lablet-bar` and a fourth exception to that rule would change a rule. A scratch directory is named for its test, the process and a count, so no two tests share one. The three scripts named `FAILS_CALLS_ENDS` differ on purpose, and each says why.
- **`lablet-run` keeps its own fakes.** A dev-dependency of `lablet-run` on a crate that depends on `lablet-run` would compile it twice, and its types would stop matching. For the same reason `provider-fake`'s own unit tests can't use the support crate, which depends on it.
- **An every-variant list can't miss a variant.** `every_variant!`, beside `display_as_str!` in `lablet-model`, declares a closed enum's `ALL` with a match that has an arm for each listed variant and no other, so a variant left out doesn't compile, and neither does one listed twice. It's a macro and not a dependency, since the rule is a few lines. An enum with a variant that holds data gets a list named for what it holds instead: `FinishReason::KNOWN`, `ToolCallStatus::NOTHING_RAN` and `OutputCut::BY_CAP`. The registry's templates give each generated enum an `ALL` whose length is the registry's count of members. The tests that hold two artefacts' spellings to each other run over these lists, in both directions, so a spelling that differs between the outcome document and the registry fails, where it passed before. Phase 5's `lablet schema` can list the values from them too.

## 2026-09-30 Phase 5, closed

Phase 5 built the CLI and the config surface in three streams at a time, as the human asked, each in a working tree of its own and each owning its own files, merged by the orchestrator. The loop's cancellation, the library's config surface and the CLI's first parts landed first; the CLI's wiring and xtask's error type second; the review's fixes last. Every scenario of the phase passes through the binary, the phase's acceptance line holds (`lablet init --provider fake` then `lablet run` completes with no edits, writes an OTLP/JSON file and prints the outcome), and the CLI and the library's doctest give the same outcome for one config. `lablet-model`, `lablet-policy` and `lablet-run` hold every line, region and branch, and the daily run holds the exact mutation floor.

What the builders decided on the way:

- **A dropped provider attempt has an event of its own, `ProviderCallCancelled`, not an error kind.** No adapter could produce such a kind and the retry policy would have to read it. It counts as a failed attempt, so the chat spans still number the turns and the retries and one more.
- **A tool call that started and was stopped is `cancelled`,** a status of its own and a value of `lablet.tool.status`, since it ran; a call whose turn came after the cancellation is `not_run`. A stopped call counts in the totals and the errors, because it has a span.
- **Finished work is kept.** The loop polls the work before the cancellation, so a result that came as the run was cancelled is recorded, and the run stops at the next point.
- **`${VAR}` is substituted into the settings that hold text, when a config is checked or built, and not into the tree.** The digest and every message see the config as written, even after a caller changes a field. A variable in a setting that isn't text is refused by that setting, since the digest is taken before substitution, and `$${` is a literal `${`.
- **The schema is derived with `schemars`,** already a dependency, and `lablet/schema.json` is held to it by a test.
- **`init [dir]` takes a directory,** as `cargo init` does, and writes nothing when any of its files is there already.
- **The outcome is one line of compact JSON,** so the last line of standard output is always the outcome.
- **A usage error exits 1, not clap's 2,** because 2 says a run started. Help and version exit 0.
- **`check --resolved` prints the resolved config and not the tools,** so each run of the command prints one kind of document, and the YAML reads back as a config with the same digest.
- **Signals are caught before the build,** so a stop during the build gives a `cancelled` run with its outcome and record. A second signal doesn't force an exit, since the stop is bounded already.
- **`--quiet` keeps the `provider:` line, and a telemetry path of `-` drops it,** because it's an error and not the summary.
- **xtask's error type tells a gate's verdict from a failure to run the gate.** A gate's own tool exiting non-zero is its verdict; a helper that fails, such as git or the toolchain installer, is an error, which names its command and keeps its cause.

What the review changed. Four reviewers read the phase, one for the loop's cancellation, one for the config surface and what it keeps secret, one for the CLI, and one for xtask's gates, and a sceptic tried to refute each finding. The cancellation reviewer found nothing to fix. Ten findings held, none of high severity, and nine were fixed:

- **Nothing substituted from the environment reaches what a run shows.** The fake provider's script was named by its substituted path, which a script that ran out put in the outcome's error, the `provider:` line and telemetry; it's named as the config writes it.
- **Whether telemetry goes to standard error is decided once.** The CLI read it from the config as written and the library after substitution, so `path: ${T}` with T=`-` put the summary among the OTLP lines. The library decides, and the CLI asks it before it installs the log.
- **The root check follows `..` and a link at the transcript's path.** A `..` after a directory that doesn't exist yet leads back where the file is made, and a link at the transcript's path is replaced by the writer rather than followed, so the check resolves its directory. **Partly superseded on 2026-09-30 by "What the assessment after phase 5 settled":** the check keeps a config from pointing lablet's own files into the root and keeps the confined file tools off them; it isn't a boundary against `bash`.
- **A run id over 128 bytes is refused,** by the library, the CLI and both writers, since a longer one can't name a file and the run's transcript and telemetry were lost.
- **The changelog gate lists content changes only.** The phase's index fix made it fail on a contract file whose timestamp moved and whose content didn't, until a `git status` refreshed the index. A failed read of the changelog at the base is an error, not an absent file, and the warning that no base resolved is printed off a terminal too.
- **`cargo xtask run` is lablet itself.** xtask replaces its process with cargo's, so lablet's exit code and its signals reach the caller; it had turned exit 2 into 1 and died on Ctrl-C before lablet finished cancelling.

Deferred: a refused setting inside an entry of `tools.mcp` names the server, not its own key, because serde reads an internally tagged enum's fields through a buffer. MCP's config is phase 8's, and phase 8's plan carries it.

Decided by the human at the end of the phase: **a closed standard output isn't a panic.** `main.rs` writes to standard output in one place with `writeln!`, and a write that fails is said on standard error, so `lablet schema | head -1` no longer exits 101. The contributing rule that named the one `print_stdout` says that now.

Left open: a telemetry file's path that's a link inside the root pointing outside it, and a dangling link, still pass the root check. `write_file` finishes its write on a blocking thread after a call is dropped, for about a millisecond, which the port's "a dropped call stops its work" doesn't quite hold. And `lablet run` with no prompt flag reads standard input, as the spec asks, even when that's a terminal.

The build took about 2.77 million tokens and the fix pass 0.5 million. The review took 1.87 million against a budget of 0.55 million, more than three times over: most of its sixteen agents built and ran the binary to confirm what they found. Phase 6's review has three reviewers, and its sceptics read the code before they build anything.

## 2026-09-30 What the assessment after phase 5 settled

Before phase 6, four readers read the three commits that closed phase 5, which no independent reviewer had read, the seams the later phases build on, the domain and the application, and a critic read their assessment. They built nothing. The domain and the application are sound: rules are held by types, the loop reads as spec §5 describes it, and no reader found a wrong answer in either. What they found is shape to improve, each in the phase that uses it, and a few sentences that promise more than the code does.

Decided by the human:

- **The one-writer rule for standard output is a review convention.** Since `main.rs` writes with `writeln!`, clippy's `print_stdout` no longer marks the one place, and a `disallowed-methods` lint for one call site isn't worth its upkeep.
- **The contributing guide names what the mutation floor can't see.** cargo-mutants mutates neither a function named `new` nor code a `macro_rules!` generates, so "every viable mutant caught" doesn't cover a checked constructor's own rules, which live in a function of another name or are held by unit tests.
- **What the later phases will meet is written into their plans, as questions where the owner decides.** Phase 6 bounds the per-run flush and flushes each destination on its own. Phase 7 maps finish reasons in each adapter and holds `Usage`'s cache addition by a constructor. Phase 7a splits `service.rs` in a move-only commit first, then measures the request whole after masking. Phase 8 asks how a run ends when its servers can't be started again, and builds a `RunService` for each run. Phase 10 designs the tools the loop answers itself together, with the schema check made outside the domain. Phase 11 decides the observer's signature before the release.

What changed now:

- **The root check's promise is stated as it is.** It keeps a config from pointing lablet's own files into the root and keeps the confined file tools off them. It isn't a boundary against `bash`, which isn't confined, and a dangling link, or a link made after `build`, passes it. The entry "Phase 5, closed" said it can't be passed.
- **A failed write to standard output goes unsaid when the telemetry is standard error's,** so no line but OTLP's lands there.
- **Stale documentation corrected:** `RunId`'s doc, which still said a run id is always a ULID, the `ToolExecutor` doc, which gave a tool announced mid-run to the next run where the spec gives it to the next `Lablet`, and `tools.builtin.env`'s description in the config's schema, which still described the short list `bash` started with.

What the critic corrected in the assessment, and why it's worth saying: it had put five items before phase 6 that phase 6 doesn't touch, rated the root check a security boundary it isn't, proposed a synchronous observer signature that bounds nothing, and proposed reshaping domain constructors around a tool's blind spot that unit tests already cover. Each is recorded where it belongs instead.

The pass took 1.31 million tokens, for reading alone.
