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

The resource carries [`service.name`, `service.version`](telemetry/service/entities.md), what the [OpenTelemetry SDK adds](telemetry/telemetry/entities.md), and the attributes `telemetry.resource` states, as a map or as OpenTelemetry's `k=v` text with each value's escaped characters decoded. The config's attributes have keys the composer chooses, so the registry can't declare them, and they appear on the resource only. `service.name` is the first `telemetry.resource` states, and `lablet` when it states none. No variable names an attribute: a file that wants `OTEL_SERVICE_NAME` and `OTEL_RESOURCE_ATTRIBUTES` on the resource writes `resource: service.name=${OTEL_SERVICE_NAME:-},${OTEL_RESOURCE_ATTRIBUTES:-}`. `service.version` is lablet's and the SDK's attributes are the SDK's, whatever names them. That's the `lablet` command's resource: a library run's is its host's.

`gen_ai.conversation.id`, `session.id`, and `lablet.config.digest` are on every span and log record, so any of them groups a run's data without a join. So are `lablet.task.id`, `lablet.experiment.id`, and `lablet.trial`, each when the run request named it.

Every span starts when the loop measured its call to start and lasts as long as the loop measured it to take, as finely as the clock measured it: a sub-millisecond call is a span of its own length, not of none. The span events and log records are timed the same way. The `*_ms` attributes are whole milliseconds, each cut from the time or length it measures, so a latency the wide event totals is the sum of its spans' durations each cut to whole milliseconds, and can fall short of their summed durations by under a millisecond a span.

Captured content is a JSON string in the form the GenAI conventions give its attribute, and a text longer than 1 MiB is cut there. The wide event is emitted once the run's transcript is written, through the logger every destination shares, and the `lablet` command flushes the run's telemetry once after it. So it may share an export with the run's last content records, a batch a collector refuses for its size takes it along, and in a file it isn't always the run's last line. Lablet doesn't count what's lost on the way: a processor whose queue is full drops what doesn't fit, as the OpenTelemetry SDK's do.

## Where telemetry goes

This section and the next are the `lablet` command's. A library run's telemetry goes where its host's SDK sends it ([below](#running-lablet-as-a-library)).

The `lablet` command's telemetry goes where its config's `telemetry` section names, and nowhere else. A field left out isn't stated, which is never a fallback to the environment, so a run whose config names no file and no endpoint sends and writes nothing, and its end waits for nothing. A variable gives a setting only where the file writes `${VAR}` for it ([below](#variables-and-a-pod)). `lablet check` says where a run's telemetry goes on its `telemetry:` line, as `file (line 17); OTLP http/protobuf to http://localhost:4318 (line 16)`, or `none named, so a run sends and writes nothing`, and `lablet check --resolved` prints the config as the file wrote it, each value's source in a comment, and nothing the environment holds.

A stated endpoint sends both signals there, beside the file when the config names one, over `telemetry.otlp.protocol`: HTTP/protobuf unless it says `grpc` or `http/json`. On HTTP the endpoint is a base URL to which `/v1/traces` and `/v1/logs` are appended, and a gRPC endpoint written without a scheme is given `https://`. Each export carries the exporter's own headers and `telemetry.otlp.headers`, a map or OpenTelemetry's `k=v` text, and trusts the certificates `telemetry.otlp.tls.ca_file` names in place of the platform's, with `client_cert` and `client_key` as its identity to a collector that asks for one. One export takes at most the specification's ten seconds, its retries included. With no collector listening at the endpoint, the telemetry sent there is lost, and the run's outcome is the same. The run's end costs about 0.9 s, the exporter's retries, and writes five lines on the diagnostic log: for spans and for log records, the exporter's line that it gave up and the SDK's line that the export failed, then lablet's warning that the run's telemetry wasn't exported whole. A collector that accepts the connection and never answers costs more: a run's end waits about five seconds for it, and a shutdown, and so the `lablet` command's exit, about five more.

| To send the telemetry                 | State                                                                                              |
| ------------------------------------- | -------------------------------------------------------------------------------------------------- |
| to a collector                        | `telemetry.otlp.endpoint`                                                                          |
| to a file, and to the network as well | `telemetry.file.path`, or `-` for standard error, and `telemetry.otlp.endpoint`                    |
| to a file alone                       | `telemetry.file.path`                                                                              |
| nowhere                               | neither, as by default; `--set telemetry.otlp.endpoint=null` takes back an endpoint the file names |

## Variables and a pod

