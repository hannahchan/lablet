# Acceptance

Lablet is done when every scenario below is green in CI and the release checklist is signed off. Scenarios run with `provider-fake` and `lablet-test-mcp-server` unless stated. Each names the phase that must make it pass. Scenarios in phase 3 run against the port fakes in `lablet-run`; from phase 4 they run through the library or the CLI, with telemetry assertions made on the OTLP/JSON file read back by the conformance reader until the network exporter lands in phase 6.

## Scenarios

### Loop and completion

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| L1 | natural mode, a script that calls a tool then ends | run | `completed`, `turns` = 2, `tool_calls` = 1 | 3 |
| L2 | explicit mode, a script that calls `task_complete` with JSON | run | `completed`, `result.structured` equals the argument, `tool_calls` = 0, no `ToolCallStarted` for it | 3 |
| L3 | explicit mode, a script that ends without `task_complete` | run | `ended_without_completion` | 3 |
| L4 | `max_turns: 2`, a script that keeps calling tools | run | `max_turns` after turn 2, exactly 2 provider calls | 3 |
| L5 | `max_total_tokens` below the script's cumulative usage | run | `max_total_tokens`, no further provider call | 3 |
| L6 | a script whose finish reason is `max_tokens` with no tool call | run | `output_truncated` | 3 |
| L7 | a script whose finish reason is `max_tokens` with a tool call | run | the tool runs and the loop continues | 3 |
| L8 | `timeout: 1s`, a fake clock advancing 2s per call | run | `timeout`, no provider call after the overrun | 3 |

### Errors and retries

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| E1 | a script injecting 2 retryable errors then success, `max_retries: 3` | run | `completed`, `provider.retries` = 2, backoff delays follow the policy on the fake clock | 3 |
| E2 | a script injecting 3 retryable errors on one call, `max_retries: 3` | run | `retries_exhausted` | 3 |
| E3 | a script injecting 2 retryable errors on each of 3 calls, `max_retries: 3` | run | `completed`; the budget is per call | 3 |
| E4 | a script injecting a context-exhausted error | run | `context_exhausted`, wide event still emitted | 3 |
| E5 | a script injecting a fatal error | run | `provider_error`, `error` populated | 3 |
| E6 | a script injecting a malformed payload then success | run | treated as retryable, `completed` | 3 |
| E7 | a tool that always errors, `max_consecutive_tool_errors: 3` | run | `tool_errors_exhausted` after the 3rd error result with no further provider call; a success between them resets the count | 3 |
| E8 | a script calling a tool name that's not listed | run | the model receives an error result; it counts toward the cap | 3 |
| E9 | a script calling `bash` with `sleep` past `tool_timeout` | run | the model receives an error result with kind `timeout`; the run continues | 4 |

### Tools

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| T1 | a built-in and an MCP tool, `deny: [read_file]` | run | the provider request never lists `read_file`; the wide event's `lablet.tools.names` excludes it | 8 |
| T2 | `allow: [bash]` | run | only `bash` is listed | 4 |
| T3 | two MCP servers exposing the same tool name, no `prefix_tools` | build | build error naming both servers and the tool | 8 |
| T4 | `read_file` called with a path outside `tools.builtin.root` | run | error result, file not read | 4 |
| T5 | an MCP server that exits after its first call | run | subsequent calls to its tools return errors naming the server; `tool_errors_exhausted` follows | 8 |
| T6 | an MCP server with `--hang-startup`, `startup_timeout: 1s` | check | exit 1 with an `mcp:` message naming the server | 8 |
| T7 | two MCP servers exposing the same tool name, one with `prefix_tools: true` | check | both listed, one as `<server>__<tool>` | 8 |
| T8 | one `Lablet`, two runs, one MCP server | run twice | the server process starts once; the second run's tool calls succeed; `traceparent` is present in `params._meta` of each call | 8 |

### Telemetry

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| O1 | OTLP/JSON file exporter | any run | every line parses as an OTLP `ExportTraceServiceRequest` or `ExportLogsServiceRequest` with the full resource including the composer's `telemetry.resource` attributes; `gen_ai.conversation.id`, `session.id`, and `lablet.config.digest` are on every span; exactly one `lablet.run` record whose counts, tokens, bytes, and per-tool counts equal the sums over chat and tool spans, whose chat span count equals `calls + retries`, and whose latency totals are within 5 ms of the summed span durations; the file is complete when `run` returns | 4 |
| O2 | OTLP/JSON file read back by the conformance reader | any run | root, chat, and tool spans with the registry's attributes; one `lablet.run` log record whose trace id matches the root span | 4 |
| O3 | OTLP to a closed port | run | outcome unchanged, exit code unchanged, export failure in the diagnostic log, process exits within 5s of the run ending | 6 |
| O4 | `capture_content: false` | run | no prompt, response, or tool content in any observer output | 4 |
| O5 | `capture_content: true` | run | content log records present in both the file and the network export | 6 |
| O6 | a fake-provider run | `cargo xtask weaver live-check` | no undeclared or mistyped attributes | 6 |
| O7 | OTLP to the in-process receiver | run | root, chat, tool spans and one `lablet.run` record arrive, matching O2 | 6 |
| O8 | `transcript_path` set, any stop reason | run | the file contains the system prompt and every message including tool results | 4 |
| O9 | file and OTLP network both on, same run | run | the in-process receiver and the file contain the same multiset of spans and log records after ungrouping them from their export requests | 6 |

