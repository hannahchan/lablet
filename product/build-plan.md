# Build plan

Phased so that every phase ends with something runnable and tested. [acceptance.md](acceptance.md) names the scenarios each phase must turn green; a phase is not done until they pass. An agent building lablet works one phase at a time, in order, and does not start a phase until the previous one's acceptance holds. Read [spec.md](spec.md), [quality-bar.md](quality-bar.md), and [../contributing/README.md](../contributing/README.md) first.

## Phase 0: scaffold

- Create the `lablet/` Cargo workspace (edition 2024, resolver 3, `rust-version` set) with every crate from spec §2 as an empty library or binary with a one-line doc comment. Root `.cargo/config.toml` with the `xtask` alias.
- `xtask/` at the repo root with `lint-layers`, `fmt`, `clippy`, `deny`, `doc`, `test`, `changelog`, `weaver check`, `weaver generate`, `pre-commit`, `pre-push`. Port `lint_layers.rs` from UsefulBytes, reducing the ring set and forbidden-dependency table to spec §2.
- `rust-toolchain.toml`, `mise.toml` pinning weaver, cargo-deny, cargo-llvm-cov, cargo-mutants; `scripts/install-hooks.sh`; workspace lints from contributing; `CHANGELOG.md`; GitHub Actions running `cargo xtask pre-push`.
- `lablet/telemetry/` with `registry_manifest.yaml`, the core and GenAI semantic-convention registries vendored at pinned commits under `deps/`, a policy file, `weaver.yaml`, and Rust templates that generate **attribute name constants and enums only**. The generated `telemetry-registry` crate is checked in.
- `lablet/README.md` placeholder and `lablet/docs/` directory.

Acceptance: `cargo xtask pre-push` passes on an empty workspace, and `cargo xtask weaver generate` is a no-op on a clean tree.

## Phase 1: domain

- `lablet-model`: every type in spec §3 with serde derives, `Usage: Add` and `total()`, display impls for `StopReason` and `FinishReason`, constructors that validate (non-empty tool names, unique block ids).
- `lablet-policy`: `StopPolicy::evaluate` at both stop points, `RetryPolicy::delay`, `Pricing::cost`.

Acceptance: unit tests cover each stop reason at the stop point that owns it (including token budget, truncated output, and context exhaustion), each completion mode, backoff growth, cap, and exhaustion, and cost arithmetic. No acceptance scenarios yet.

## Phase 2: the loop

- `lablet-run`: ports, errors, `RunEvent`, `RunSummary` accumulation, `ToolSet`, `RunService` with the two-point stop evaluation and `task_complete` interception.
- Hand-written fakes for every port, including a fake clock, in the crate's tests.

Acceptance: end-to-end tests with fakes prove natural and explicit completion, every stop reason, retry with backoff via the fake clock, consecutive tool error counting and reset, allow and deny filtering, duplicate tool name rejection, and that content fields are `None` when capture is off. The event stream for a scripted run is asserted exactly, and the `RunSummary` on `RunFinished` matches the per-step events it summarises. Scenarios L1 to L7, E1 to E5, and C8 pass against the fakes.

## Phase 3a: fake provider, CLI, first traced run

- `provider-fake` with scripted completions, latency, and injected errors.
- `telemetry-jsonl`, including the wide event as the final line.
- `apps/lablet`: config types, `build`, `Lablet` with multi-run and shutdown, CLI with `init`, `run`, `check` (including `--resolved`), `schema`, `--set`, env substitution, config digest over the resolved config, transcript output, diagnostic logging on stderr, and the error message contract from spec §7.
- Smoke test: `provider-fake` plus JSONL observer through `build` and `run`.
- Coverage and mutation floors in CI for `lablet-model`, `lablet-policy`, and `lablet-run`.

Acceptance: `lablet init --provider fake && lablet run --config lablet.yaml --prompt "..."` completes, writes a JSONL trace, and prints a `RunOutcome`. Scenarios O1, O4, O8, C1 to C7, C9, and C10 pass.

