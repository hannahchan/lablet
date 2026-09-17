# Contributing

How to work in this repository. The what and why live in [../product/](../product/).

## Layout

| Area | Question | Contents |
| --- | --- | --- |
| `product/` | What are we building, and why? | Brief, spec, build plan, decisions log |
| `contributing/` | How do we work? | This document |
| `lablet/` | The output | Rust workspace and user-facing docs |
| `xtask/` | Gates | Root-level crate, not a workspace member |

## Architecture rules

Explicit architecture. Inside `lablet/`, directory `foo/bar/` is package `lablet-bar`.

| Ring | Path | May depend on | Forbidden crates |
| --- | --- | --- | --- |
| Domain | `crates/domain/*` | domain | tokio, serde (derive), reqwest, tracing, opentelemetry, rmcp |
| Application | `crates/application/*` | domain | tokio, serde (derive), reqwest, opentelemetry, rmcp |
| Secondary adapters | `crates/adapters/secondary/*` | application, domain | none |
| Composition root | `apps/*` | everything | none |
| Test support | `tests/*` | anything, dev-only | none |

`cargo xtask lint-layers` enforces this and runs at pre-commit. `serde_json` (the `Value` type) is allowed everywhere.

- Ports are object-safe `async_trait` traits, `Send + Sync`, defined in the application crate that consumes them, injected as `Arc<dyn Trait>` by constructor. No DI framework, no generics over adapters.
- Adapters implement ports and map their own errors into the port's error at the `impl` boundary. Adapter types never appear in application or domain signatures.
- The composition root selects adapters from config in `build()`. Adding an adapter is a new crate plus a match arm.
- No primary adapters: the binary and library entry points call the use case directly.
- Domain crates contain types and pure functions only. Anything async is application or outward.

## Code conventions

- `thiserror` for errors. No `anyhow`. One error enum per port or boundary, owned by the layer that defines it. Variants carry `String`, not foreign error types.
- No `unwrap`, `expect`, `todo!`, `unimplemented!`, `dbg!`, or `println!` in library code. Suppress a lint with `#[expect(..., reason = "...")]`, never `#[allow]`.
- Workspace lints: `unsafe_code = "forbid"`, `missing_docs = "warn"`, clippy `all` and `pedantic` at warn. Nursery is not enabled.
- `clap` derive API in the binary.
- Every non-obvious dependency in a `Cargo.toml` carries a one-line comment saying why it is there. Group dependencies under `# Domain`, `# Application`, `# Adapters`, `# External` headers.
- Test names are sentences: `a_tool_error_does_not_consume_the_retry_budget`.
- One integration test target per crate: `tests/it/main.rs` declaring modules.
- Test doubles are hand-written fakes in the consuming crate. No mocking framework.

## Gates

```bash
cargo xtask pre-commit    # fmt, clippy, lint-layers
```

```bash
cargo xtask pre-push      # pre-commit plus tests and rustdoc
```

Install the hooks once with `scripts/install-hooks.sh`. Plain cargo commands run from `lablet/`.

## Git

- Branch off `main`; `main` is protected.
- One logical change per commit. Moves and content edits in separate commits.
- Changing a decision means a new entry in `product/decisions.md` and a spec update in the same pull request.
