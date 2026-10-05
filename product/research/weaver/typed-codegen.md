# Typed telemetry per crate: spike

Date: 2026-10-05. Weaver v0.26.1, `opentelemetry` 0.33.0, `opentelemetry-appender-tracing` 0.33.0. The files are under [`spike/typed/`](spike/typed/), and `gen.sh` there regenerates them (it names the weaver binary by its absolute path).

The question was whether the registry can work the way the owner pictured it: YAML files organised by the layers and crates, generated code landing in the crate that emits each signal, typed wrappers rather than constants, and a check for drift. It was tried on a copy of the registry, never on the tree, with two stand-in crates: `spike-run` for `lablet-run` and `spike-root` for the composition root.

## What works

**The registry can be organised by crate.** Weaver loads every YAML file under the registry directory, in subdirectories too. The spike moved the four files into `shared/` (the attributes, the resource, and the join group), `application/run/` (the chat and tool spans and their events) and `apps/lablet/` (the root span and the wide event). `weaver registry check` passes with lablet's policies, and the telemetry pages render byte for byte the same as from the flat registry.

**A signal's folder is enough to route it.** The resolved schema keeps each signal's source file in `provenance.path`, so a template's filter selects a crate's signals by the prefix `lablet/telemetry/registry/<crate folder>/`. No annotation is needed for routing, and the folder layout is the routing table.

**Each crate gets its own generated module.** One `weaver registry generate` per crate, with the folder as `--param crate_dir`, writes `src/telemetry/` of that crate: `mod.rs`, `spans.rs`, `events.rs` and `enums.rs`, holding only what that crate's signals use. Both crates take 0.8 seconds together. It fits `cargo xtask weaver generate`'s staging as it is: one more entry in its list of outputs for each crate.

**The wrappers are typed.** Each span is a struct with a field for each attribute, the field an `Option` unless the registry requires it, and methods for its name, built from the registry's `name.note` (`chat {gen_ai.request.model}` becomes `format!("chat {}", self.gen_ai_request_model)`), its kind, its attributes, and `record`, which sets them on a span. Each event is a struct whose `emit` writes a `tracing` event with the record's name, lablet's target, and every field's key written out where the macro needs it, which a hand-written call can't take from a constant. An attribute with no value is left out of the record. Lablet's own enums are closed; an upstream enum is open, with an `Other(String)` variant. A template attribute, such as the wide event's per-tool counts, is a map from suffix to value.

**What it emits is what the registry declares.** A test in `spike-run` records one failed attempt through the generated types only, against the SDK's in-memory exporters: the chat span has its kind, the start and end times the caller measured, its parent, the attributes that have values and none that don't, and the retry as a span event. The exception record has its event name, the target `lablet`, severity `WARN`, its attributes, and the trace context of the chat span, which the appender took from the active context with nothing wired.

**Drift is caught twice.** Making `lablet.request.bytes` conditionally required changes the generated field to an `Option`: a staged generation differs from the committed one, which is what `weaver generate --check` already compares at pre-commit, pre-push and in CI, and once regenerated, the call site no longer compiles. A registry change that a call site has to answer is a compile error at that site.

## What the registry can't say

Each needs an annotation, which v2 allows on any signal or attribute, or a decision.

- **Whether an event is a log record or a span event.** The registry's `events:` are both. `lablet.retry` is a span event and the others are log records, so the spike annotates `lablet: {emit: span_event}`, and the generator writes `add_to(span, timestamp)` in place of `emit()`.
- **A record's severity.** The exception record is `WARN` only in prose. The spike annotates `lablet: {severity: warn}`; the default is `INFO`.
- **A value that's fixed.** `gen_ai.operation.name` is `chat` on every chat span, but the registry says so only in a brief, so the generated field is one the caller fills. Lablet's own values of `error.type` (`retryable`, `auth` and the rest) are in a brief too, and the caller writes `ErrorType::Other("retryable")`. An annotation for a fixed value would make the field a constant, and lablet's values could be declared as members.
- **Content.** The content attributes are typed `any`. The spike maps them to `String`, holding JSON, as lablet sends them today.
- **When an attribute is known.** A chat span's response attributes are known only at its end. Since every trace is sampled, no sampler reads the attributes at the start, so the struct is filled and recorded when the span ends, and the span is opened at the start for its context.

## What `tracing` can't carry

The generator knows each event's shape, so it refuses an event `tracing` can't carry with a `compile_error!` that names the Logs Bridge API: more than 32 attributes, a list-valued attribute, or a template attribute. The wide event trips the first and the third: it has 73 attributes, 70 fixed and 3 templates. With an emitter over the Logs Bridge API, which the owner leans towards, the generator would write that emitter in place of `emit()`, these limits would go, and no call site would change.

The appender's instrumentation scope is empty: no name, no version, no schema URL, as the decision of 2026-10-05 expected from its source.

## Costs

- **The floors see generated functions.** `as_str`, `name`, `attributes`, `record` and `emit` would be functions in `lablet-run`, so they'd count towards its 100% line and region floors and its mutants. Either tests reach them, or the floors leave the generated directory out by path.
- **The join keys repeat.** Every struct has the six join keys as fields of its own, since the resolved schema inlines the join group into each signal. A generated struct for the group, filled once for each run, would cut what each call site writes.
- **The templates are lablet's own.** Weaver's own typed Rust example generates metrics only.
- **Weaver's noise.** Each run prints a warning for every file in v2 syntax and for three upstream groups, which the xtask already filters.

## Recommendation

Adopt the shape for phase 6a: the registry in folders named for the crates, generated modules routed by folder, typed structs for every span and event, and the annotations above for what the registry can't say. Settle with the owner first: whether `tracing` emitters or bridge emitters come first, how the floors treat generated code, and whether a fixed value and the join group are generated or left to the call site.
