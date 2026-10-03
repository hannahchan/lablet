# Examples

Configs to run lablet with, and a collector to send the runs to. A relative path in a config starts at the directory lablet runs in, so run each config from its own directory. The `cargo run` lines below build lablet on their first use.

Lablet takes the OpenTelemetry settings of the environment it runs in, so run these steps in a shell where no `OTEL_*` variable is set: an endpoint there turns the network exporter on and stops the file, and a protocol there changes the transport. `env | grep ^OTEL_` lists any.

## A two-turn run (`two-turns/`)

A fake-provider config whose script answers the first provider call with a `read_file` call and the second with text, so a run has two turns, a tool span, and a turn 2 to find. It needs no key.

```bash
cd lablet/examples/two-turns
cargo run --locked --manifest-path ../../Cargo.toml --bin lablet -- run --config lablet.yaml --prompt "Read notes.md and say what it holds."
```

With no endpoint set, the run writes `lablet-<run_id>.otlp.jsonl` beside the config, which git ignores. `cargo xtask weaver live-check` runs this config too, against the registry's live checker.

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

Run the config with its file in `replay/`, where the collector reads it:

```bash
cd lablet/examples/two-turns
cargo run --locked --manifest-path ../../Cargo.toml --bin lablet -- run --config lablet.yaml --prompt "Read notes.md and say what it holds." --set telemetry.file.path=../replay/lablet-replay.otlp.jsonl
```

The collector's OTLP/JSON file receiver reads the file from its start, and the run reaches Jaeger within seconds. It reads lines of up to 64 MiB, as `otel-collector.yaml` sets: the receiver's default is 1 MiB, and it drops a longer line without a message, which one export of a long run's content records can be. Its run id is in the outcome the run printed; find it with the tag `session.id=<run id>`, or as the newest trace of the service `lablet`.
