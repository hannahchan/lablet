# Build plan

Phased so that every phase ends with something runnable and tested. An agent building lablet works one phase at a time, in order, and does not start a phase until the previous one's acceptance holds. Read [spec.md](spec.md) and [../contributing/README.md](../contributing/README.md) first.

## Phase 0: scaffold

- Create the `lablet/` Cargo workspace (edition 2024, resolver 3) with every crate from spec §2 as an empty library or binary with a one-line doc comment.
- `xtask/` at the repo root with `lint-layers`, `fmt`, `clippy`, `test`, `pre-commit`, `pre-push`. Port `lint_layers.rs` from UsefulBytes, reducing the ring set and forbidden-dependency table to spec §2.
- `rust-toolchain.toml`, workspace lints from contributing, GitHub Actions running `cargo xtask pre-push`.
- `lablet/README.md` placeholder and `lablet/docs/` directory.

Acceptance: `cargo xtask pre-push` passes on an empty workspace.

## Phase 1: domain

- `lablet-model`: every type in spec §3, with `Usage: Add`, display impls for `StopReason` and `FinishReason`, and constructors that validate (non-empty tool names, unique block ids).
- `lablet-policy`: `StopPolicy::evaluate`, `RetryPolicy::delay`, `Pricing::cost`.

Acceptance: unit tests cover each stop reason (including token budget, truncated output, and context exhaustion), each completion mode, backoff growth and cap, and cost arithmetic.

## Phase 2: the loop

- `lablet-run`: ports, errors, `RunEvent`, `RunSummary` accumulation, `ToolSet`, `RunService`.
- Hand-written fakes for every port in the crate's tests.

Acceptance: end-to-end tests with fakes prove natural and explicit completion, every stop reason, retry with backoff via the fake clock, consecutive tool error counting and reset, allow and deny filtering, duplicate tool name rejection, and that content fields are `None` when capture is off. The event stream for a scripted run is asserted exactly, and the `RunSummary` on `RunFinished` matches the per-step events it summarises.

## Phase 3: first real run

- `provider-anthropic` with wiremock tests for happy path, tool use, thinking round trip, cache token mapping, per-call timeout, and error classification including context exhaustion.
- `tools-builtin` with `bash`, `read_file`, `write_file`, `task_complete`, and root escape rejection.
- `telemetry-jsonl`, including the wide event as the final line.
- `apps/lablet`: config types, `build`, `Lablet`, CLI with `run`, `check`, `schema`, `--set`, env substitution, config digest, transcript output, diagnostic logging on stderr.
- Smoke test: fake provider plus JSONL observer through `build` and `run`.

Acceptance: `lablet run --config examples/anthropic.yaml --prompt "..."` completes a real task against the Anthropic API, writes a JSONL trace, and prints a `RunOutcome`. `lablet check` lists the resolved tools.

## Phase 4: MCP

- `tools-mcp` over `rmcp`, stdio and streamable HTTP, name collision prefixing, startup timeout, stderr forwarding, dead-server behaviour, shutdown on run end.
- Conformance crate `tests/conformance` with the `ToolExecutor` cases, run against both `tools-builtin` and `tools-mcp` (the latter against a tiny in-repo test MCP server).

Acceptance: a run using a public MCP server over stdio completes, and removing a tool via `tools.deny` is visible in the `RunStarted` event.

## Phase 5: OpenTelemetry

- `telemetry-otel` with the span and attribute table from spec §6, the `lablet.run` wide-event log record, content log records behind `capture_content`, resource attributes, flush on exit.
- Fan-out observer in the composition root so JSONL and OTLP can run together.
- `RunObserver` conformance cases added to `tests/conformance`, including that exactly one wide event is emitted per run, that its numbers equal the sum of the per-step events, and that an unreachable endpoint does not change the run outcome.

Acceptance: a run against a local OTel collector (docker compose file under `lablet/examples/`) shows the root, chat, and tool spans with the documented attributes in Jaeger or the collector debug exporter, and one `lablet.run` log record per run.

## Phase 6: second provider

- `provider-openai` against Ollama and an OpenAI-compatible gateway, wiremock tests as for Anthropic.

Acceptance: the same config with only the `model` section changed completes the same task on a local Ollama model.

## Phase 7: polish

- Skills inlining, pricing and cost on the root span, `task_complete` schema from config.
- User docs in `lablet/docs/`: getting started, config reference generated from the schema, telemetry reference.
- Example configs for each provider and for the MCP optimisation use case.

Acceptance: a new user can follow `lablet/docs/getting-started.md` from clone to a traced run without reading the spec.

## Working rules for the building agent

- Never move to the next phase with failing gates.
- Do not change a decision in `product/` unilaterally. Record the question in spec §9 and continue under the spec as written.
- Keep dependencies minimal and justify each new one with a comment in `Cargo.toml`.
- Prefer a small change to the spec, made explicitly, over a workaround that makes the code disagree with it.