No `OTEL_*` variable gives the `lablet` command a setting. `OTEL_SDK_DISABLED`, the exporter selectors, the endpoint, protocol, header, timeout, insecure and certificate variables, the sampler, the span limits, the batch processors' settings, `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`, `OTEL_PROPAGATORS` and `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` change nothing unless the file writes them. Lablet states on the SDK the value each of the SDK's own would give, the specification's defaults: the sampler parent-based always on, so a run under an inbound parent that isn't sampled exports its log records and no span, 128 of each span limit, and the batch processors' delays, queues and batch sizes. The one variable the pinned exporter reads for itself is `OTEL_EXPORTER_OTLP_COMPRESSION`, with its `TRACES` and `LOGS` forms, `gzip` or `none`, which changes how a network export is encoded and nothing in the file; `check`'s `telemetry:` line says so when one is set and an endpoint is named. `check`'s `unread:` line names each set variable lablet read before its settings came from the config alone, unless the file writes it, as `OTEL_TRACES_SAMPLER (no field yet): set, and read only where the file writes ${NAME}`.

A file maps what a platform injects where it wants it. `${VAR:-}` takes nothing when the variable is unset or empty, and a setting that comes out empty isn't stated, so this block reads the OpenTelemetry variables a cluster injects and sends nothing on a laptop where none of them is set:

```yaml
telemetry:
  propagators: ${OTEL_PROPAGATORS:-}
  otlp:
    endpoint: ${OTEL_EXPORTER_OTLP_ENDPOINT:-}
    protocol: ${OTEL_EXPORTER_OTLP_PROTOCOL:-}
    headers: ${OTEL_EXPORTER_OTLP_HEADERS:-}
    tls:
      ca_file: ${OTEL_EXPORTER_OTLP_CERTIFICATE:-}
      client_cert: ${OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE:-}
      client_key: ${OTEL_EXPORTER_OTLP_CLIENT_KEY:-}
  resource: service.name=${OTEL_SERVICE_NAME:-},${OTEL_RESOURCE_ATTRIBUTES:-}
  capture_content: ${OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT:-}
```

Where telemetry is required, drop the endpoint's `:-`, so an unset variable refuses the run. Every value of `telemetry.otlp.headers` is one of lablet's own credentials, and holds a `${VAR}`: no command a run starts inherits the variable that filled it, and its value is cut from what a tool returns. A command that exports needs a token of its own, given under `tools.builtin.credentials`, as `OTEL_EXPORTER_OTLP_HEADERS: ${COMMAND_OTLP_HEADERS:-}`. A header variable the file doesn't write is the framework's: every command inherits it uncut, and `unread:` names it. What the client key's file holds is cut from what a tool returns too, whenever `client_key` is stated, endpoint or none, and a file that can't be read is refused.

Three things the `lablet` command reads of its environment aren't settings. The inbound context, through the propagators `telemetry.propagators` names, `tracecontext` and `baggage` by default, and `b3` and `b3multi` besides, or `none`: each key a propagator reads is read from the variable its name normalises to, as `traceparent` from `TRACEPARENT`. With a valid parent the run is its child, and a `TRACESTATE` is on each of its spans. `BAGGAGE` is read, emitted nowhere, and given to the commands the run starts; a member that doesn't parse, or a value over the propagator's 8,192 bytes, is warned about by the variable's name alone. The process conventions: `RUST_LOG`, which the diagnostic log follows, `PATH`, and the proxy variables, `SSL_CERT_FILE`, `SSL_CERT_DIR` and `TOKIO_WORKER_THREADS`, which the crates read for themselves. And its whole environment, once, which every command a run starts begins from.

A command inherits that environment, less every variable that filled one of lablet's credentials and every variable that holds a credential's value, then gets `tools.builtin.env` as set and `tools.builtin.credentials` under their names. Its context variables are its tool span's, injected through the propagators as it starts: it inherits none of `TRACEPARENT`, `TRACESTATE`, `BAGGAGE`, `B3` and the `X_B3_*` variables, a variable `tools.builtin.env` states wins over the one injected, and under `telemetry.propagators: none` a command gets none but those it states. So with the default propagators, a `lablet run` that a command starts is a child of that command's tool span.

`lablet check` refuses, as `lablet run` does, before any run: an endpoint the exporter doesn't accept; a header in `telemetry.otlp.headers` whose name or value no header may have, or whose value holds no `${VAR}`; a URL whose text writes a user or a password out, or that a variable fills with either, since each is its own `${VAR}`, as `https://${GW_USER}:${GW_PASS}@gw/v1`; a client key's file that can't be read, whenever it's stated; and a certificate, client key or client certificate file that can't be loaded, or a key or certificate named without the other, each loaded whenever an endpoint is named. The message names the setting and the variable that filled it, or shows the endpoint as the file writes it, and never what a variable or a file holds.

## Rules of the contract

