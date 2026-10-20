# Examples

Configs to run lablet with, a collector to send the runs to, and a host that runs lablet as a library. A relative path in a config starts at the config's directory, and the commands below name each config by its file name, so run each from the config's directory. The `cargo run` lines below build lablet on their first use.

The `lablet` command takes its settings from its config alone, but it reads the inbound context from its environment, and the SDK of the library host below takes the OpenTelemetry settings of the environment it runs in. So run these steps in a shell where no context variable is set, `TRACEPARENT`, `TRACESTATE`, `BAGGAGE`, `B3` or an `X_B3_*` one, since a `TRACEPARENT` makes the command's run its child, and no `OTEL_*` variable either: an endpoint there changes where the host's runs are sent, and `OTEL_EXPORTER_OTLP_COMPRESSION`, which the exporter reads for itself, changes how the command's are encoded. `env | grep -E '^(OTEL_|TRACEPARENT|TRACESTATE|BAGGAGE|B3|X_B3_)'` lists any.

## A two-turn run (`two-turns/`)

A fake-provider config whose script answers the first provider call with a `read_file` call and the second with text, so a run has two turns, a tool span, and a turn 2 to find. It needs no key.

```bash
cd lablet/examples/two-turns
cargo run --locked --manifest-path ../../Cargo.toml --bin lablet -- run --config lablet.yaml --prompt "Read notes.md and say what it holds."
```

The config names no file and no collector, so the run sends and writes no telemetry, and its end waits for nothing. To keep it in a file, add `--set telemetry.file.path=lablet-run.otlp.jsonl`; git ignores a file named `lablet-*.otlp.jsonl`. `cargo xtask weaver live-check` runs this config too, against the registry's live checker, naming its endpoint.

## A retried call with pricing (`retry/`)

A fake-provider config whose script answers the first provider call with a retryable failure, `529 overloaded`, and the second with text and cache counts, under a price list, so a run has one failed attempt, one retry after a short wait, and a cost. It needs no key.

```bash
cd lablet/examples/retry
cargo run --locked --manifest-path ../../Cargo.toml --bin lablet -- run --config lablet.yaml --prompt "Say hello." --set telemetry.otlp.endpoint=http://localhost:14317 --set telemetry.otlp.protocol=grpc
```

With the collector and Jaeger below running, the trace has two `chat scripted` spans, one for each attempt, and the first carries the span event `lablet.retry` with `lablet.retry.will_retry=true` and the wait in `lablet.retry.backoff_ms`. The failed attempt's record, `gen_ai.client.operation.exception` with `exception.type=retryable`, and the wide event `lablet.run` are in the collector's output, since Jaeger stores no logs: the wide event carries `lablet.run.cost_usd`, the price list as `lablet.pricing.*`, `lablet.provider.retries=1`, and what the failed attempt used as `lablet.provider.failed.input_tokens`. `cargo xtask weaver live-check` runs this config too, named `--task retry --experiment live-check --trial 1`, and asks its report for the failed attempt's record and the span event.

## A run against Anthropic (`anthropic.yaml`)

A config whose model is reached through Anthropic's Messages API, with the three built-in tools over `work/`, which holds two short files. It needs a key: the config reads it from `ANTHROPIC_API_KEY`, which you export in your shell, and the key is never written to a file; the run drops its variable from every command and cuts its value from every result. The prompt below asks for two reads in one go, then a write, so a model may make two `read_file` calls in one response and a `write_file` after them.

```bash
export ANTHROPIC_API_KEY=...   # in your shell; lablet never writes it anywhere
cd lablet/examples
cargo run --locked --manifest-path ../Cargo.toml --bin lablet -- run --config anthropic.yaml --prompt "Read work/notes.md and work/todo.md in one go, then write work/summary.md with one line about each, and say what you wrote."
```

The run leaves `work/summary.md` behind; delete it before the next run, or keep it. `lablet check --config anthropic.yaml` says what the run would do without calling the provider, and is refused when the variable is unset. `cargo xtask weaver live-check --config lablet/examples/anthropic.yaml --prompt "..."` runs the config against the registry's live checker with the same prompt.

Two things to expect in its telemetry. The cache prefix is rendered as tools, then system, then messages, and lablet puts a breakpoint on the system prompt and on the last tool, so what a conversation shares is written once and read after, once the prefix is over the model's minimum, 512 tokens on the 5.5 models; the first real run shows what that comes to for this example. And `max_tokens` is 16,000 where lablet's default is 32,000: lablet reads each response whole, Anthropic's own clients stream above about 16,000 output tokens, and thinking counts toward the cap, so the example asks for a response a connection holds comfortably. Raise it for long writes, and expect a retry if a connection drops.

### Which model

