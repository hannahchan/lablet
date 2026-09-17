# Quality bar

What lablet commits to as a product for developers and AI engineers, and what the repository commits to so those promises hold. Every item here is either verifiable by a user or enforced by a gate; adjectives do not belong on this page.

## Commitments to users

1. **Three stable contracts.** The config schema, the outcome JSON, and the telemetry registry are the product surface. Each is versioned, every change has a changelog entry, and a breaking change to any of them is a major version bump. The config JSON schema and the telemetry registry are checked in so changes are reviewable diffs.
2. **First traced run in under five minutes.** One binary via GitHub releases or `cargo install`. `lablet init` writes a working config. `lablet/examples/` has a docker compose for a local collector and Jaeger. The getting-started page is walked end to end before each release by someone who did not write it.
3. **Errors that say what to do.** Config errors name the key, the line, the value, and the accepted values. Startup errors distinguish a bad config from an MCP server that did not start from a provider that rejected the key. `lablet check` catches everything that can be caught without a model call, including starting MCP servers and listing their tools. `lablet check --resolved` prints the fully resolved config with no hidden defaults.
4. **Determinism where possible, honesty where not.** The same config and prompt give the same digest, tool list, and telemetry shape. Model variance is reported, never hidden.
5. **Telemetry that lands in what you already run.** OTLP that appears correctly in Jaeger, Grafana Tempo, Honeycomb, and Langfuse without custom mapping, verified against a real collector in CI. Attributes follow the GenAI semantic conventions; the `lablet.` extensions are defined in the registry and documented from it.
6. **Library parity.** Anything the CLI does, `build` and `run` do. Nothing lives only in `main.rs`. Doc examples compile and run as doctests.
7. **A fake provider is part of the product.** `provider-fake` plays scripted responses so users can test their own frameworks without spending tokens. It is what lablet's own smoke tests and doc examples run on.
8. **Runs where the loop runs.** Static Linux binary for containers, macOS and Linux release artifacts, no runtime dependencies beyond the MCP servers you configure.
9. **Unsurprising security defaults.** Secrets only via environment. Built-in file and bash tools sandboxed to a root. Content capture off by default and documented as the one flag that puts prompts into telemetry.

## Commitments in the repository

10. **The gates are the bar.** Every rule in `contributing/` is enforced by `cargo xtask` or it is not a rule. Nothing is "please remember to".
11. **Contract-first telemetry.** Every span, event, attribute, and the wide event is declared in an OpenTelemetry Weaver registry before it is emitted. Rust attribute constants and typed builders are generated from the registry, the registry is checked for policy, and emitted telemetry is validated against it with `weaver registry live-check` in CI. Documentation is generated from the same source. An attribute cannot exist without being declared and documented.
12. **Coverage and mutation testing on the core.** Domain and application crates carry a line coverage floor and a mutation score floor in CI. Adapters are held by conformance suites and recorded HTTP tests instead of a number.
13. **Spec and code agree mechanically.** Config reference generated from the schema, telemetry reference generated from the registry, stop reasons rendered from the enum. Regeneration in CI must produce no diff.
14. **Lablet's own overhead is measured.** Criterion benchmarks for loop overhead per turn and per tool call, tracked so lablet stays negligible next to what it measures.
15. **A small, boring dependency tree.** Every dependency justified in `Cargo.toml`, `cargo deny` for licences and advisories, a documented MSRV policy.

## Deliberately not committed

- A runtime plugin system for providers or tools. The port traits and the library are the extension point.
- A dashboard or UI. Lablet emits into the tools people already run.