- **Semantic conventions first.** Where the core or GenAI semantic conventions define an attribute, lablet uses it under its own name and never adds a `lablet.*` twin.
- **Every `lablet.*` attribute is justified.** Its entry in the registry has a note that opens with `Justification:` and says why no convention covers it. The reference shows the note beside the attribute, and `cargo xtask weaver check` fails an entry without one.
- **Every attribute on a signal has a requirement level.** Required attributes are always present. A conditionally required one states its condition. Captured content is opt-in, and absent rather than blanked when capture is off.
- **Only raw values.** Counts, bytes, tokens, and durations in milliseconds. Ratios and averages belong to whoever aggregates.
- **Flat shape.** No nested maps. Per-tool values are template attributes such as `lablet.tool.calls.<tool name>`. The wide event has them for a tool the run offered and called, so the names in `lablet.tools.names` bound the keys.
- **Absent, never blank.** An attribute whose condition doesn't hold of a run is left out: the turn cap of a run without one, the cost of a run that isn't priced, a count the provider didn't report. A `0` is a count of zero.

A rename in the conventions lablet depends on is a breaking change to this contract, as is any change to a `lablet.*` attribute, and both get a `CHANGELOG.md` entry.

## Adding or changing an attribute

1. Look for a semantic-convention attribute first, in the vendored registries under `lablet/telemetry/deps/`.
2. Edit the registry: a new `lablet.*` attribute goes in `shared/attributes.yaml` with its justification, and every span or event that carries it, in the folder of the crate that emits it (`application/run/`, for every signal today), refers to it with a requirement level.
3. Run `cargo xtask weaver check`, then `cargo xtask weaver generate`, which writes the generated telemetry module of the crate that emits the signal, `lablet-run` for every signal today, and the reference again. The struct of the signal gains a field.
4. Fill the field where the crate builds the struct. The build fails until it's filled, which is the point: an attribute added to `lablet.run` is a field of the generated `LabletRun` that `lablet-run`'s runner must say the value of. Attribute names never appear as string literals; tests read them, and each signal's lists of required and declared keys, from the `key` module beside the struct.
5. Add an entry under `Unreleased` in `CHANGELOG.md`.

`cargo xtask pre-commit` runs the check and fails when the generated files are out of date.

## Running lablet as a library

A library run goes to the OpenTelemetry its host provides. A host builds a `Lablet` with `Lablet::builder(config, otel)`, or with `lablet::build(config, otel)`, where `otel` is a `lablet::Otel`: `Otel::new(tracer_provider, logger_provider, propagator)`, the three pieces a run's spans, its records and a command's context go through. Each is required, and lablet reads no global tracer provider or propagator of OpenTelemetry's: a host that wants none of a piece hands in the API's no-op for it, and `Otel::noop()` is the no-op of all three. A host on the global ones hands them in itself: `opentelemetry::global::tracer_provider()`, called once the host has set its global tracer provider, since the `Otel` takes its tracer when it's made, and the propagator it gave `set_text_map_propagator`, made again, or one of its own that asks the global one at each call and names the global one's fields in its own. A run's root span is the child of the context that's current where `run` is awaited, so a run under a span the host has open is in the host's trace, and the host's sampler decides whether it's recorded. The host's SDK exports and flushes: a run returns without flushing anything, and `Lablet::shutdown` does no telemetry work. So a host hands `Otel::new` clones of its providers and keeps the handles, to flush them when it wants the telemetry sent and to shut them down after the last run. The tracer and the logger an `Otel` takes keep the providers they came from alive, so a provider handed in by value can't be flushed, and is shut down only when the last `Lablet` and `Otel` that hold it drop, taking an in-memory exporter's spans and records with it.

Library mode leaves alone what configures an SDK, and says nothing of it: the config's `telemetry.file`, `telemetry.otlp`, `telemetry.propagators` and `telemetry.resource`. The library reads no environment variable: a host that keeps the command line's file fills its `${VAR}`s through `RawConfig::substituted`, places its relative paths with `Config::relative_to`, and hands the environment its commands start from to `Builder::with_process_environment`. `telemetry.capture_content` alone decides whether content is captured. Lablet's credentials are still kept from commands and cut from what they print, though no SDK uses them, `model.api_key`, the values of `telemetry.otlp.headers` and the user information of `telemetry.otlp.endpoint` among them, and each entry of `tools.builtin.credentials` is given to `bash` under its name. A command's context is its tool span's, injected through the propagator the `Lablet` was built with.

The host's own `tracing` subscriber sees lablet's diagnostics, such as a transcript that wasn't written, and none of its telemetry, which never passes through `tracing`. Two `Lablet`s in one process, each given providers of its own, each reach only their own. A run is stopped through its `CancelHandle`; dropping the future `Lablet::run` returns abandons it with no outcome. Dropped before the loop returns, it leaves no transcript or wide event either; dropped during the transcript write, it still leaves the transcript and no wide event.

The `traced_run` example is a host with an SDK of its own, which runs lablet beneath a span of its own and sends both over OTLP to `http://localhost:4318`, or where `OTEL_EXPORTER_OTLP_ENDPOINT` says, as the host's own SDK reads it (see [../examples/](../examples/README.md)).