The example and the `init --provider anthropic` starter name `claude-opus-5-5`, the current Opus. Any of these works in `model.name`, US dollars per million tokens, standard tier:

| Model id            | Input                    | Output                   | Cache read | Cache write (5 min) | Notes                                                                               |
| ------------------- | ------------------------ | ------------------------ | ---------- | ------------------- | ----------------------------------------------------------------------------------- |
| `claude-opus-5-5`   | 4                        | 20                       | 0.20       | 5                   | the default here and the API's; effort default `medium`; thinking can't be disabled |
| `claude-sonnet-5-5` | 2                        | 10                       | 0.20       | 2.50                | effort default `high`; `disabled` thinking is a 400                                 |
| `claude-sonnet-5`   | 2                        | 10                       | 0.20       | 2.50                | the previous Sonnet; 1,024-token cache minimum                                      |
| `claude-haiku-5-5`  | 0.10, or 0.50 above 100K | 0.50, or 2.50 above 100K | not listed | not listed          | effort default `medium`; counts about 30% more tokens for the same text             |
| `claude-fable-5-1`  | 10                       | 50                       | 0.25       | 12.50               | needs 30-day retention; shares the Fable rate-limit pool                            |

A model comparison states `effort` and `thinking: adaptive`, both accepted by every model in the table, so that changing `name` changes one thing: left unstated, the effort default moves with the model, and the trace can't show it, since `gen_ai.request.reasoning.level` is set only when `effort` is.

Lablet copies no price into code: `model.pricing` is your statement, and the table is a snapshot. Check [the pricing page](https://platform.claude.com/docs/en/about-claude/pricing) before trusting a row.

## A collector and Jaeger (`docker-compose.yaml`)

An OpenTelemetry Collector and Jaeger, for looking at a run. The collector takes OTLP on the host at `localhost:14317` over gRPC and `localhost:14318` over HTTP, prints everything it receives with its debug exporter, and sends the traces on to Jaeger, whose UI is at <http://localhost:16686>. Jaeger stores no logs, so the wide event and the content records are in the collector's output only: `docker compose logs collector`.

The host ports are set away from the defaults, since another process often holds 4317 on a developer's machine. To change one, edit the host side of the `ports` entries in `docker-compose.yaml`; the container side stays.

```bash
cd lablet/examples
docker compose up --detach
```

`docker compose down` stops both.

### Turn 2 in Jaeger

Run the two-turn config against the collector:

```bash
cd lablet/examples/two-turns
cargo run --locked --manifest-path ../../Cargo.toml --bin lablet -- run --config lablet.yaml --prompt "Read notes.md and say what it holds." --set telemetry.otlp.endpoint=http://localhost:14317 --set telemetry.otlp.protocol=grpc
```

In Jaeger, select the service `lablet`, put `lablet.turn=2` in the Tags field, and find traces. The one trace found holds the span `chat scripted` of turn 2 beside the root span `invoke_agent lablet`, the chat span of turn 1, and `execute_tool read_file`.

### A file replayed through the collector

Run the config with its file in `replay/`, where the collector reads it, and no network export, since the config names no endpoint:

```bash
cd lablet/examples/two-turns
cargo run --locked --manifest-path ../../Cargo.toml --bin lablet -- run --config lablet.yaml --prompt "Read notes.md and say what it holds." --set telemetry.file.path=../replay/lablet-replay.otlp.jsonl
```

The collector's OTLP/JSON file receiver reads the file from its start, and the run reaches Jaeger within seconds. It reads lines of up to 64 MiB, as `otel-collector.yaml` sets: the receiver's default is 1 MiB, and it drops a longer line without a message, which one export of a long run's content records can be. Its run id is in the outcome the run printed; find it with the tag `session.id=<run id>`, or as the newest trace of the service `lablet`.

## A host with an SDK of its own (`traced_run`)

`lablet/apps/lablet/examples/traced_run.rs` runs lablet as a library, as an application instrumented with OpenTelemetry would. It builds an SDK of its own, hands its tracer and logger providers and a trace-context propagator to the `Lablet`, opens a span of its own, and makes a fake-provider run beneath it, whose `bash` call lists an empty directory, so it needs no key. Lablet configures nothing of the host's SDK, which sends over OTLP/HTTP to `http://localhost:4318`, or where `OTEL_EXPORTER_OTLP_ENDPOINT` says, as the host's SDK reads it for itself. To send to the collector above:

```bash
cd lablet
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:14318 cargo run --locked -p lablet --example traced_run
```

It prints the run's outcome, the trace's id and where it left the transcript. In Jaeger, look the trace up by its id, or select the service `traced-run-host`, the host's name: the root span `invoke_agent lablet` is a child of the host's span `host operation`, and `execute_tool bash` is beneath it.
