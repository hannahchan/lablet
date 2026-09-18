# Build plan

Twelve phases, each ending with something runnable and tested. [acceptance.md](acceptance.md) names the scenarios each phase must turn green; a phase is not done until they pass and the exit criteria below hold. An agent building lablet works one phase at a time, in order, and does not start a phase until the previous one is closed. Read [spec.md](spec.md), [quality-bar.md](quality-bar.md), and [../contributing/README.md](../contributing/README.md) first.

Sequencing rationale: the loop is proven against fakes before any real adapter exists; OpenTelemetry comes before the first real provider because a wrong span shape is more expensive to fix late than a wrong provider mapping; the telemetry contract is a phase of its own because the Weaver Rust templates are the riskiest piece of the project and must not stall the scaffold.

## Phase 0: scaffold

- The `lablet/` Cargo workspace (edition 2024, resolver 3, `rust-version`, `license = "MIT OR Apache-2.0"` in workspace package metadata) with every crate from spec §2 as an empty library or binary with a one-line doc comment. Root `.cargo/config.toml` with the `xtask` alias.
- `xtask/` at the repo root with `lint-layers`, `lint-manifests`, `fmt`, `clippy`, `deny`, `doc`, `test`, `coverage` and `mutants` (floors and crate list held as data in xtask), `changelog`, `pre-commit`, `pre-push`.
- `[workspace.dependencies]` with every third-party crate pinned to an exact version checked against crates.io that day: at least `async-trait`, `thiserror`, `serde`, `serde_json`, `serde_yaml`, `tokio`, `reqwest`, `rmcp`, `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `opentelemetry-semantic-conventions`, `tracing`, `tracing-subscriber`, `clap`, `humantime`, `ulid`, `sha2`, `schemars`, `wiremock`, `criterion`. Port `lint_layers.rs` from UsefulBytes, reducing the ring set and forbidden-dependency table to spec §2.
- `rust-toolchain.toml`, `mise.toml` pinning weaver, cargo-deny, cargo-llvm-cov, cargo-mutants; `scripts/install-hooks.sh`; workspace lints from contributing; `CHANGELOG.md`; GitHub Actions running `cargo xtask pre-push`.
- `lablet/README.md` placeholder and `lablet/docs/` directory.

Acceptance: `cargo xtask pre-push` passes on the empty workspace. A crate given a deliberately wrong-direction dependency fails `lint-layers`. A fresh clone passes the gates with one command after `mise install`.

## Phase 1: telemetry contract

Follow `product/research/weaver/README.md` and start from its `spike/` files; it verified everything below against Weaver v0.26.1.

- `mise.toml` pins weaver `v0.26.1` (`ubi:open-telemetry/weaver`). `cargo xtask weaver vendor` copies the `model/` trees of core semconv `v1.44.0` and `semantic-conventions-genai` at the pinned commit, plus the policies and markdown templates from `opentelemetry-weaver-packages`, under `lablet/telemetry/deps/` with `SOURCES` files.
- `lablet/telemetry/registry/` in v2 syntax with `manifest.yaml` using relative dependency paths; `lablet.*` attributes with justification notes; lablet-owned spans `lablet.invoke_agent`, `lablet.chat`, `lablet.execute_tool` and events `lablet.run` and the content record, referencing the `gen_ai.*`, `mcp.*`, and `error.type` keys from spec §1 and §6 with requirement levels; entity imports for the resource. Every attribute must have a source field in spec §3 or §5; if one does not, the spec is fixed first.
- `lablet/telemetry/policies/justification.rego` and `cargo xtask weaver check` running the lablet and vendored policies.
- Rust templates under `lablet/telemetry/templates/registry/rust/` (constants, enums, per-signal key lists; no builders) and `cargo xtask weaver generate` producing the `lablet-telemetry-registry` crate, `cargo fmt`, and `lablet/docs/telemetry/` from the vendored markdown templates, with a `--check` mode used as the generated-files gate.
- `lablet/telemetry/.weaver.toml` with the live-check finding filters from the spike; the `live-check` xtask command itself lands in phase 6.

Acceptance: `cargo xtask weaver check` passes. Regeneration is a no-op on a clean tree. Every attribute in spec §1 and §6 has a constant in the generated crate and the generated docs list it. A `lablet.*` attribute added without a justification note fails the policy. A weekly CI job runs `check` against the git URLs at the pinned refs to catch vendoring drift.

## Phase 2: domain

- `lablet-model`: every type in spec §3 with serde derives (including `RequestDefaults`, `Endpoint`, `TraceContext`, `McpCallMeta`), `Usage: Add` and `total()`, display impls for `StopReason` and `FinishReason`, constructors that validate (non-empty tool names, unique block ids). The transcript shape must map losslessly onto an ATIF v1.8 trajectory (one turn is one step, tool results joined by call id) so the phase 10 export needs no model change.
- `lablet-policy`: `StopPolicy::evaluate` at both stop points, `RetryPolicy::delay`, `Pricing::cost`.

Acceptance: unit tests cover each stop reason at the stop point that owns it (including token budget, truncated output, and context exhaustion), each completion mode, backoff growth, cap, and exhaustion, and cost arithmetic. Coverage and mutation floors met. No async code and no serde beyond derives in either crate.

## Phase 3: the loop

- `lablet-run`: ports, errors, `RunEvent`, `RunSummary` accumulation, `ToolSet` with the `task_complete` spec, `RunService` with the two-point stop evaluation, `task_complete` interception, and the trace-context handoff from observer to `ToolCall`.
- Hand-written fakes for every port, including a fake clock, in the crate's tests.

Acceptance: end-to-end tests with fakes prove natural and explicit completion, every stop reason, retry with backoff via the fake clock, consecutive tool error counting and reset, allow and deny filtering, duplicate tool name rejection, and that content fields are `None` when capture is off. The event stream for a scripted run is asserted exactly, and the `RunSummary` on `RunFinished` matches the per-step events it summarises. Scenarios L1 to L8, E1 to E8, and C8 pass.

## Phase 4: library and first traced run

- `provider-fake` with scripted completions, latency, and injected errors.
- `tools-builtin` with `bash`, `read_file`, `write_file`, and root escape rejection.
- `telemetry-jsonl` as a flat rendering of the trace with registry attribute names, join keys, and `schema_url` on every line, the wide event as the final line.
- `lablet-conformance` with the `RunObserver` cases (exactly one wide event per run, its numbers equal the sum of the per-step events, an unwritable destination does not change the outcome) run against `telemetry-jsonl`, and the `ToolExecutor` cases run against `tools-builtin`.
- `apps/lablet` as a library only: config types with defaults, `build` with `BuildError::Unsupported` for adapters from later phases, `Lablet` with multi-run and shutdown, `RunContext` construction, config digest over the resolved config, transcript output, the fan-out observer.
- Smoke test: `provider-fake` plus `tools-builtin` plus JSONL observer through `build` and `run`, asserting the event stream.

Acceptance: a doctest builds a `Lablet` from a config string, runs twice, and asserts both outcomes and the JSONL file. Scenarios O1, O4, O8, and C9 pass.

## Phase 5: CLI and config surface

- `main.rs` and the `clap` derive CLI: `init`, `run`, `check` (including `--resolved`), `schema`, `--set`, `${VAR}` substitution, prompt sources, diagnostic logging on stderr, the end-of-run summary line and `--quiet`, Ctrl-C into the `Cancellation` port, exit codes, and the error message contract from spec §7.
- `lablet/schema.json` checked in and covered by the changelog gate.

Acceptance: `lablet init --provider fake && lablet run --config lablet.yaml --prompt "..."` completes with no edits, writes a JSONL trace, and prints a `RunOutcome`. The CLI and the phase 4 doctest produce identical outcomes for the same config. Scenarios E9, T2, T4, C1 to C7, C10, and C11 pass.

## Phase 6: OpenTelemetry

- `telemetry-otel` built only on `telemetry-registry` constants, with unit tests asserting each span's name and required attributes against the generated key lists: root, chat, and tool spans with the spec §6 attributes, `lablet.turn` on children, the `gen_ai.client.operation.exception` log record and retry span event, the `lablet.run` wide-event log record carrying the root span's trace context, content log records behind `capture_content`, resource attributes, bounded shutdown.
- `cargo xtask weaver live-check`: starts `weaver registry live-check` without `--v2` on a random free port pair, runs the fake-provider config over OTLP gRPC, stops it through the admin endpoint, saves the report, fails on violations. Its CI job runs on Linux against the vendored registry.
- An in-process OTLP receiver in `lablet-conformance` (gRPC and HTTP) so O2 and O7 run in CI without Docker; `telemetry-otel` added to the `RunObserver` conformance matrix, including that an unreachable endpoint does not change the run outcome.
- `lablet/examples/docker-compose.yaml` with a collector (debug exporter) and Jaeger, for the manual check.

Acceptance: a fake-provider run against the in-process receiver yields the root, chat, and tool spans with the documented attributes and one `lablet.run` log record per run. `cargo xtask weaver live-check` passes with zero undeclared attributes. Scenarios O2, O3, O5, O6, O7, and O9 pass. Manual: a reviewer runs the docker compose example and finds turn 2's spans in Jaeger with one filter.

## Phase 7: Anthropic and built-in tools

- `provider-anthropic` with wiremock tests for happy path, tool use, thinking and redacted thinking round trip, cache-control presence, cache token mapping to `cache_write`, per-call timeout, and error classification including context exhaustion.
- `lablet/examples/anthropic.yaml`.

Acceptance: scenarios P1, P2, and P6 pass in CI against wiremock. Manual: `lablet run --config examples/anthropic.yaml --prompt "..."` completes a real task against the Anthropic API and its trace passes live-check with no attribute added for it.

## Phase 8: MCP

- `lablet-test-mcp-server` (echo, sleep, exit-after-N, `--hang-startup`).
- `tools-mcp` over `rmcp`, stdio and streamable HTTP, collision rejection and `prefix_tools`, startup timeout, stderr forwarding, dead-server behaviour, lifetime tied to the `Lablet`, trace context in `params._meta`, and the `mcp.*` attributes reported to the observer.
- `tools-mcp` added to the `ToolExecutor` conformance matrix.

Acceptance: scenarios T1, T3, T5, T6, T7, and T8 pass in CI against the test server. Manual: a run using a public MCP server over stdio completes, its tool spans carry the `mcp.*` attributes, and removing a tool via `tools.deny` changes the `RunStarted` tool list and nothing else.

## Phase 9: second provider

- `provider-openai` against Ollama and an OpenAI-compatible gateway, wiremock tests as for Anthropic, `max_completion_tokens` fallback, tool-role expansion, reasoning field mapping.

Acceptance: the same config with only the `model` section changed completes the same task on a local Ollama model and passes the same scenarios. Scenarios P3, P4, and P7 pass; P5 is recorded manually.

## Phase 10: features

- Skills inlining, pricing and cost on the root span and wide event, `task_complete` schema from config.
- `run.transcript_format: atif` exporting the transcript as an ATIF v1.8 trajectory for Harbor and Terminal-bench, available to the library and the CLI alike.

Acceptance: scenarios S1 to S4 pass. An ATIF export of a fake run validates against Harbor's Pydantic models.

## Phase 11: hardening and release

- User docs in `lablet/docs/`: getting started, config reference generated from the schema, telemetry reference generated from the registry, a page on using `provider-fake` to test a framework, example configs for each provider and for the MCP optimisation use case.
- `cargo xtask bench`: criterion benchmarks for loop overhead per turn and per tool call with the regression threshold in CI.
- Release workflow publishing static Linux (musl, rustls) and macOS binaries; `cargo install` works from the repo. First `CHANGELOG.md` release section.

Acceptance: a new user can follow `lablet/docs/getting-started.md` from clone to a traced run in under five minutes without reading the spec, verified and timed by someone who did not write it. The release checklist in `acceptance.md` is signed off once.

## Exit criteria for every phase

A phase is closed when all of these hold, in addition to its acceptance line:

1. Every automated scenario assigned to the phase is green in CI, not only locally. Manual items (below) are recorded by a human in the closing PR.
2. `cargo xtask pre-push` passes, and no new `#[expect]` suppression lacks a reason tied to the phase.
3. The spec and the code agree. A deviation is a spec edit in the same phase, with a `decisions.md` entry if it changes a decision.
4. The traceability table in `acceptance.md` has no phase-assigned statement without a test or gate.
5. The closing commit message contains one demo command a reviewer can run in under a minute.
6. Every new dependency has a justification comment and `cargo deny` is clean.
7. No scenario has been moved to a later phase to close this one.

Human sign-off, which the building agent cannot do itself: phase 6 the Jaeger check via docker compose; phase 7 the real Anthropic run; phase 8 the public MCP server run; phase 9 the Ollama run (P5); phase 11 the timed getting-started walk and the release checklist.

Delivery: work lands in small, reviewable pull requests, each one logical unit with CI green, merged by a human. The bullet list of a phase is its PR plan; a phase closes when its last PR merges and the human sign-off items are recorded.

Signals of drift, any of which means stop and fix before continuing: the `lablet.*` attribute count grows without decisions entries; an adapter crate imports another adapter; tests exercise only the CLI rather than the ports; a port gains a method that only one fake needs.

## Working rules for the building agent

- Never move to the next phase with failing gates.
- Clarify the spec freely when it has a gap or an inconsistency: edit it in the same PR and say so in the description. Never change anything recorded in `product/decisions.md` unilaterally: stop, present options, and wait for a human.
- Keep dependencies minimal and justify each new one with a comment in `Cargo.toml`.
- Prefer a small change to the spec, made explicitly, over a workaround that makes the code disagree with it.
- When a normative sentence in the spec has no test or gate, add a row to the traceability table in `acceptance.md` before implementing it.
