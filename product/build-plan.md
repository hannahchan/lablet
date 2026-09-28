# Build plan

Phases 0 to 12, each ending with something runnable and tested. [acceptance.md](acceptance.md) names the scenarios each phase must turn green; a phase isn't done until they pass and the exit criteria below hold. An agent building lablet works one phase at a time, in order, and doesn't start a phase until the previous one is closed. Read [spec.md](spec.md), [quality-bar.md](quality-bar.md), and [../contributing/README.md](../contributing/README.md) first.

Sequencing rationale: the loop is proven against fakes before any real adapter exists; OpenTelemetry comes before the first real provider because a wrong span shape is more expensive to fix late than a wrong provider mapping; the telemetry contract is a phase of its own because the Weaver Rust templates are the riskiest piece of the project and must not stall the scaffold; phases 3a to 3c settle the domain and the loop before anything is built on them; phase 12 holds what waits for the first release.

## Phase 0: Scaffold

- The `lablet/` Cargo workspace (edition 2024, resolver 3, `rust-version`, `license = "MIT OR Apache-2.0"` in workspace package metadata) with every crate from spec §2 as an empty library or binary with a one-line doc comment. Root `.cargo/config.toml` with the `xtask` alias.
- `xtask/` at the repo root with `lint-layers`, `lint-manifests`, `fmt`, `clippy`, `deny`, `doc`, `test`, `coverage` and `mutants` (floors and crate list held as data in xtask), `changelog`, `pre-commit`, `pre-push`.
- `[workspace.dependencies]` with every third-party crate pinned to an exact version checked against crates.io that day: at least `async-trait`, `thiserror`, `serde`, `serde_json`, `serde-saphyr` (YAML; `serde_yaml` is archived), `tokio`, `reqwest`, `rmcp`, `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-proto` (features `gen-tonic-messages`, `trace`, `logs`, `with-serde`; not the default `full`), `opentelemetry-otlp`, `prost`, `tonic`, `axum` (phase 6 receiver), `tracing`, `tracing-subscriber`, `clap`, `humantime`, `ulid`, `sha2`, `schemars`, `wiremock`, `criterion`. Port `lint_layers.rs` from UsefulBytes, reducing the ring set and forbidden-dependency table to spec §2.
- `rust-toolchain.toml`, `mise.toml` pinning weaver, cargo-deny, cargo-llvm-cov, cargo-mutants; `scripts/install-hooks.sh`; workspace lints from contributing; a root `clippy.toml`; `scripts/setup.sh`; `CHANGELOG.md`; GitHub Actions on every pushed branch running `cargo xtask ci` (the pre-push list) with `cargo xtask coverage` and `cargo xtask mutants` as matrix legs beside it.
- `lablet/README.md` placeholder and `lablet/docs/` directory.

Acceptance: `cargo xtask pre-push` passes on the empty workspace. A crate given a deliberately wrong-direction dependency fails `lint-layers`. A fresh clone passes the gates with one command after `mise install`.

## Phase 1: Telemetry contract

Follow `product/research/weaver/README.md` and start from its `spike/` files; it verified everything below against Weaver v0.26.1.

- `mise.toml` pins weaver `0.26.1` (`github:open-telemetry/weaver`; the `ubi` backend the research used is deprecated in mise). `cargo xtask weaver vendor` copies the `model/` trees of core semconv `v1.44.0` and `semantic-conventions-genai` at the pinned commit, plus the policies and markdown templates from `opentelemetry-weaver-packages`, under `lablet/telemetry/deps/` with `SOURCES` files.
- `lablet/telemetry/registry/` in v2 syntax with `manifest.yaml` using relative dependency paths; `lablet.*` attributes with justification notes; lablet-owned spans `lablet.invoke_agent`, `lablet.chat`, `lablet.execute_tool` and events `lablet.run` and the content record, referencing the `gen_ai.*`, `mcp.*`, and `error.type` keys from spec §1 and §6 with requirement levels; entity imports for the resource. Every attribute must have a source field in spec §3 or §5; if one doesn't, the spec is fixed first.
- `lablet/telemetry/policies/justification.rego` and `cargo xtask weaver check` running the lablet and vendored policies.
- Rust templates under `lablet/telemetry/templates/registry/rust/` (constants, enums, per-signal key lists; no builders) and `cargo xtask weaver generate` producing the `lablet-telemetry-registry` crate, `cargo fmt`, and `lablet/docs/telemetry/` from the vendored markdown templates, with a `--check` mode used as the generated-files gate.
- `lablet/telemetry/.weaver.toml` with the live-check finding filters from the spike; the `live-check` xtask command itself lands in phase 6.

