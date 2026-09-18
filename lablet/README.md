# lablet workspace

The Cargo workspace that builds lablet: a lightweight, instrumented agent loop, as a library and a `lablet` binary. What it does and why is in [../product/](../product/); how to work on it is in [../contributing/README.md](../contributing/README.md). User-facing docs will live in [docs/](docs/).

Every crate is an empty shell from the phase 0 scaffold. Each crate's doc comment names the [build-plan](../product/build-plan.md) phase that fills it.

## Layout

Explicit architecture: a ring is a directory prefix, and directory `foo/bar/` is package `lablet-bar`. The two exceptions are `apps/lablet` (package `lablet`) and the test-support crates, whose package names are in the table.

| Path | Ring | May depend on |
| --- | --- | --- |
| `crates/domain/model`, `crates/domain/policy` | Domain | domain |
| `crates/application/run` | Application | domain |
| `crates/adapters/secondary/*` (`provider-anthropic`, `provider-openai`, `provider-fake`, `tools-builtin`, `tools-mcp`, `telemetry-otel`) | Secondary adapters | application, domain, the adapter shared kernel |
| `crates/adapters/secondary/shared/telemetry-registry` | Adapter shared kernel | application, domain |
| `apps/lablet` | Composition root | everything |
| `tests/conformance` (`lablet-conformance`), `tests/mcp-server` (`lablet-test-mcp-server`) | Test support | anything; used as dev-dependencies only |

Domain crates may not use tokio, reqwest, tracing, opentelemetry, or rmcp; the application crate may not use tokio, reqwest, opentelemetry, or rmcp (`tracing` is allowed there). The gate also refuses tonic, axum, and hyper in both rings, and matches a whole crate family by name, so `opentelemetry` covers `opentelemetry_sdk` and `tracing-opentelemetry` alike. `serde` and `serde_json` are allowed everywhere. An adapter never depends on another adapter, and there is no primary adapter ring: a crate under `crates/adapters/primary/` fails the gate as a member in no ring. `cargo xtask lint-layers` enforces all of this by crate name on `[dependencies]` and `[build-dependencies]`; dev-dependencies are exempt. The full rules are in [../contributing/README.md](../contributing/README.md).

Also here: `deny.toml` (the `cargo deny` policy) and, from phase 1, `telemetry/` (the Weaver registry the telemetry crate and docs are generated from).

## Dependencies

Third-party crates are declared once, in `[workspace.dependencies]` in [Cargo.toml](Cargo.toml), pinned to exact versions, each with a comment saying why it is there. A member takes one with `<name>.workspace = true` and repeats the reason in its own manifest. A version bump is its own commit.

## Running things

Plain cargo commands run from this directory:

```bash
cd lablet
cargo check --workspace --all-targets
cargo test -p lablet-run
```

The gates are `cargo xtask` commands. `xtask/` is a separate crate at the repository root, not a member of this workspace, reached through a cargo alias:

```bash
cargo xtask pre-commit    # fmt, clippy, lint-layers, lint-manifests
cargo xtask pre-push      # pre-commit plus tests, rustdoc, cargo deny, changelog
```

Run them from the repository root. The alias is also defined in `.cargo/config.toml` here, so the same commands work from `lablet/` itself, but not from a crate directory below it.
