# Telemetry

Lablet's telemetry is a contract. An [OpenTelemetry Weaver](https://github.com/open-telemetry/weaver) registry in [`lablet/telemetry/registry/`](../telemetry/registry/) declares every span, log record, and attribute before any code emits it, and the reference in [telemetry/](telemetry/README.md) is generated from that registry. The reference is the place to look up an attribute's type, meaning, and requirement level.

## What a run emits

| Signal                                                                                                          | Kind                | When                                        |
| --------------------------------------------------------------------------------------------------------------- | ------------------- | ------------------------------------------- |
| [`lablet.invoke_agent`](telemetry/lablet/spans.md#labletinvoke_agent)                                           | span, the root      | once for each run                           |
| [`lablet.chat`](telemetry/lablet/spans.md#labletchat)                                                           | span, child of root | once for each attempt of a provider call    |
| [`lablet.execute_tool`](telemetry/lablet/spans.md#labletexecute_tool)                                           | span, child of root | once for each tool call                     |
| [`lablet.run`](telemetry/lablet/events.md#labletrun)                                                            | log record          | once, when the run ends: the wide event     |
| [`lablet.retry`](telemetry/lablet/events.md#labletretry)                                                        | span event          | on the chat span of a failed provider call  |
| [`gen_ai.client.operation.exception`](telemetry/gen-ai/events.md#gen_aiclientoperationexception)                | log record          | with each failed provider call              |
| [`gen_ai.client.inference.operation.details`](telemetry/gen-ai/events.md#gen_aiclientinferenceoperationdetails) | log record          | only when `telemetry.capture_content` is on |

The resource carries [`service.name`, `service.version`](telemetry/service/entities.md), what the [OpenTelemetry SDK adds](telemetry/telemetry/entities.md), and the `telemetry.resource` attributes from the config. Those last ones have keys the composer chooses, so the registry can't declare them, and they appear on the resource only.

`gen_ai.conversation.id`, `session.id`, and `lablet.config.digest` are on every span and log record, so any of them groups a run's data without a join.

## Rules of the contract

- **Semantic conventions first.** Where the core or GenAI semantic conventions define an attribute, lablet uses it under its own name and never adds a `lablet.*` twin.
- **Every `lablet.*` attribute is justified.** Its entry in the registry has a note that opens with `Justification:` and says why no convention covers it. The reference shows the note beside the attribute, and `cargo xtask weaver check` fails an entry without one.
- **Every attribute on a signal has a requirement level.** Required attributes are always present. A conditionally required one states its condition. Captured content is opt-in, and absent rather than blanked when capture is off.
- **Only raw values.** Counts, bytes, tokens, and durations in milliseconds. Ratios and averages belong to whoever aggregates.
- **Flat shape.** No nested maps. Per-tool values are template attributes such as `lablet.tool.calls.<tool name>`.

A rename in the conventions lablet depends on is a breaking change to this contract, as is any change to a `lablet.*` attribute, and both get a `CHANGELOG.md` entry.

## Adding or changing an attribute

1. Look for a semantic-convention attribute first, in the vendored registries under `lablet/telemetry/deps/`.
2. Edit the registry: a new `lablet.*` attribute goes in `attributes.yaml` with its justification, and every span or event that carries it refers to it with a requirement level.
3. Run `cargo xtask weaver check`, then `cargo xtask weaver generate`, which writes the `lablet-telemetry-registry` crate and the reference again.
4. Use the generated constant in the code. Attribute names never appear as string literals.
5. Add an entry under `Unreleased` in `CHANGELOG.md`.

`cargo xtask pre-commit` runs the check and fails when the generated files are out of date.
