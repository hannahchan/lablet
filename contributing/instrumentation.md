# Instrumentation

How lablet produces its telemetry, and how to add to it. [README.md](README.md) holds the rules the gates enforce; this page explains the approach behind them, for someone new to the code.

## What lablet emits, and why it matters

Lablet exists to be measured, so its telemetry is part of the product, not a debugging aid. A run produces three kinds of signal, all following the OpenTelemetry GenAI semantic conventions:

- **Spans.** The root span `invoke_agent lablet` covers the run, and each provider call attempt (`chat`) and each tool call (`execute_tool`) has a span beneath it. Spans are for drilling into one run.
- **Log records.** A failed provider attempt has a record beside its span, and when the run captures content, the conversation is in records of its own. No span holds content.
- **The wide event.** One record, `lablet.run`, carries everything worth knowing about the run as flat attributes, so an analyst can answer most questions from one row per run. It's emitted once, after the run's transcript is written. The command line flushes the run's telemetry once after it, and a library host's SDK exports it as it decides.

Every one of these is a contract: a consumer builds queries and dashboards on the names and values, so a change to them is a breaking change, versioned like a change to the config schema.

Diagnostics are different. They're messages for the person running lablet, such as an export that failed or a flush that gave up, written through `tracing` to stderr. They're never part of the contract, and never hold content.

## The approach in six rules

**1. The registry comes first.** Every span, record and attribute is declared in the Weaver registry under `lablet/telemetry/registry/` before any code emits it. Rust code is generated from it, and CI checks what lablet actually emits against it. If it isn't in the registry, lablet doesn't emit it.

**2. OpenTelemetry first.** Spans go through the OpenTelemetry tracing API, and metrics will too when lablet has any. Log records go through lablet's own logger, a thin wrapper over OpenTelemetry's Logs Bridge API. Lablet doesn't use `tracing` for telemetry, nor `tracing-opentelemetry`.

The OpenTelemetry Rust project's guidance is to emit logs through `tracing` and its appender, and to keep application code off the Logs Bridge API. Lablet departs from it on purpose: every call to the Logs Bridge API is in generated code or in one hand-written function, so nothing hand-writes it at a call site, which is what the guidance protects against. In exchange, records are timed on the run's clock, carry lablet's instrumentation scope, hold any value the registry allows, and never pass through a host's `tracing` subscriber.

**3. Each layer uses only what it should.**

| Layer                                   | OpenTelemetry API | OpenTelemetry SDK          | `tracing` (diagnostics) |
| --------------------------------------- | ----------------- | -------------------------- | ----------------------- |
| Domain (`crates/domain/`)               | no                | no                         | no                      |
| Application (`crates/application/`)     | yes               | in tests only              | not in `lablet-run`     |
| Adapters (`crates/adapters/secondary/`) | yes               | in tests only              | yes                     |
| Root kernels (`apps/shared/`)           | yes               | in tests only              | yes                     |
| Library root (`apps/lablet`)            | yes               | in tests and examples only | yes                     |
| CLI root (`apps/lablet-cli`)            | yes               | yes                        | yes                     |

The domain has no clock and does no I/O, so it has nothing to instrument. The SDK is what a process installs, so only the CLI root, which configures the `lablet` command's, may hold it; a library host installs its own, and the library's tests and examples stand in for one. `cargo xtask lint-layers` enforces the first two columns.

