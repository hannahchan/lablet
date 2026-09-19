# lablet workspace

The Cargo workspace that builds lablet: a lightweight, instrumented agent loop, as a library and a `lablet` binary. What it does and why is in [../product/](../product/); how to work on the code is in [../contributing/README.md](../contributing/README.md). User-facing docs will live in [docs/](docs/).

Every crate but one is an empty shell from the phase 0 scaffold, and its doc comment names the [build-plan](../product/build-plan.md) phase that fills it. The exception is `telemetry-registry`, whose sources `cargo xtask weaver generate` writes from the registry in `telemetry/`.

## Layout

Explicit architecture: a ring is a directory prefix, and directory `foo/bar/` is package `lablet-bar`. The two exceptions are `apps/lablet` (package `lablet`) and the test-support crates, whose package names are in the table.

| Path                                                                                                                                     | Ring                  | May depend on                                     |
| ---------------------------------------------------------------------------------------------------------------------------------------- | --------------------- | ------------------------------------------------- |
| `crates/domain/model`, `crates/domain/policy`                                                                                            | Domain                | domain                                            |
| `crates/application/run`                                                                                                                 | Application           | domain                                            |
| `crates/adapters/secondary/*` (`provider-anthropic`, `provider-openai`, `provider-fake`, `tools-builtin`, `tools-mcp`, `telemetry-otel`) | Secondary adapters    | application, domain, the adapter shared kernel    |
| `crates/adapters/secondary/shared/telemetry-registry`                                                                                    | Adapter shared kernel | application, domain, other adapter shared kernels |
| `apps/lablet`                                                                                                                            | Composition root      | everything except test support                    |
| `tests/conformance` (`lablet-conformance`), `tests/mcp-server` (`lablet-test-mcp-server`)                                                | Test support          | anything; used as dev-dependencies only           |

Domain crates may not use tokio, reqwest, tracing, opentelemetry, or rmcp; the application crate may not use tokio, reqwest, opentelemetry, or rmcp (`tracing` is allowed there). The gate also refuses tonic, axum, and hyper in both rings, and matches a whole crate family by name, so `opentelemetry` covers `opentelemetry_sdk` and `tracing-opentelemetry` alike. `serde` and `serde_json` are allowed everywhere. An adapter never depends on another adapter, and there is no primary adapter ring: a crate under `crates/adapters/primary/` fails the gate as a member in no ring. `cargo xtask lint-layers` enforces all of this on `[dependencies]` and `[build-dependencies]`; dev-dependencies are exempt from the ring rules. It reads a dependency's real name and path from `[workspace.dependencies]`, so a dependency it can't find there, dev ones included, fails the gate rather than passing unread. The full rules are in [../contributing/README.md](../contributing/README.md).

Also here: `deny.toml` (the `cargo deny` policy) and `telemetry/` (the Weaver registry the telemetry crate and docs are generated from, its policies, and the vendored upstream registries it depends on).

## Dependencies

Every dependency is declared once, in `[workspace.dependencies]` in [Cargo.toml](Cargo.toml), with a comment on the line directly above it saying why it's there: third-party crates pinned to exact versions, workspace crates as `{ path, version }` with the path as `members` lists it. A member takes one with `<name>.workspace = true` (adding only `features`, `optional`, or `default-features`) and declares nothing of its own, in any dependency table, so its manifest needs no comments. A version bump is its own commit. The lints refuse the Cargo features lablet doesn't use (member globs, `exclude`, `[patch]`, and the like) instead of modelling them; the message says so. The gates run cargo with `--locked`, so after editing a manifest refresh the lockfile (any plain cargo command here, or in `../xtask/` for xtask's own) and commit it with the manifest.

## Running things

Plain cargo commands run from this directory:

```bash
cd lablet
cargo check --workspace --all-targets
cargo test -p lablet-run
```

The gates are `cargo xtask` commands. `xtask/` is a separate crate at the repository root, not a member of this workspace, reached through a cargo alias:

```bash
cargo xtask pre-commit    # fmt, clippy, lint-layers, lint-manifests, weaver check, weaver generate --check, lint-shell, lint-prose
cargo xtask pre-push      # pre-commit plus cargo deny, changelog, rustdoc, tests
```

Run them from the repository root. The alias is also defined in `.cargo/config.toml` here, so the same commands work from `lablet/` itself, but not from a crate directory below it.
