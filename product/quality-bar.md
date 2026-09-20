# Quality bar

What lablet commits to as a product for developers and AI engineers, and what the repository commits to so those promises hold. Every item here is either verifiable by a user or enforced by a gate; adjectives don't belong on this page.

## Commitments to users

1. **Three stable contracts, one borrowed.** The config schema, the outcome JSON, and the telemetry registry are lablet's own surface. The file output is OTLP/JSON, the OpenTelemetry Collector's own file format, so it's a contract someone else maintains and lablet promises only to conform to it. Each is versioned, every change has a changelog entry, and a breaking change to any of them is a major version bump. The config JSON schema and the telemetry registry are checked in so changes are reviewable diffs.
2. **First traced run in under five minutes.** One binary via GitHub releases or `cargo install`. `lablet init` writes a working config. `lablet/examples/` has a docker compose for a local collector and Jaeger. The getting-started page is walked end to end before each release by someone who didn't write it.
3. **Errors that say what to do.** Config errors name the key, the line, the value, and the accepted values. Startup errors distinguish a bad config from an MCP server that didn't start from a provider that rejected the key. `lablet check` catches everything that can be caught without a model call, including starting MCP servers and listing their tools. `lablet check --resolved` prints the fully resolved config with no hidden defaults.
4. **Determinism where possible.** The same config and prompt give the same digest, tool list, and telemetry key set. The seed, when one was used, and the finish reasons are in the wide event so variance can be seen.
5. **Telemetry that lands in what you already run.** OTLP that appears correctly in Jaeger, Grafana Tempo, Honeycomb, and Langfuse without custom mapping. The OTLP path is verified in CI against an in-process receiver and Weaver live-check; the collector, Jaeger, and file-replay path is a per-phase human sign-off, and the rest is on the release checklist. Attributes follow the GenAI semantic conventions; the `lablet.` extensions are defined in the registry and documented from it.
6. **Library parity.** Anything the CLI does, `build` and `run` do. Nothing lives only in `main.rs`. Doc examples compile and run as doctests.
7. **A fake provider is part of the product.** `provider-fake` plays scripted responses so users can test their own frameworks without spending tokens. It's what lablet's own smoke tests and doc examples run on.
8. **Runs where the loop runs.** Static Linux binary for containers, macOS and Linux release artifacts, no runtime dependencies beyond the MCP servers you configure.
9. **Unsurprising security defaults.** Secrets only via environment. Built-in file and bash tools sandboxed to a root. Content capture off by default and documented as the one flag that puts prompts into telemetry.

## Commitments in the repository

10. **The gates are the bar.** Every rule in `contributing/` is enforced by `cargo xtask` or is labelled a review convention. Nothing is "please remember to."
11. **Contract-first telemetry.** Every span, event, attribute, and the wide event is declared in an OpenTelemetry Weaver registry before it's emitted. Rust attribute name constants, enums, and per-signal key lists are generated from the registry, the registry is checked for policy, and emitted telemetry is validated against it with `weaver registry live-check` in CI. Documentation is generated from the same source. An attribute can't exist without being declared and documented.
12. **Coverage and mutation testing on the core.** Domain crates carry a 100% line and region coverage floor, the application crate 90% of each, and all three an 80% mutation score floor in CI. Regions are the stricter coverage measure: they count each match arm and each `else` a line never spells out apart, where a line counts as covered when any region on it ran. Adapters are held by conformance suites and recorded HTTP tests instead of a number.
13. **Spec and code agree mechanically.** Config reference generated from the schema, telemetry reference generated from the registry, stop reasons rendered from the enum. Regeneration in CI must produce no diff.
14. **Lablet's own overhead is measured.** Criterion benchmarks for loop overhead per turn and per tool call, with a 20% regression threshold in CI.
15. **A small, boring dependency tree.** Every dependency justified in `Cargo.toml`, `cargo deny` for licences and advisories, a documented MSRV policy.

16. **Aggregatable by construction.** One wide event per run with a fixed flat shape; run id, config digest, and every composer-supplied resource attribute on every record; one mapping from events to OTel signals with pluggable exporters, so the network and the file carry identical data; the full resource on every export batch; one file per run and no state across runs; raw counts, bytes, tokens, and durations rather than derived ratios.

## Deliberately not committed

- Aggregation, reporting, or export across runs: no `report` command, no CSV, no cross-run state. The composing system owns that.
- A dashboard, trace viewer, or UI. Lablet emits into the tools people already run.
- A runtime plugin system for providers or tools. The port traits and the library are the extension point.
- A Parquet or Arrow writer, for now. The OTel adapter's exporter is pluggable, so one can be added later as a `SpanExporter` and `LogExporter` pair without touching the loop or the registry; see the open question in the spec.