Acceptance: `cargo xtask weaver check` passes. Regeneration is a no-op on a clean tree. Every attribute in spec §1 and §6 has a constant in the generated crate and the generated docs list it. A `lablet.*` attribute added without a justification note fails the policy. A weekly CI job runs `cargo xtask weaver vendor --check`, which fetches each pinned commit again and fails on any difference from the vendored tree.

## Phase 2: Domain

- `lablet-model`: every type in spec §3 with serde derives (including `RequestParams` and `Endpoint`), `Usage: Add` and `total()`, display impls for `StopReason` and `FinishReason`, constructors and deserialisation that hold each type to its rules (non-empty tool names, distinct tool call ids, a transcript made of turns whose outcomes answer their calls, an outcome whose error and structured result follow from its stop reason), the tool output cap, and `Run`, which keeps the transcript and turns a run into its `FinishedRun`. The transcript maps onto an ATIF v1.8 trajectory (one turn is one step, tool call outcomes joined by call id), which is what makes phase 10 an export rather than a reconstruction; the shape is the run's rather than ATIF's, and the export carries what ATIF has no slot for (see the 2026-09-21 commitment to being vendor and solution agnostic).
- `lablet-policy`: `StopPolicy` with one method for each of the three stop points (`before_call`, `after_response`, `after_tools`) and `allows_wait` for a backoff that would reach the timeout, `RetryPolicy::delay`, `Pricing::cost`.