### Config and CLI

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| C1 | an unknown key | check | exit 1, `config:` message names the key and line | 5 |
| C2 | an invalid enum value | check | message names the key, value, and accepted values | 5 |
| C3 | `--set run.max_turns=5` | check --resolved | printed config shows 5 | 5 |
| C4 | `api_key_env` naming an unset variable, provider anthropic | check | exit 1, message names the variable; the same config with provider fake passes | 5 |
| C5 | two configs differing only in a `${VAR}` value | run both | same `config.digest` | 5 |
| C6 | two configs differing in `max_turns`, and one config with a default spelled out versus omitted | run | different digests for the first pair, identical for the second | 5 |
| C7 | `lablet init --provider fake` | init then run | a traced run completes with no edits | 5 |
| C8 | the `Cancellation` port flips during a tool call | run | `cancelled` after the call returns, transcript and wide event written, no further provider call | 3 |
| C9 | the library | `build`, `run` twice, `shutdown` in a doctest | both runs complete with distinct run ids and one `RunStarted` each; outcomes are identical after removing `run_id` and `duration_ms` | 4 |
| C10 | `lablet schema`, `--prompt-file`, stdin prompt, `system_file`, `${VAR}` unset | each | schema is valid JSON Schema; each prompt source yields the same run; unset variable is a `config:` error | 5 |
| C11 | any CLI run | run | one summary line on stderr naming stop reason, turns, tokens, tool calls, duration; absent with `--quiet` | 5 |

### Providers (recorded HTTP, wiremock)

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| P1 | Anthropic tool use response with a thinking block (empty text, signature), a redacted thinking block, and cache usage | complete | `Thinking`, `RedactedThinking`, `ToolUse` mapped; cache tokens mapped; the next request replays both thinking blocks byte-identical and in order | 7 |
| P2 | Anthropic 429, 529, 500, timeout, `prompt is too long`, 401 | complete | retryable, retryable, retryable, retryable, context exhausted, fatal | 7 |
| P3 | OpenAI-compatible function call response with `usage` and `reasoning_content` | complete | `ToolUse`, `Thinking`, and `Usage` mapped; tool results sent back as `tool` role messages, one per result | 9 |
| P4 | OpenAI-compatible `context_length_exceeded`, and invalid JSON in `arguments` | complete | context exhausted; malformed | 9 |
| P5 | the same task config, provider swapped to a local Ollama model | run (manual) | `completed` | 9 |
| P6 | `cache: true` | complete | the request carries `cache_control` on the system prompt and the last tool spec; with `cache: false` it doesn't | 7 |
| P7 | a server rejecting `max_completion_tokens` | complete | the adapter retries once with `max_tokens` | 9 |

### Skills and pricing

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| S1 | two `SKILL.md` paths in `prompt.skills` | run | the system prompt sent to the provider ends with both, in order; `lablet.skills.count` = 2 | 10 |
| S2 | `pricing` set | run | `lablet.run.cost_usd` equals the policy's arithmetic on the wide event's usage | 10 |
| S3 | `pricing` unset | run | `lablet.run.cost_usd` absent | 10 |
| S4 | an `Opaque` block in a fake script | run | it's replayed unchanged in the next request | 10 |

## Traceability

Every normative statement in the spec is held by a scenario above, a gate, or a unit test in the crate that owns it. The building agent adds a row here when it finds a statement with no home.

| Spec | Held by |
| --- | --- |
| §1 loop and stop points | L4, L5, L8, E7, C8 |
| §1 completion modes, stop reasons | L1 to L8, E2, E4, E5, E7 |
| §1 retries | E1 to E3, E6, E8, E9, `lablet-policy` unit tests |
| §1 outcome and exit codes | L1, L3, C1, T6 |
| §1 wide event and aggregatability | O1, O2, O7, O9, S2, S3 |
| §1 transcript | O8, C8 |
| §2 layer rules | `cargo xtask lint-layers` |
| §3, §4 domain | `lablet-model` and `lablet-policy` unit tests, coverage and mutation floors |
| §5 ports and observer rules | O3, O4, C9, T8 (trace context handoff), `lablet-run` tests with fakes |
| §6 providers | P1 to P7, S4 |
| §6 tools | T1 to T8, E9, conformance suite |
| §6 telemetry contract | O6, `weaver registry check`, generated-files-up-to-date gate |
| §7 config, CLI, digest | C1 to C11 |
| §8 versioning | `cargo xtask changelog`, `rust-version` in the workspace manifest |
| Quality bar 1 | schema and registry checked in, changelog gate |
| Quality bar 2, 5 | release checklist |
| Quality bar 14 | criterion benchmarks with a regression threshold |

## Release checklist

Run before tagging a release and recorded in the release notes. These can't be automated.

- [ ] Someone who didn't write the docs follows `lablet/docs/getting-started.md` from clone to a traced run in under five minutes.
- [ ] A fake-provider run's traces and wide event look right in Jaeger, Grafana Tempo, Honeycomb, and Langfuse.
- [ ] P5 passes against a local Ollama model.
- [ ] A real Anthropic run against a public MCP server over stdio completes and its trace passes live-check.
- [ ] The changelog names every change to the config schema, outcome JSON, and telemetry registry since the last release, and the version bump matches.