**4. Time comes from the loop.** The loop measures every call on its injected `Clock`, and each span and record gets exactly those times: a span is opened with an explicit start time and ended with an explicit end time, and a record carries the time the thing it reports happened. Never let the SDK read the system clock for a span or record. The times are the loop's `Duration`s added to the run's start whole, never their whole milliseconds, so a span is as fine as the clock and the API allow, and each `*_ms` value is the whole milliseconds of the time or the length it measures, and a total sums those. This keeps lablet's own overhead out of the agent's measured latency, makes the latency the transcript and the wide event report the whole milliseconds of what the spans measured, lets tests drive time with a fake clock, and keeps a mid-run clock adjustment from skewing durations. [README.md](README.md#telemetry-is-contract-first) says what holds this and the next rule, and where each is a review convention.

**5. No global state of lablet's.** The tracer and lablet's logger are injected into the loop and the runner, never taken from OpenTelemetry's global providers there, and so is the propagator each adapter that injects context is built with, because one process may hold several `Lablet`s, each given providers of its own, and tests run in parallel. Each root resolves the pieces once and hands them in. The CLI root hands in its SDK's providers and the composite of the propagators `OTEL_PROPAGATORS` names. The library root hands in the pieces of the `lablet::Otel` its host must hand it: the tracer and the logger taken from the host's tracer provider and logger provider, and the host's propagator. Nothing of lablet's reads or sets a global, so a host on OpenTelemetry's global tracer provider and propagator reads them itself and hands them in. The root `clippy.toml` refuses every call to `opentelemetry::global`'s tracer, meter and propagator functions, so each call in test code that sets or reads a global carries an `#[expect]` of its own. The root span is the child of `Context::current()` when a run starts: in library mode the host's current context, so a run under a span the host has open joins its trace, and in the command line the inbound context it extracted from its environment through its propagators and made current around the run. An adapter injects the context that's current as its carrier needs it, and the loop makes that its call's span with `with_context`: a `bash` command's environment is given its tool span's context. The sampler is the host's SDK's in library mode, and in the command line the SDK's as the environment names it, parent-based by default: `parentbased_always_on` unless `OTEL_TRACES_SAMPLER` says otherwise. So a run under a parent that isn't sampled records no span, while its records, the wide event among them, are still exported, and the wide event's sums then have no spans to agree with.

**6. The host provides OpenTelemetry.** An application that runs lablet as a library hands a `Lablet` one `lablet::Otel` of three pieces, each required: the tracer provider its spans go to, the logger provider its records go to, and the propagator a command's context is injected through, the API's no-op for a piece it wants none of. It configures nothing else for lablet's telemetry: its SDK samples, exports and flushes what a run emits, and what configured an SDK in the config or the environment does nothing in library mode, and is never warned about. Content capture and the cut of lablet's secrets still apply. Its own `tracing` subscriber sees lablet's diagnostics and none of its telemetry. A tool executor finds the span of its own call in the current OpenTelemetry context, and a provider adapter the chat span of its attempt.

## How the registry is organised

The registry is organised by crate. A signal is declared in the folder of the crate that emits it, and that crate's generated module holds it:

- **`shared/`** holds the attributes, the resource and the join group several crates use. It generates nothing of its own.
- **`application/run/`** holds what `lablet-run` emits: the root span and the wide event, which its runner makes, and the chat and tool spans, the failed-attempt and content records, and the retry event, which the loop makes.

An adapter gets a folder when it first emits a signal of its own.

The registry's own syntax can't say everything the generator needs, so lablet adds annotations under the `lablet` key. `weaver check` runs policies that hold each one, and refuse an annotation with any other name:

| Annotation               | Where                            | What it does                                                                                                     |
| ------------------------ | -------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `emit: span_event`       | an event                         | the event is added to a span rather than written as a log record (only `lablet.retry` today)                     |
| `severity: warn`         | an event                         | the record's severity; the default is `info`                                                                     |
| `value: chat`            | a signal's attribute reference   | the attribute always has this value on this signal, so the generator writes a constant, not a field              |
| `values: [retryable, …]` | a signal's attribute reference   | the values this signal uses of a string or an open enum, as a closed enum, so a value not listed doesn't compile |
| `join: true`             | each reference in the join group | the attribute is one of the run's join keys, collected into the generated `Join` struct                          |

An annotation goes on a signal or on one of its attribute references, never on an attribute's definition in `shared/attributes.yaml`: resolution copies a definition's annotations onto every reference to it, so a value fixed there would be fixed on every signal, and the policy refuses one. A `value` fixes the attribute on every signal of its kind, so its reference is `required`, and must be one the attribute's type allows, a text for a string attribute or an open enum (one with an `_OTHER` member, such as `error.type`) and a member for a closed one; `values` may narrow a string attribute or an enum, must be members of a closed one, and name each value once; a span event takes no `severity`. A class of an open attribute is any text to the policy; the spellings tests beside the generated module, which match every variant of the domain's enums to the generated ones, hold that each is spelt as the loop spells it. The join group is inlined into each signal when the registry is resolved, so the policy holds its result: the join keys are the keys on every span and every record, each marked, and a marked key is on all of them.

## What the generator writes

`cargo xtask weaver generate` writes a `src/telemetry/generated/` module into each crate that emits signals, from the templates in `lablet/telemetry/templates/registry/rust-crate/`. The generator writes every file there: don't edit them. Beside the structs, the module holds the enums its signals choose from (each with `ALL`, `as_str` and a conversion to an attribute value), a `key` module with a constant for each attribute name the module's signals use and, for each signal, the lists of its required keys, of all its keys and of its template keys (`LABLET_RUN_REQUIRED`, `LABLET_RUN_KEYS`, `LABLET_RUN_TEMPLATES`), which tests read, and `SCHEMA_URL`; `lablet-run`'s holds the `Join` struct, which another crate's module would use from there. For each span and event it holds a struct:

- A required attribute is a field of its own type, and any other is an `Option`, so a span can't be recorded without what the registry requires of it. An `int` is an `i64`, as the wire carries it, and a template attribute a map from suffix to value.
- The six join keys are one `Join` field, built once per run from `RunContext`, so they can't differ between a run's signals.
- A fixed value isn't a field at all but an associated constant, and a signal's own values are a closed enum named for the signal and the attribute.
- An upstream enum a signal doesn't narrow is text, since generating every value the conventions list would leave code no test can reach. An enum whose members aren't all text has no Rust type, and the generator refuses it.
- A span has its `KIND`, its `name()` as the registry builds it, and `record()`, which sets its attributes on a span; a record has its `NAME` and `SEVERITY` and `record()`, which makes the log record at a time in a span's context; a span event has its `NAME` and `add_to()`.

The generated code is data, not logic. Each struct lists its attributes as pairs of key and value, with no branches, one to a line. The logic, leaving out an attribute with no value, cutting text to the length limit, turning a map into attributes, and building a span's attributes or a log record, is written by hand once in `lablet-run`'s `telemetry` module and shared by every struct; `spellings.rs` beside it converts the domain's enums to the generated ones, exhaustively, with tests over every variant. This is how the coverage and mutation floors cover generated code without excusing any: the ordinary tests that record a span reach its generated code, and the hand-written logic has tests of its own.

## Adding to the telemetry

1. Check the semantic conventions first. A core or GenAI attribute is used wherever one exists, and a `lablet.*` attribute is the last resort, with a note that opens `Justification:` and says why no convention covers it.
2. Declare it in the registry, in the folder of the crate that emits it, with its requirement level, and the annotations it needs.
3. Run `cargo xtask weaver generate`. The struct gains a field.
4. Fill the field where the crate builds the struct. If you skip this, the build fails at that call site, which is the point.
5. Add the CHANGELOG entry the registry change needs, and run `cargo xtask weaver live-check` to see the new attribute on the wire.

A call site fills a struct and hands it on; it never writes a key or calls an API by hand. For a chat attempt, the loop builds the generated `LabletChat` with the run's `Join`, the attempt's values and its `error.type` class, and records it on the span it opened, then ends the span at the time it measured. `lablet-run`'s `Runner` does the same for the root span and the wide event: it opens the root span as the child of the current context, runs the loop in it, fills `LabletInvokeAgent` from the finished run and ends the span with the run's measured duration, and fills `LabletRun` whole from the run's context and summary and emits it once, after the run's transcript is written.

## What holds it

| Check                                       | When                     | What it catches                                                                                                                                                      |
| ------------------------------------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo xtask weaver check`                  | pre-commit, pre-push, CI | a registry that breaks the conventions or lablet's policies, annotations included                                                                                    |
| `cargo xtask weaver generate --check`       | pre-commit, pre-push, CI | generated code or reference pages that differ from what the registry renders to                                                                                      |
| The build                                   | always                   | a call site that doesn't fill a field the registry added or now requires                                                                                             |
| `cargo xtask lint-layers`                   | pre-commit, pre-push, CI | the API in the domain, the SDK outside the CLI root, and either root depending on the other                                                                          |
| The golden comparison                       | tests                    | a change in what a run emits, with ids, times and attribute order normalised                                                                                         |
| The policy tests in `xtask`                 | tests                    | a rule of lablet's policies that no longer refuses the mistake it's there for                                                                                        |
| `lablet-run`'s timing scenarios             | tests                    | a loop span or record timed off the run's clock, or cut to the millisecond                                                                                           |
| The runner's and the library's timing tests | tests                    | the root span or the wide event timed off the run's clock, or cut to the millisecond                                                                                 |
| The `hosts` tests                           | tests                    | a `Lablet` whose telemetry reaches a host's subscriber or another `Lablet`'s providers, or a run that doesn't join the span its host has open                        |
| The `library_mode` tests                    | tests                    | a library run that heeds or reports what configured an SDK, or that stops capturing content or cutting a secret                                                      |
| The `globals` test                          | tests                    | a global lablet sets, or a run's spans or a command's context that reach a global set before the build, though the host handed in its own pieces or the API's no-ops |
| `tools-builtin`'s context tests             | tests                    | a command that inherits a context variable, or isn't given its tool span's context through the propagator its executor was built with                                |
| `cargo xtask clippy`                        | pre-commit, pre-push, CI | a call to OpenTelemetry's global tracer, meter or propagator functions but calls in test code, each under its own `#[expect]`                                        |
| The canaries                                | tests                    | an OpenTelemetry crate that, after an upgrade, closes a gap lablet fills, so lablet's code for it can go or must say why it stays                                    |
| The hostile golden run                      | tests                    | an OpenTelemetry crate that starts reading a variable lablet decides, or one it doesn't read                                                                         |
| `cargo xtask weaver live-check`             | CI                       | an emitted attribute or record the registry doesn't declare, or one of the wrong type                                                                                |
| The coverage and mutation floors            | CI, daily                | generated or hand-written telemetry code that no test reaches or checks                                                                                              |

## Further reading

- `product/decisions.md`, the entries of 2026-10-03 to 2026-10-07, from "The application and the adapters instrument with the OpenTelemetry API" onward, record each choice above and what it replaced; "Lablet is configured as OpenTelemetry configures an SDK" records where the sampler, the inbound context and the rest of the environment come from. The entries of 2026-10-07 and 2026-10-08, "The host provides OpenTelemetry" and "What phase 6c's landings settled," record the two composition roots, the library on its host's OpenTelemetry, and propagation into the processes a run starts.
- `product/research/weaver/typed-codegen.md` holds the spike that tried the registry's layout and the generated code.
- `product/spec.md`, §6, lists every span, record and attribute.
- The OpenTelemetry Rust project's guidance: [`docs/traces.md`](https://github.com/open-telemetry/opentelemetry-rust/blob/main/docs/traces.md) and [`docs/logs.md`](https://github.com/open-telemetry/opentelemetry-rust/blob/main/docs/logs.md).