## Phase 3b: Anthropic and built-in tools

- `provider-anthropic` with wiremock tests for happy path, tool use, thinking and redacted thinking round trip, cache-control presence, cache token mapping, per-call timeout, and error classification including context exhaustion.
- `tools-builtin` with `bash`, `read_file`, `write_file`, `task_complete`, and root escape rejection.
- `lablet/examples/anthropic.yaml`.

Acceptance: `lablet run --config examples/anthropic.yaml --prompt "..."` completes a real task against the Anthropic API. Scenarios E6, T2, T4, P1, P2, and P6 pass.

## Phase 4: MCP

- `lablet-test-mcp-server` (echo, sleep, exit-after-N, `--hang-startup`).
- `tools-mcp` over `rmcp`, stdio and streamable HTTP, collision rejection and `prefix_tools`, startup timeout, stderr forwarding, dead-server behaviour, lifetime tied to the `Lablet`.
- `lablet-conformance` with the `ToolExecutor` cases, run against `tools-builtin` and `tools-mcp`.

Acceptance: a run using a public MCP server over stdio completes, and removing a tool via `tools.deny` is visible in the `RunStarted` event. Scenarios T1, T3, T5, T6, and T7 pass.

## Phase 5: OpenTelemetry

- Weaver registry filled in: every span, event, attribute, template attribute, and the wide event from spec §1 and §6, with `gen_ai.*` referenced from the vendored GenAI registry. Regenerate `telemetry-registry` and `docs/telemetry.md`.
- `telemetry-otel` built only on `telemetry-registry` constants, with the `lablet.run` wide-event log record carrying the root span's trace context, content log records behind `capture_content`, resource attributes, bounded shutdown.
- Fan-out observer in the composition root so JSONL and OTLP can run together.
- `cargo xtask weaver live-check` and its CI job.
- `RunObserver` conformance cases added to `lablet-conformance`, including that exactly one wide event is emitted per run, that its numbers equal the sum of the per-step events, and that an unreachable endpoint does not change the run outcome.
- `lablet/examples/docker-compose.yaml` with a collector (debug exporter) and Jaeger.

Acceptance: a run against the example collector prints the root, chat, and tool spans with the documented attributes on the debug exporter, and one `lablet.run` log record per run. `cargo xtask weaver live-check` passes. Scenarios O2, O3, O5, O6, and O7 pass.

## Phase 6: second provider

- `provider-openai` against Ollama and an OpenAI-compatible gateway, wiremock tests as for Anthropic, `max_completion_tokens` fallback, tool-role expansion, reasoning field mapping.

Acceptance: the same config with only the `model` section changed completes the same task on a local Ollama model. Scenarios P3, P4, and P7 pass; P5 is recorded manually.

## Phase 7: polish and release

- Skills inlining, pricing and cost on the root span and wide event, `task_complete` schema from config.
- User docs in `lablet/docs/`: getting started, config reference generated from the schema, telemetry reference generated from the registry, a page on using `provider-fake` to test a framework.
- Example configs for each provider and for the MCP optimisation use case.
- Criterion benchmarks for loop overhead per turn and per tool call with a regression threshold.
- Release workflow publishing static Linux (musl, rustls) and macOS binaries; `cargo install` works from the repo. First `CHANGELOG.md` release section.

Acceptance: a new user can follow `lablet/docs/getting-started.md` from clone to a traced run in under five minutes without reading the spec, verified by someone who did not write it. Scenarios S1 to S4 pass. The release checklist in `acceptance.md` is signed off once.

## Working rules for the building agent

- Never move to the next phase with failing gates.
- Do not change a decision in `product/` unilaterally. Record the question in spec §10 and continue under the spec as written.
- Keep dependencies minimal and justify each new one with a comment in `Cargo.toml`.
- Prefer a small change to the spec, made explicitly, over a workaround that makes the code disagree with it.
- When a normative sentence in the spec has no test or gate, add a row to the traceability table in `acceptance.md` before implementing it.
