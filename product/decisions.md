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

## 2026-09-20 The small calls, and what phase 3 inherits

Decisions taken rather than deferred again, plus the constraints phase 3 should build on rather than rediscover.

- **`gen_ai.request.seed` is an `i64` in the model too.** It was an `Option<u64>`, and a seed above `i64::MAX` would have reached telemetry as some other number. Unlike the token counts it isn't bounded in practice, so the domain now holds the type the wire can carry.
- **`lablet.run.transcript_path` is the path rendered as UTF-8**, replacing anything that isn't. A path lablet chose from a config a person wrote is UTF-8 in practice, and refusing a run over the rendering of a telemetry attribute would be the wrong trade.
- **No string newtypes yet.** `config_digest`, `ModelRef::name`, `Endpoint::host` and an MCP server name stay `String`. `config_digest` has exactly one producer, in the composition root, which phase 6 writes; the others are pass-through values whose shape lablet doesn't get to decide. Revisit `ConfigDigest` when that producer exists.
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

**Not changed, and worth saying.** Cancellation is still polled at points A and B, and a turn's tool phase still runs to its end once it starts. Stopping between groups is now possible in one place, since the domain owns the phase. It needs a status for a call that never ran, because the transcript accepts all of a turn's outcomes or none, so it stays open. A turn's `task_complete` interception and point R's order are untouched.

## 2026-09-23 Two fixes from the review of the loop

- **Cancellation is polled before each retry**, after the backoff and before the next attempt. Spec section 1 says a run never makes a provider call after the condition that should have stopped it, and a retry is a provider call. Without the check, Ctrl-C during a run of provider failures paid for every retry left and slept through each backoff.
- **A filter name that nothing serves is refused**, as `ToolSetError::UnknownFilterName`. A misspelt `deny` entry left the tool it meant to deny on offer, so a safety control could fail open without a word, and a misspelt `allow` entry offered nothing. Names are checked against what the executors serve. So in explicit mode, `task_complete` in either list is refused as well, since the filter doesn't apply to it and naming it can only be a mistake.

## 2026-09-23 `Pending::answer` takes `Fn(ToolUse) -> Fut`

The phase 3a spike took the fallback recorded in "A run's states are types, and a turn's tool calls run together." With `answer: impl AsyncFn(&ToolUse) -> Answer`, the future `RunService::run` returns isn't `Send`. The compiler reports that `Send` is "not general enough": the future an async closure returns for a borrowed argument is higher-ranked over the borrow, and a generic function that holds several at once can't prove them `Send` for every lifetime. It fails the same way through `StreamExt::buffered` and through a hand-driven `FuturesOrdered`, so the join isn't the cause. With `answer: impl Fn(ToolUse) -> Fut` and `Fut: Future<Output = Answer>`, the future is `Send`, a spawned task on a multi-threaded runtime runs it, and a turn's groups overlap and finish in call order as designed. The price is one clone of each `ToolUse`, which the loop already made.

The same spike found what the testability rule needs from its tests. A future that returns `Pending` without waking its task is never polled again by `FuturesOrdered`, so a test that drives `answer` with `Waker::noop()` and such a future hangs rather than fails. The domain's test helper polls a bounded number of times and panics past it, and the futures a test hands in wake themselves before they return `Pending`, as any correct future does.

## 2026-09-24 A `task_complete` call whose arguments didn't parse completes nothing

It used to complete an explicit run with a null structured result. In explicit mode that result is what the run is for, so a run reporting `completed` with nothing in it looks like success to a grader and holds none. It was also the one call with bad arguments that the model never saw an error for. Now `Pending::calls` reads such a response as `Tools`: the `task_complete` call is answered `malformed_input`, the response's other calls run, and the model can call it again. The result counts as a tool error, so a model that keeps failing reaches `tool_errors_exhausted`, and `max_turns` bounds it too.

`ToolSet::source` answers for `task_complete` now, because it's offered: a real name with bad arguments is `malformed_input`, not `unknown`. When validating the argument against `run.completion_schema` lands, an argument that doesn't fit takes the same path.