Acceptance: unit tests cover each of the nine stop reasons the policy decides at the stop point that owns it (including token budget, truncated output, and refusal; `cancelled`, `retries_exhausted`, `provider_error`, and `context_exhausted` for a rejected request belong to the loop and are held by phase 3's scenarios), each completion mode, backoff growth, cap, and exhaustion, and cost arithmetic. Coverage and mutation floors met. No async code and no serde beyond derives in either crate.

## Phase 3: The loop

- `lablet-run`: ports, errors, the port data types `TraceContext`, `McpCallMeta`, and `NetworkTransport`, `RunEvent`, the `FinishedRun` built through the model's `Run`, the tool output cap applied to every tool call, `ToolSet` with the `task_complete` spec, `RunService` with the three-point stop evaluation, `task_complete` interception, and the trace-context handoff from observer to `ToolCall`.
- Hand-written fakes for every port, including a fake clock, in the crate's tests.

Acceptance: end-to-end tests with fakes prove natural and explicit completion, every stop reason, retry with backoff via the fake clock, consecutive tool error counting and reset, and that content fields are `None` when capture is off. `ToolSet`'s allow and deny filtering and its duplicate-name rejection are unit-tested here, against the crate's own fakes; the scenarios that drive them end to end are T1, T2, and T3, which acceptance.md assigns to phases 4 and 8. The event stream for a scripted run is asserted exactly, and the `RunSummary` on `RunFinished` matches the per-step events it summarises. Scenarios L1 to L10, E1 to E8, E10, T10, and C8 pass, E7, E8, and T10 as they stood then, since phase 3b rewrites them, and with one clause deferred: L9's exit code is the CLI's mapping of a stop reason that isn't `completed`, so phase 3 asserts the stop reason and phase 5 asserts the code.

## Phase 3a: Settle the domain and the loop

Found by the review of the loop after phase 3 closed, and done before phase 4 so that the observer, the built-in tools, and the documents phase 4 writes are built on the domain and loop they'll keep. Every change is in a phase 2 or phase 3 crate. The design is in `decisions.md`, 2026-09-23.

- A spike, first: `Pending::answer` taking an `AsyncFn`, called from `RunService::run`, with the run's future still `Send`. If it isn't, the recorded fallback (`Fn(ToolUse) -> Fut` with an owned `ToolUse`) is taken and the decision entry amended before anything else lands.
- The run's states. In `lablet-model`: `Responded`, `Final`, and `Pending` with `Pending::calls`, and `TranscriptError` deleted along with the checks behind it. In `lablet-policy`: `after_final`, and `Calls` without `None`.
- Concurrent groups. In `lablet-model`: `Answer`, `ToolConcurrency` on `ToolSpec`, `Schedule`, and `Pending::answer` over `futures-util`'s `StreamExt::buffered`. Its tests poll with `std::task::Waker::noop()` and futures the test controls, and they show overlap and call order on one thread, or the work stops and the rule is raised again. In `lablet-run`: the loop over the new states, `ToolSet::concurrency`, `CallLimits::max_concurrent_tool_calls`, and `Stopped::defect` deleted.
- Loop fixes: cancellation polled before each retry; `ToolSetError::UnknownFilterName`; a tool call's start offset read from the clock reading that started it, as a provider attempt's is.
- A `task_complete` call whose arguments didn't parse, decided and recorded. Recommended: it's answered as `malformed_input` and the run goes on, because `Pending::calls` reads it as `Tools`, so a run never reports `completed` with no result.
- `lablet-run`'s coverage floor raised to 100% lines and regions, in `xtask/src/floors.rs`, `contributing/README.md`, and `quality-bar.md`. `contributing/reviews.md` item 11 names `Pending::calls`.

Landing order, riskiest first: the spike; the run's states; concurrent groups; the loop fixes; the `task_complete` decision; the floor.

Acceptance: scenarios L11, L12, and E11 pass. `ToolSet`'s refusal of an unknown filter name is unit-tested here, and T11 drives it end to end in phase 4, as T2 does for the allow list. `lablet-model`, `lablet-policy`, and `lablet-run` hold 100% lines and regions, and every mutation floor is met. Phase 3's scenarios still pass, with the one whose `task_complete` arguments didn't parse updated to the decision.

## Phase 3b: Settle the domain and the loop after the design review

Found by the design reviews of 2026-09-24, and done before phase 4 for phase 3a's reason: the observer, the built-in tools, the documents and the library are built on the types this phase changes. Every change is in a phase 2 or phase 3 crate, or in the telemetry registry where an enum the domain mirrors changes. The design is in `decisions.md`, 2026-09-28, and spec §1, §3, §4 and §5 describe it.

- Usage. In `lablet-model`: `Usage` and `TokenCounts` hold the reasoning, cache-read and cache-write counts as options, `Add` leaves a sum absent only when no call reported the count, and the outcome fixture writes an absent count as `null`, with its changelog entry.
- Provider failures. In `lablet-model`: `ProviderErrorKind::Auth`, and `Run::failed_attempt` taking the usage a failed attempt reported, which `Progress`, the summary and the cost then count, with `spent` on each of the run's states. In `lablet-run`: `ProviderError` with `usage` and `retry_after`, a message held to the port's bound, and point A asked before each retry.
- The retry policy. In `lablet-policy`: `RetrySettings`, `RetryPolicy::next` taking the server's hint and a salt, the cap on a hint, and jitter that's a pure function of the salt. In `lablet-model`: `RunId::salt`.
- The stop policy. In `lablet-policy`: an optional turn cap, and the cap on consecutive invalid turns in place of the cap on tool errors. In `lablet-model`: `Progress::consecutive_invalid_turns`, and `StopReason::InvalidCallsExhausted`, renamed in the registry too, with its changelog entry.
- `task_complete` alone. In `lablet-model`: `CompletionMode::intercepts`, `ToolCallStatus::Rejected`, with its registry value, and `Pending::completed_with` answering only for a response whose one call is the completion call. In `lablet-run`: the loop rejects a completion call that wasn't the response's only call, and runs the others.
- The output cap. In `lablet-model`: `OutputCap`, `OutputCut`, and `KeptOutput`, the accumulator an executor feeds, with `Answer::measured` cutting what it kept. In `lablet-run`: `ToolCall::keep` and `ToolOutput::output`, so an executor keeps no more than the cut can use.
- Deadlines. In `lablet-policy`: `StopPolicy::time_left`. In `lablet-model`: `ToolCallStatus::NotRun`, with its registry value, in the transcript and in no total. In `lablet-run`: a tool call's deadline is the time the run has left, a provider call's is the shorter of that and `run.provider_timeout`, a tool phase doesn't start with no time left, a call whose turn comes after the time has gone is answered `not_run`, `CallLimits::tool_timeout` is deleted, and `ToolExecutor` states that a call that timed out has stopped.
- What the record names. In `lablet-model`: `RunLabels` in `RunSetup`, `RunContext` and `RunOutcome`; the two digests and the size of the tool specs in `RunSetup` and the summary; the API and whether reasoning is sent back in `ModelRef`; the cache scope in `RequestParams`; and the MCP lifetime, the servers' versions and the run's start time in `RunContext`. In `lablet-run`: the loop takes the digests from what it sends, and `ProviderRequest::cache_key`.

Landing order, riskiest first: usage, which touches the most code; provider failures and the retry policy; the stop policy and the renamed stop reason; `task_complete` alone; the output cap; deadlines; what the record names.

Acceptance: scenarios L13 to L15, E12 to E18, and T13 pass, and E7, E8, and T10 pass as rewritten. Two clauses of E15 are deferred, as L9's exit code was: the chat span's `error.type` to phase 4, and the exit code to phase 5. `lablet-model`, `lablet-policy`, and `lablet-run` hold 100% lines and regions, and every mutation floor is met. Every scenario of phases 3 and 3a still passes.

## Phase 3c: Context management

Decided on 2026-09-24 and 2026-09-28. Done before phase 4 because it changes what phase 4 publishes: the transcript document, the request size on a chat span, and what a chat span counts. The phase opens with its design commit, which edits spec §1, §3, §4 and §5 as phase 3a's did. When the first capture has been run by then, the trigger and the placeholder of masking come from it. When it hasn't, they're provisional, and `decisions.md` says so.

- The record and the view. In `lablet-model`: the transcript stays the record of what happened, and `Run::messages` renders what's sent under the run's context policy.
- Masking. In `lablet-policy`: when to mask, read from `Progress`. In `lablet-model`: a `Run` masks its older tool results in one step and is a `Run` again, the transition phase 12's summarisation will take too; each masked outcome records the turn from which the model was sent a placeholder in its place; and the placeholder is worded once. `run.context` is `full` or `mask`, and `full` is the default.
- Request size. In `lablet-run`: `request_bytes` is measured from what each call sends, so a call after a masking step reports the smaller request.
- Provider calls that aren't turns. In the registry: a chat span's purpose, with `turn` its one value until phase 12 adds another, and the rule that chat spans number `turns + retries` restated for the spans whose purpose is `turn`.
- Room for compaction. The transcript document's rules say where a compaction entry goes, so that phase 12 adds to the document and changes nothing in it.

Acceptance: scenarios K1 to K3 pass, the floors hold, and every earlier scenario still passes.

## Phase 4: Library and first traced run

- `provider-fake` with scripted completions, latency, and injected errors, which may carry the usage a failed attempt reported and the wait a server asked for. The script format is this adapter's own type, read into the domain through `ProviderResponse::new`, so the file a user hand-writes isn't the domain's serde form. It keeps its validating read: a human writes it, so a mistyped key that read as a default would be a script that lies about what the model said.
- `tools-builtin` with `bash`, `read_file`, `write_file`, and root escape rejection after symbolic links are resolved. None is enabled by default, and with one enabled the root is required and may not hold lablet's own files. `bash` starts a new process, in a process group of its own, for each command, with a short list of environment variables. It reports a non-zero exit as an ordinary result, and at its deadline the whole group is killed before `execute` returns. `read_file` takes an offset and a limit. Every tool feeds its output to a `KeptOutput`, so it keeps no more than the call's limit allows.
- `telemetry-otel`: the observer mapping events to spans and log records built only on `telemetry-registry` constants (open spans in a map keyed by call id, explicit parent contexts, always-on sampler, per-run file path set on `RunStarted`, `force_flush` after the wide event), with unit tests asserting each span's name and required attributes against the generated key lists; root, chat, and tool spans with the spec §6 attributes, `lablet.turn` on children, the `gen_ai.client.operation.exception` log record and retry span event, the `lablet.run` wide-event log record with the root span's trace context, content records behind `capture_content`, with the tool specs as `gen_ai.tool.definitions` among them, resource attributes, bounded shutdown. The wide event goes in an export of its own after the content records are flushed, attributes have a length limit, and the wide event counts the records the exporter dropped. The run's labels are on every record, and the instrumentation scope carries the registry's schema URL. The registry declares every attribute the decisions of 2026-09-28 added, each with its justification and a changelog entry, before the observer emits it. Only the **OTLP/JSON file exporter** in this phase, serialised through `opentelemetry-proto`'s `with-serde` types and the `group_*_by_resource_and_scope` transforms, compact one request per line; with a reader in `lablet-conformance` that dispatches on `resourceSpans` or `resourceLogs` and parses the file back into spans and records for assertions.
- `lablet-conformance` with the `RunObserver` cases (exactly one wide event per run, its numbers equal the sum of the per-step events, an unwritable destination doesn't change the outcome) and the `ToolExecutor` cases run against `tools-builtin`, among them that a call past its deadline has stopped when `execute` returns and that an executor keeps no more output than its limit.
- `lablet-documents` under `crates/adapters/secondary/shared/`: the transcript document, which gains the run id, the run's labels, the config digest, lablet's version, the model, the start time and the offered tool specs; the outcome document, which gains `schema_version` and the run's labels; and the shapes they nest, deriving serde, built from the domain by mappings that take their source apart by pattern, so a new domain field is a compile error. It's a shared kernel because `provider-fake`'s script, the transcript writer and the outcome share shapes, `Usage` among them, some read and some written. Both documents move in this one change rather than in sequence, since building the crate and then reopening it carries the whole objection to moving the outcome at all. `Transcript`, `Turn`, `RunOutcome` and the leaves they nest lose their derives when it lands, `RawOutcome` becomes the parts `Run::finish` fills in with `RunOutcome::closing` public as a checked constructor, and `RunSummary` and `FinishedRun` lose theirs with no replacement, since the wide event is a field mapping rather than a serialisation. `serde_json::Value` stays in the domain, which is vocabulary rather than a wire form, and `Message` keeps `Serialize` because `RequestBytes` measures with it and those bytes are counted and discarded.
- `lablet/tests/fixtures/transcript.json`: a checked-in document built in code by a test, compared byte for byte and written back unchanged, as `outcome.json` already is, and added to the changelog gate's watched list beside it. It replaces the compile-time drift guard `TranscriptDocument::of` had while it lived beside the private fields it read.
- `transcript-json`: writes the transcript document to `run.transcript_path`, which may hold `{run_id}`. The write stays in `Lablet::run`, where effects belong; a port for it waits for phase 10, when ATIF makes a second format to generalise from.
- `apps/lablet` as a library only: config types with the defaults of spec §7, `build` with `BuildError::Unsupported` for adapters from later phases, `Lablet` with multi-run and shutdown, `Lablet::run` taking a `RunRequest` and returning the `FinishedRun`, `RunContext` construction with the run's labels and the digests of the tool specs and the system prompt, the config digest over the settings that affect behaviour, transcript output, the fan-out observer.
- Smoke test: `provider-fake` plus `tools-builtin` plus the file exporter through `build` and `run`, asserting the spans and records read back.
- The wide-event mapping is held to the generated key list rather than written out by hand: every key the registry declares for `lablet.run` is filled exactly once, and a key added to the registry without a source in `RunSummary` fails to compile. Twelve of the summary's fields are bare `u64` bytes, milliseconds and counts, so two of them swapped in a hand-written mapping is a bug no gate would catch: the compiler sees one type, the registry declares every one of them `int`, and the summary's own JSON test checks how it serialises rather than how it reaches telemetry. Group `RunSummary`'s totals into value types with `Add` impls, as `Usage` already has, in the same change: `finish` then folds each turn's totals in rather than initialising twenty zeros and mutating them, and the real mapping shows which groups it wants.

Landing order, riskiest first: `provider-fake`; observer plus file exporter plus reader (O1, O2, O4, O10, O14, O15); library (C9, O8, O11, O12); built-in tools (E9, T2, T4, T14 to T17). The exhaustive mapping and the summary grouping land together, after the observer exists and before the mapping is written out by hand.

Acceptance: a doctest builds a `Lablet` from a config string, runs twice, and asserts both outcomes and the file. Scenarios O1, O2, O4, O8, O10 to O12, O14, O15, C9, E9, T2, T4, T11, and T14 to T17 pass, with the clause of E15 that phase 3b deferred here.

## Phase 5: CLI and config surface

- `main.rs` and the `clap` derive CLI: `init`, `run`, `check` (including `--resolved`), `schema`, `--set`, `${VAR}` substitution, prompt sources, diagnostic logging on stderr, the end-of-run summary line and `--quiet`, Ctrl-C and `SIGTERM` into the `Cancellation` port, exit codes, and the error message contract from spec §7. `check` and `build` refuse a setting the config states and the selected provider can't apply, `--run-id`, `--task`, `--experiment` and `--trial` fill the run request, a message shows a config value as it was written, and a telemetry file path of `-` implies `--quiet`.
- `lablet/schema.json` checked in and covered by the changelog gate.

Acceptance: `lablet init --provider fake && lablet run --config lablet.yaml --prompt "..."` completes with no edits, writes an OTLP/JSON file, and prints a `RunOutcome`. The CLI and the phase 4 doctest produce identical outcomes for the same config. Scenarios C1 to C7 and C10 to C18 pass, with the exit codes of L9 and E15.

## Phase 6: OTLP network export and live-check

- The OTLP network exporter (gRPC and HTTP/protobuf on `rustls`) added to `telemetry-otel`, selected by `telemetry.otlp.endpoint`, active alongside the file exporter when both are set.
- An in-process OTLP receiver in `lablet-conformance` (gRPC and HTTP) so network scenarios run in CI without Docker; `telemetry-otel` over the network added to the `RunObserver` conformance matrix, including that an unreachable endpoint doesn't change the run outcome.
- `cargo xtask weaver live-check`: starts `weaver registry live-check` without `--v2` on a random free port pair, runs the fake-provider config with `--set telemetry.otlp.endpoint=<port>` over OTLP gRPC, stops it through the admin endpoint, saves the report, fails on violations. Its CI job runs on Linux against the vendored registry.
- `lablet/examples/docker-compose.yaml` with a collector (debug exporter, plus the `otlpjsonfile` receiver with `start_at: beginning` and the same `include` path wired into both a traces and a logs pipeline, since the receiver is instantiated per signal and defaults to tailing from the end) and Jaeger, for the manual checks.

Acceptance: a fake-provider run against the in-process receiver yields the same spans and log records as the file exporter wrote for the same run. `cargo xtask weaver live-check` passes with zero undeclared attributes. Scenarios O3, O5, O6, O7, O9, and O13 pass. Manual: a reviewer runs the docker compose example, finds turn 2's spans in Jaeger with one filter, and replays a lablet file through the collector's OTLP JSON file receiver into Jaeger.

## Phase 7: Anthropic and built-in tools

- `provider-anthropic` with wiremock tests for happy path, tool use, thinking and redacted thinking round trip, cache breakpoints on the system prompt, the tool specs and the latest messages, `model.cache_scope`, cache token mapping to `cache_write`, the server's retry hint, per-call timeout, error classification including `auth` and context exhaustion, and error messages held to the port's bound.
- The capture of the primary reference. The builder prepares the commands from `product/research/parity/claude-code.md`, and the human runs them, since they need an API key. The builder compares what the reference sent with what lablet sends for the same conversation, records each row the capture settles in `product/research/parity/matrix.md`, and changes a default the capture contradicts.
- `lablet/examples/anthropic.yaml`.

Acceptance: scenarios P1, P2, P6, P8, and P11 pass in CI against wiremock. Manual: `lablet run --config examples/anthropic.yaml --prompt "..."` completes a real task against the Anthropic API and its trace passes live-check with no attribute added for it; the capture is recorded.

## Phase 8: MCP

- `lablet-test-mcp-server` (echo, sleep, exit-after-N, an image tool that returns a text item and an image item, a structured tool that returns `structuredContent` beside its text, a tool that reports the server's environment, a tool annotated `readOnlyHint`, server instructions, `--hang-startup`).
- How a run ends when its servers can't be started, designed and recorded before anything else lands, since `tools.mcp_lifetime: run` starts servers at the start of a run.
- `tools-mcp` over `rmcp`, stdio and streamable HTTP: names that carry the server's by default, and `names: own` with collision rejection; server instructions and the cap on descriptions; `tools.mcp_result`, and a resource link sent as text; `call_timeout`, with a cancellation notice at the deadline; a short list of environment variables for a stdio server; startup timeout, stderr forwarding, and dead-server behaviour; `tools.mcp_lifetime`; the server's name and version on the wide event; error messages held to the port's bound; trace context in `params._meta`, and the `mcp.*` attributes reported to the observer.
- The capture of the matrix's MCP rows, run and compared as phase 7's was.
- `tools-mcp` added to the `ToolExecutor` conformance matrix.

Acceptance: scenarios T1, T3, T5 to T9, T12, and T18 to T20 pass in CI against the test server. Manual: a run using a public MCP server over stdio completes, its tool spans carry the `mcp.*` attributes, and removing a tool via `tools.deny` changes the `RunStarted` tool list and nothing else; the capture is recorded.

## Phase 9: Second provider

- `provider-openai`, wiremock tests as for Anthropic. The Responses API for OpenAI's own models, with reasoning items sent back and nothing stored. Chat completions for every other server, against Ollama and an OpenAI-compatible gateway, with `max_completion_tokens` fallback, tool-role expansion, reasoning field mapping, and a setting for each server that sends reasoning back. `effort` mapped, a request's cache key sent as the API allows, the error table of spec §6, and error messages held to the port's bound.
- The capture of the secondary reference, from `product/research/parity/codex.md`, run and compared as phase 7's was.
- The Ollama example says how to set the context size on the server, because Ollama's default cuts a long prompt short without an error and its API has no parameter for the size.
- An example config for the secondary reference, which sets the values where it differs from the defaults. A profile is a config file, and this is the first one lablet ships.

Acceptance: the same config with only the `model` section changed completes the same task on a local Ollama model and passes the same scenarios. Scenarios P3, P4, P7, P9, and P10 pass; P5 and the capture are recorded manually.

## Phase 10: Features

- Skills loaded through the loop's `skill` tool, with inlining as `prompt.skills_mode: inline`, the skills that were loaded on the wide event, and `lablet.skills.digest`. The phase opens with the design commit that settles the wording of the list and its size. Pricing and cost on the root span and wide event, counting what failed attempts reported. The `task_complete` schema from config, with the argument checked against it.
- `run.transcript_format: atif` exporting the transcript as an ATIF v1.8 trajectory for Harbor and Terminal-bench, available to the library and the CLI alike.

Acceptance: scenarios S1 to S5 pass. An ATIF export of a fake run validates against Harbor's Pydantic models.

## Phase 11: Hardening and release

- User docs in `lablet/docs/`: getting started, config reference generated from the schema, telemetry reference generated from the registry, a page on using `provider-fake` to test a framework, example configs for each provider and for the MCP optimisation use case, and a page of lablet's known differences from its references that says when to run the real harness instead and how.
- `cargo xtask bench`: criterion benchmarks for loop overhead per turn and per tool call with the regression threshold in CI.
- Release workflow publishing static Linux (musl, rustls) and macOS binaries; `cargo install` works from the repo. First `CHANGELOG.md` release section.

Acceptance: a new user can follow `lablet/docs/getting-started.md` from clone to a traced run in under five minutes without reading the spec, verified and timed by someone who didn't write it. The release checklist in `acceptance.md` is signed off once.

## Phase 12: After the release

Features that pass the first rule of `decisions.md`, 2026-09-28, and wait for the release. Each opens with its own design commit, as phase 3c does. The order below is the one expected today, and the transfer checks may change it.

- Tool search: tool schemas kept back until the model searches for them, with the tools offered to each turn recorded.
- Summarisation: the conversation summarised by a provider call that isn't a turn, and a request that's too long compacted and sent again.
- Resuming after a response is cut off at `max_tokens`.
- Reconnecting a remote MCP server.
- An image in a tool result passed through to the model.

Acceptance: each feature names its scenarios in its design commit. It closes when its row in the parity matrix reads `same`, or names the difference that's left.

## Exit criteria for every phase

A phase is closed when all of these hold, in addition to its acceptance line:

1. Every automated scenario assigned to the phase is green in CI, not only locally. Manual items (below) are recorded by a human at the phase review.
2. `cargo xtask pre-push` passes, and no new `#[expect]` suppression lacks a reason tied to the phase.
3. The spec and the code agree. A deviation is a spec edit in the same phase, with a `decisions.md` entry if it changes a decision.
4. The traceability table in `acceptance.md` has no phase-assigned statement without a test or gate.
5. The phase report contains one demo command a reviewer can run in under a minute.
6. Every new dependency has a justification comment and `cargo deny` is clean.
7. No scenario has been moved to a later phase to close this one.

Human sign-off, which the building agent can't do itself: phase 6 the Jaeger check and the file replay via docker compose; phase 7 the real Anthropic run and the capture of the primary reference; phase 8 the public MCP server run and the capture of the MCP rows; phase 9 the Ollama run (P5) and the capture of the secondary reference; phase 11 the timed getting-started walk and the release checklist, the transfer check among its items.

Delivery: work lands on `main` in small logical units, each fast-forwarded from a branch after `cargo xtask pre-push` passes locally, with the Actions run checked after the push. No pull requests for now. The bullet list of a phase is its landing plan. The building agent goes as far as it can in a phase and stops where a human is needed. A phase closes with a review of its diff that's scaled to risk and budgeted (see `contributing/README.md`, Reviews), a phase report (what landed, scenarios green, gate results, review cost against budget, architectural decisions, spec clarifications, human sign-off items, open risks, one demo command), and a stop for human review; the next phase starts only on an explicit go-ahead.

Signals of drift, any of which means stop and fix before continuing: the `lablet.*` attribute count grows without decisions entries; an adapter crate imports another adapter; tests exercise only the CLI rather than the ports; a port gains a method that only one fake needs.

## Working rules for the building agent

- Never move to the next phase with failing gates.
- Clarify the spec freely when it has a gap or an inconsistency: edit it alongside the code and say so in the commit message. Architectural decisions may be made without asking; record each in `product/decisions.md` when made and list it in the phase report.
- If commit signing fails, stop and wait. Never handle secrets; prepare the command for the human.
- Keep dependencies minimal and justify each new one with a comment in `Cargo.toml`.
- Prefer a small change to the spec, made explicitly, over a workaround that makes the code disagree with it.
- When a normative sentence in the spec has no test or gate, add a row to the traceability table in `acceptance.md` before implementing it.
