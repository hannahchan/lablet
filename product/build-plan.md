# Build plan

Phased so that every phase ends with something runnable and tested. [acceptance.md](acceptance.md) names the scenarios each phase must turn green; a phase is not done until they pass. An agent building lablet works one phase at a time, in order, and does not start a phase until the previous one's acceptance holds. Read [spec.md](spec.md), [quality-bar.md](quality-bar.md), and [../contributing/README.md](../contributing/README.md) first.

## Phase 0: scaffold

- Create the `lablet/` Cargo workspace (edition 2024, resolver 3) with every crate from spec §2 as an empty library or binary with a one-line doc comment.
- `xtask/` at the repo root with `lint-layers`, `fmt`, `clippy`, `deny`, `doc`, `test`, `pre-commit`, `pre-push`. Port `lint_layers.rs` from UsefulBytes, reducing the ring set and forbidden-dependency table to spec §2.
- `rust-toolchain.toml`, `mise.toml` pinning weaver, cargo-deny, cargo-llvm-cov, cargo-mutants; workspace lints from contributing; GitHub Actions running `cargo xtask pre-push` plus release builds for Linux (static, musl) and macOS.
- `lablet/telemetry/` with an empty Weaver registry that imports the upstream semconv registry, a policy file, and `xtask weaver check`, `weaver generate`, `weaver docs` commands. The generated `telemetry-registry` crate is checked in.
- `lablet/README.md` placeholder and `lablet/docs/` directory.

Acceptance: `cargo xtask pre-push` passes on an empty workspace, and `cargo xtask weaver generate` is a no-op on a clean tree.

## Phase 1: domain

- `lablet-model`: every type in spec §3, with `Usage: Add`, display impls for `StopReason` and `FinishReason`, and constructors that validate (non-empty tool names, unique block ids).
- `lablet-policy`: `StopPolicy::evaluate`, `RetryPolicy::delay`, `Pricing::cost`.

Acceptance: unit tests cover each stop reason (including token budget, truncated output, and context exhaustion), each completion mode, backoff growth and cap, and cost arithmetic. No acceptance scenarios yet.

## Phase 2: the loop

- `lablet-run`: ports, errors, `RunEvent`, `RunSummary` accumulation, `ToolSet`, `RunService`.
- Hand-written fakes for every port in the crate's tests.

Acceptance: end-to-end tests with fakes prove natural and explicit completion, every stop reason, retry with backoff via the fake clock, consecutive tool error counting and reset, allow and deny filtering, duplicate tool name rejection, and that content fields are `None` when capture is off. The event stream for a scripted run is asserted exactly, and the `RunSummary` on `RunFinished` matches the per-step events it summarises. Scenarios L1 to L7 and E1 to E5 pass against the fakes.

## Phase 3: first real run

- `provider-anthropic` with wiremock tests for happy path, tool use, thinking round trip, cache token mapping, per-call timeout, and error classification including context exhaustion.
- `tools-builtin` with `bash`, `read_file`, `write_file`, `task_complete`, and root escape rejection.
- `telemetry-jsonl`, including the wide event as the final line.
- `provider-fake` with scripted completions, latency, and injected errors.
- `apps/lablet`: config types, `build`, `Lablet`, CLI with `init`, `run`, `check` (including `--resolved`), `schema`, `--set`, env substitution, config digest, transcript output, diagnostic logging on stderr, and the error message contract from spec §7.
- Smoke test: `provider-fake` plus JSONL observer through `build` and `run`.
- Coverage and mutation floors in CI for `lablet-model`, `lablet-policy`, and `lablet-run`.

Acceptance: `lablet run --config examples/anthropic.yaml --prompt "..."` completes a real task against the Anthropic API, writes a JSONL trace, and prints a `RunOutcome`. `lablet check` lists the resolved tools. Scenarios E6, T2, T4, O1, O4, O8, C1 to C9, P1, and P2 pass.

## Phase 4: MCP

- `tools-mcp` over `rmcp`, stdio and streamable HTTP, name collision prefixing, startup timeout, stderr forwarding, dead-server behaviour, shutdown on run end.
- Conformance crate `tests/conformance` with the `ToolExecutor` cases, run against both `tools-builtin` and `tools-mcp` (the latter against a tiny in-repo test MCP server).

Acceptance: a run using a public MCP server over stdio completes, and removing a tool via `tools.deny` is visible in the `RunStarted` event. Scenarios T1, T3, T5, and T6 pass.

## Phase 5: OpenTelemetry

- Weaver registry filled in: every span, event, attribute, and the wide event from spec §1 and §6, with `gen_ai.*` referenced from upstream. Regenerate `telemetry-registry` and `docs/telemetry.md`.
- `telemetry-otel` built only on `telemetry-registry` builders, with the `lablet.run` wide-event log record, content log records behind `capture_content`, resource attributes, flush on exit.
- CI job: a `provider-fake` run exports to `weaver registry live-check`; any undeclared or mistyped attribute fails the build.
- Fan-out observer in the composition root so JSONL and OTLP can run together.
- `RunObserver` conformance cases added to `tests/conformance`, including that exactly one wide event is emitted per run, that its numbers equal the sum of the per-step events, and that an unreachable endpoint does not change the run outcome.

Acceptance: a run against a local OTel collector (docker compose file under `lablet/examples/`) shows the root, chat, and tool spans with the documented attributes in Jaeger or the collector debug exporter, and one `lablet.run` log record per run. `weaver registry live-check` passes on that run. Scenarios O2, O3, O5, O6, and O7 pass.

## Phase 6: second provider

- `provider-openai` against Ollama and an OpenAI-compatible gateway, wiremock tests as for Anthropic.

Acceptance: the same config with only the `model` section changed completes the same task on a local Ollama model. Scenarios P3 and P4 pass; P5 is recorded manually.

## Phase 7: polish

- Skills inlining, pricing and cost on the root span, `task_complete` schema from config.
- User docs in `lablet/docs/`: getting started, config reference generated from the schema, telemetry reference generated from the registry, a page on using `provider-fake` to test a framework.
- Example configs for each provider and for the MCP optimisation use case; docker compose for collector plus Jaeger.
- Criterion benchmarks for loop overhead per turn and per tool call.
- Release workflow publishing static Linux and macOS binaries; `cargo install` works from the repo.

Acceptance: a new user can follow `lablet/docs/getting-started.md` from clone to a traced run in under five minutes without reading the spec, verified by someone who did not write it. The release checklist in `acceptance.md` is signed off once.

## Working rules for the building agent

- Never move to the next phase with failing gates.
- Do not change a decision in `product/` unilaterally. Record the question in spec §9 and continue under the spec as written.
- Keep dependencies minimal and justify each new one with a comment in `Cargo.toml`.
- Prefer a small change to the spec, made explicitly, over a workaround that makes the code disagree with it.
