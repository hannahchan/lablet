# Acceptance

Lablet is done when every scenario below is green in CI and the release checklist is signed off. Scenarios run with `provider-fake` and the in-repo test MCP server unless stated. Each names the phase that must make it pass.

## Scenarios

### Loop and completion

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| L1 | natural mode, a script that calls a tool then ends | run | `completed`, `turns` = 2, `tool_calls` = 1, exit 0 | 2 |
| L2 | explicit mode, a script that calls `task_complete` with JSON | run | `completed`, `result.structured` equals the argument | 2 |
| L3 | explicit mode, a script that ends without `task_complete` | run | `ended_without_completion`, exit 2 | 2 |
| L4 | `max_turns: 2`, a script that keeps calling tools | run | `max_turns` after turn 2 | 2 |
| L5 | `max_total_tokens` below the script's usage | run | `max_total_tokens` | 2 |
| L6 | a script whose finish reason is `max_tokens` with no tool call | run | `output_truncated` | 2 |
| L7 | `timeout: 1s`, a fake clock advancing 2s per call | run | `timeout` | 2 |

### Errors and retries

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| E1 | a script injecting 2 retryable errors then success, `max_retries: 3` | run | `completed`, `provider.retries` = 2, backoff delays follow the policy on the fake clock | 2 |
| E2 | a script injecting 4 retryable errors, `max_retries: 3` | run | `retries_exhausted` | 2 |
| E3 | a script injecting a context-exhausted error | run | `context_exhausted`, wide event still emitted | 2 |
| E4 | a script injecting a fatal error | run | `provider_error`, `error` populated | 2 |
| E5 | a tool that always errors, `max_consecutive_tool_errors: 3` | run | `tool_errors_exhausted` after 3 error results; a success between them resets the count | 2 |
| E6 | a tool that sleeps past `tool_timeout` | run | the model receives an error result; the run continues | 3 |

### Tools

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| T1 | a built-in and an MCP tool, `deny: [read_file]` | run | the provider request never lists `read_file`; the wide event's `tools.names` excludes it | 4 |
| T2 | `allow: [bash]` | run | only `bash` is listed | 3 |
| T3 | two MCP servers exposing the same tool name | check | both listed with `<server>__` prefixes | 4 |
| T4 | `read_file` called with a path outside `tools.builtin.root` | run | error result, file not read | 3 |
| T5 | an MCP server that exits after its first call | run | subsequent calls to its tools return errors naming the server; `tool_errors_exhausted` follows | 4 |
| T6 | an MCP server that never starts, `startup_timeout: 1s` | check | exit 1 with a message naming the server and distinguishing it from a config error | 4 |

### Telemetry

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| O1 | JSONL observer | any run | one line per `RunEvent`, last line is the wide event, its totals equal the sum of the per-step events | 3 |
| O2 | OTLP to a local collector | any run | root, chat, and tool spans with the registry's attributes; one `lablet.run` log record correlated to the root span | 5 |
| O3 | OTLP to a closed port | run | outcome unchanged, exit code unchanged, export failure in the diagnostic log | 5 |
| O4 | `capture_content: false` | run | no prompt, response, or tool content in any observer output | 3 |
| O5 | `capture_content: true` | run | content present in the JSONL events and in the OTel log records | 5 |
| O6 | the fake-provider CI run | `weaver registry live-check` | no undeclared or mistyped attributes | 5 |
| O7 | JSONL and OTLP both configured | run | both receive every event and the same wide event numbers | 5 |
| O8 | `transcript_path` set, any stop reason | run | the file contains the system prompt and every message including tool results | 3 |

### Config and CLI

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| C1 | an unknown key | check | exit 1, message names the key and line | 3 |
| C2 | an invalid enum value | check | message names the key, value, and accepted values | 3 |
| C3 | `--set run.max_turns=5` | check --resolved | printed config shows 5 | 3 |
| C4 | `api_key_env` pointing at an unset variable | check | exit 1, message names the variable | 3 |
| C5 | two configs differing only in an `${VAR}` value | run both | same `config.digest` | 3 |
| C6 | two configs differing in `max_turns` | run both | different `config.digest` | 3 |
| C7 | `lablet init --provider fake` | init then run | a traced run completes with no edits | 3 |
| C8 | Ctrl-C during a tool call | run | `cancelled` after the call returns, transcript and wide event written | 3 |
| C9 | the library | `build` then `run` in a doctest | same outcome as the CLI for the same config | 3 |

### Providers (recorded HTTP, wiremock)

| # | Given | When | Then | Phase |
| --- | --- | --- | --- | --- |
| P1 | Anthropic tool use response with thinking and cache usage | complete | `ToolUse`, `Thinking` with signature, cache tokens mapped; the next request replays the thinking block unchanged | 3 |
| P2 | Anthropic 429, 529, 500, timeout, `prompt is too long`, 401 | complete | retryable, retryable, retryable, retryable, context exhausted, fatal | 3 |
| P3 | OpenAI-compatible function call response with `usage` | complete | `ToolUse` and `Usage` mapped; tool results sent back as `tool` role messages | 6 |
| P4 | OpenAI-compatible `context_length_exceeded` | complete | context exhausted | 6 |
| P5 | the same task config, provider swapped to a local Ollama model | run (manual) | `completed` | 6 |

## Traceability

Every normative statement in the spec is held by a scenario above, a gate, or a unit test in the crate that owns it. The building agent adds a row here when it finds a statement with no home.

| Spec | Held by |
| --- | --- |
| §1 completion modes, stop reasons | L1 to L7, E1 to E5 |
| §1 retries | E1, E2, E5, E6, `lablet-policy` unit tests |
| §1 outcome and exit codes | L1, L3, C1 |
| §1 wide event | O1, O2, O7 |
| §1 transcript | O8, C8 |
| §2 layer rules | `cargo xtask lint-layers` |
| §3, §4 domain | `lablet-model` and `lablet-policy` unit tests, coverage and mutation floors |
| §5 ports and observer rules | O3, O4, `lablet-run` tests with fakes |
| §6 adapters | T1 to T6, P1 to P4, conformance suites |
| §6 telemetry contract | O6, `weaver registry check`, generated-files-up-to-date gate |
| §7 config, CLI, digest | C1 to C9 |
| Quality bar 1 | schema and registry checked in, changelog gate |
| Quality bar 2, 5 | release checklist |
| Quality bar 14 | criterion benchmarks in CI |

## Release checklist

Run before tagging a release and recorded in the release notes. These cannot be automated.

- [ ] Someone who did not write the docs follows `lablet/docs/getting-started.md` from clone to a traced run in under five minutes.
- [ ] A fake-provider run's traces and wide event look right in Jaeger, Grafana Tempo, Honeycomb, and Langfuse.
- [ ] P5 passes against a local Ollama model.
- [ ] A real Anthropic run against a public MCP server over stdio completes and its trace matches the registry.
- [ ] The changelog names every change to the config schema, outcome JSON, and telemetry registry since the last release, and the version bump matches.
