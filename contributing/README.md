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

Explicit architecture. Inside `lablet/`, directory `foo/bar/` is package `lablet-bar`, except `apps/lablet` (`lablet`), `tests/conformance` (`lablet-conformance`), and `tests/mcp-server` (`lablet-test-mcp-server`).

| Ring | Path | May depend on | Forbidden crates |
| --- | --- | --- | --- |
| Domain | `crates/domain/*` | domain | tokio, reqwest, tracing, opentelemetry, rmcp, tonic, axum, hyper |
| Application | `crates/application/*` | domain | tokio, reqwest, opentelemetry, rmcp, tonic, axum, hyper |
| Secondary adapters | `crates/adapters/secondary/*` | application, domain, adapter shared kernel; never a sibling adapter | none |
| Adapter shared kernel | `crates/adapters/secondary/shared/*` | application, domain, other adapter shared kernels; never an adapter | none; may be used by sibling adapters, implements no port |
| Composition root | `apps/*` | everything except test support | none |
| Test support | `tests/*` | anything, dev-only | none |

`cargo xtask lint-layers` enforces this by crate name on `[dependencies]` and `[build-dependencies]` (dev-dependencies are exempt) and runs at pre-commit. A forbidden name covers its family (`opentelemetry` also forbids `opentelemetry_sdk`), and no crate outside `tests/` may depend on a `tests/` crate. `serde` and `serde_json` are allowed everywhere; the domain model is the one serde form of the conversation.

- Ports are object-safe `async_trait` traits, `Send + Sync`, defined in the application crate that consumes them, injected as `Arc<dyn Trait>` by constructor. No DI framework, no generics over adapters.
- Adapters implement ports and map their own errors into the port's error at the `impl` boundary. Adapter types never appear in application or domain signatures.
- The composition root selects adapters from config in `build()`. Adding an adapter is a new crate plus a match arm.
- No primary adapters: the binary and library entry points call the use case directly.
- Domain crates contain types and pure functions only. Anything async is application or outward.

## Code conventions

- `thiserror` for errors. No `anyhow`. One error enum per port or boundary, owned by the layer that defines it. Variants carry `String`, not foreign error types.
- Workspace lints. Enforced by `cargo xtask clippy` with warnings denied (pre-commit): `unsafe_code = "forbid"`, `missing_docs`, `rust_2018_idioms`, clippy `all` and `pedantic`, and `unwrap_used`, `expect_used`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr`, `allow_attributes`, `allow_attributes_without_reason`. Enforced by `cargo xtask doc` with warnings denied (pre-push and CI), because clippy never runs rustdoc: rustdoc `broken_intra_doc_links`, `private_intra_doc_links`, `redundant_explicit_links` at deny. Nursery is not enabled and pedantic has no relaxations, so every public function returning `Result` needs an `# Errors` section and every one that can panic a `# Panics` section. Suppress a lint with `#[expect(..., reason = "...")]`; `#[allow]` is itself a lint failure.
- Tests may `unwrap`: the root `clippy.toml` allows it inside `#[test]` functions and `#[cfg(test)]` modules, for both the workspace and `xtask`. In an integration target, declare every module in `tests/it/main.rs` as `#[cfg(test)] mod name;` so shared helpers are covered too.
- The outcome print in `main.rs` is the one `print_stdout`, marked with `#[expect]`.
- `clap` derive API in the binary.
- Every dependency in a `Cargo.toml` carries a one-line comment saying why it is there, checked by `cargo xtask lint-manifests`, which accepts a comment on the line above, a group header over a contiguous run, or a trailing comment. Grouping under `# Domain`, `# Application`, `# Adapters`, `# External` headers is a review convention, not a gate.
- Test names are sentences: `a_tool_error_does_not_consume_the_retry_budget`. This is a review convention, not a gate.
- One integration test target per crate: `tests/it/main.rs` declaring modules.
- Test doubles are hand-written fakes in the consuming crate. No mocking framework.

## Telemetry is contract-first

Every span, event, and attribute is declared in the Weaver registry under `lablet/telemetry/registry/` before it is emitted. The `telemetry-registry` crate (attribute name constants, enums, per-signal key lists) and `lablet/docs/telemetry/` are generated from it and checked in. Do not hand-edit generated files and do not write attribute names as string literals in adapters; add to the registry, regenerate, then use the generated constants.

```bash
cargo xtask weaver generate    # regenerate the crate and docs after editing the registry
```

## Gates

Every rule on this page is enforced by a gate. If it is not enforced, it is a suggestion, not a rule.

```bash
cargo xtask pre-commit    # fmt, clippy, lint-layers, lint-manifests; from phase 1 also weaver check and generated files up to date
```

```bash
cargo xtask pre-push      # pre-commit plus tests, rustdoc without warnings, cargo deny, changelog
```

CI runs on every pushed branch as a matrix of `cargo xtask ci` (the pre-push list), `cargo xtask coverage` (floor: 90% lines on `lablet-model`, `lablet-policy`, `lablet-run`), and `cargo xtask mutants` (floor: 80% caught on the same crates). Later phases add `cargo xtask weaver live-check` (phase 6), `cargo xtask bench` with a 20% regression threshold, and the release builds (phase 11). `xtask` is a root-level crate reached through the alias, which is defined twice, in `.cargo/config.toml` and `lablet/.cargo/config.toml`; an xtask test keeps the two in step. It does not work from a crate directory below `lablet/`.

## Versioning

One workspace version. Keep-a-changelog format in `CHANGELOG.md`. A change to `lablet/schema.json`, `lablet/telemetry/registry/`, or `lablet/tests/fixtures/outcome.json` without an `Unreleased` entry fails `cargo xtask changelog`; CI checks out full history for it. Third-party crates are pinned to exact versions in `[workspace.dependencies]` and bumped only in dedicated commits. MSRV is `rust-version` in the workspace manifest: the pinned toolchain minus two minor versions, raised only in a minor release. Windows is not supported.

On a fresh clone run `scripts/setup.sh` once: it installs the pinned Rust toolchain, trusts and installs the `mise.toml` tools, and installs the git hooks (`scripts/install-hooks.sh` does only the last step). Plain cargo commands run from `lablet/`.

## License

Lablet is dual licensed under MIT OR Apache-2.0. Every crate's `Cargo.toml` sets `license = "MIT OR Apache-2.0"`. Contributions are accepted under the same terms, as stated in the root README; no contributor agreement is needed. `cargo deny` checks that dependencies are compatible with both.

## Git and pull requests

- Work on a branch off `main`. When a logical piece is complete and `cargo xtask pre-push` passes locally, fast-forward `main` and push. No pull requests for now; this will be revisited as the process is learned.
- CI runs after the push. Check it; a red `main` is fixed forward before anything else lands.
- One logical change per commit. Moves and content edits in separate commits.
- A spec clarification (filling a gap, fixing an inconsistency, adding a missing test) goes in the same commit series with a note in the message.
- Architectural decisions may be made by whoever is building. Each gets an entry in `product/decisions.md` when it is made and is listed in the end-of-phase report for human review.
- Each build phase ends with a multi-agent code review of its diff, then a stop for human review. The next phase starts only on an explicit go-ahead.
