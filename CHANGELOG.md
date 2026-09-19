# Changelog

All notable changes to lablet are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). The workspace has one version. The public contract is the config schema, the outcome JSON, and the telemetry registry; a breaking change to any of them is a major version bump.

An entry under `Unreleased` is mandatory for any change to one of the three contract files, and `cargo xtask changelog` fails without it:

- `lablet/schema.json`, the config schema
- `lablet/telemetry/registry/`, the telemetry registry
- `lablet/tests/fixtures/outcome.json`, the outcome JSON fixture

## [Unreleased]

### Added

- Project scaffold: the `lablet/` Cargo workspace with an empty crate for every ring of the architecture, the root `xtask` gate crate (`lint-layers`, `lint-manifests`, `fmt`, `clippy`, `deny`, `doc`, `test`, `coverage`, `mutants`, `changelog`, `pre-commit`, `pre-push`, `ci`), the pinned Rust toolchain and `mise.toml` gate tools, `scripts/setup.sh` and the git hooks, the `cargo deny` policy, and the GitHub Actions workflow.
- Telemetry contract plumbing: the Weaver registry under `lablet/telemetry/registry/` with its justification policy, the core and GenAI semantic conventions and the shared Weaver policies and templates vendored under `lablet/telemetry/deps/` at pinned commits, `cargo xtask weaver check` in the pre-commit gate, and `cargo xtask weaver vendor [--check]`.
- Telemetry contract: the registry declares the `lablet.invoke_agent`, `lablet.chat`, and `lablet.execute_tool` spans, the `lablet.run` wide event, the `lablet.retry` span event, the `gen_ai.client.operation.exception` and `gen_ai.client.inference.operation.details` log records, and every `lablet.*` attribute with its justification. `cargo xtask weaver generate` renders the `lablet-telemetry-registry` crate and the reference under `lablet/docs/telemetry/` from it, and `cargo xtask weaver generate --check` holds them to the registry in the pre-commit gate.
