# Contributing

How to work in this repository. The what and why live in [../product/](../product/).

## Layout

| Area            | Question                       | Contents                                 |
| --------------- | ------------------------------ | ---------------------------------------- |
| `product/`      | What are we building, and why? | Brief, spec, build plan, decisions log   |
| `contributing/` | How do we work?                | This document                            |
| `lablet/`       | The output                     | Rust workspace and user-facing docs      |
| `xtask/`        | Gates                          | Root-level crate, not a workspace member |

## Architecture rules

Explicit architecture. Inside `lablet/`, directory `foo/bar/` is package `lablet-bar`, except `apps/lablet` (`lablet`), `tests/conformance` (`lablet-conformance`), and `tests/mcp-server` (`lablet-test-mcp-server`).

| Ring                  | Path                                 | May depend on                                                       | Forbidden crates                                                 |
| --------------------- | ------------------------------------ | ------------------------------------------------------------------- | ---------------------------------------------------------------- |
| Domain                | `crates/domain/*`                    | domain                                                              | tokio, reqwest, tracing, opentelemetry, rmcp, tonic, axum, hyper |
| Application           | `crates/application/*`               | domain                                                              | tokio, reqwest, opentelemetry, rmcp, tonic, axum, hyper          |
| Secondary adapters    | `crates/adapters/secondary/*`        | application, domain, adapter shared kernel; never a sibling adapter | none                                                             |
| Adapter shared kernel | `crates/adapters/secondary/shared/*` | application, domain, other adapter shared kernels; never an adapter | none; may be used by sibling adapters, implements no port        |
| Composition root      | `apps/*`                             | everything except test support                                      | none                                                             |
| Test support          | `tests/*`                            | anything, dev-only                                                  | none                                                             |

`cargo xtask lint-layers` enforces this on `[dependencies]` and `[build-dependencies]` (dev-dependencies are exempt) and runs at pre-commit: a workspace crate is placed by the path of its `[workspace.dependencies]` entry, an external crate is matched by name. A forbidden name covers its family (`opentelemetry` also forbids `opentelemetry_sdk`), and no crate outside `tests/` may depend on a `tests/` crate. `serde` and `serde_json` are allowed everywhere; the domain model is the one serde form of the conversation.

- Ports are object-safe `async_trait` traits, `Send + Sync`, defined in the application crate that consumes them, injected as `Arc<dyn Trait>` by constructor. No DI framework, no generics over adapters.
- Adapters implement ports and map their own errors into the port's error at the `impl` boundary. Adapter types never appear in application or domain signatures.
- The composition root selects adapters from config in `build()`. Adding an adapter is a new crate plus a match arm.
- No primary adapters: the binary and library entry points call the use case directly.
- Domain crates contain types and pure functions only. Anything async is application or outward.

## Code conventions

- `thiserror` for errors. No `anyhow` (banned in `lablet/deny.toml`, so `cargo xtask deny` refuses it). One error enum per port or boundary, owned by the layer that defines it. Variants carry `String`, not foreign error types.
- Workspace lints. Enforced by `cargo xtask clippy` with warnings denied (pre-commit): `unsafe_code = "forbid"`, `missing_docs`, `rust_2018_idioms`, clippy `all` and `pedantic`, and `unwrap_used`, `expect_used`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr`, `allow_attributes`, `allow_attributes_without_reason`. Enforced by `cargo xtask doc` with warnings denied (pre-push and CI), because clippy never runs rustdoc: rustdoc `broken_intra_doc_links`, `private_intra_doc_links`, `redundant_explicit_links` at deny. Nursery isn't enabled and pedantic has no relaxations, so every public function returning `Result` needs an `# Errors` section and every one that can panic a `# Panics` section. Suppress a lint with `#[expect(..., reason = "...")]`; `#[allow]` is itself a lint failure.
- Tests may `unwrap`: the root `clippy.toml` allows it inside `#[test]` functions and `#[cfg(test)]` modules, for both the workspace and `xtask`. In an integration target, declare every module in `tests/it/main.rs` as `#[cfg(test)] mod name;` so shared helpers are covered too.
- The outcome print in `main.rs` is the one `print_stdout`, marked with `#[expect]`.
- `clap` derive API in the binary.
- Every dependency is declared once, in `[workspace.dependencies]`, with a comment on the line directly above it saying why it's there. The gate checks that the line above is a comment: a comment block counts, a trailing comment or a comment above a blank line doesn't. It can't tell a group header from a reason, so a header is followed by a blank line and never stands in for an entry's own comment. A third-party entry is an exact `=x.y.z` pin from crates.io; an internal entry is `{ path, version }` with the path exactly as `[workspace] members` lists it. A member's manifest only inherits: `name.workspace = true`, or `{ workspace = true }` with `features`, `optional`, or `default-features` beside it, in every dependency table, and no version, path, git, registry, or package key of its own. So it needs no comments; the reason lives in the workspace table. `xtask/Cargo.toml` follows the same pin and comment rules. `cargo xtask lint-manifests` checks all of this. Grouping under `# Domain`, `# Application`, `# Adapters`, `# External` headers, the blank line after a header, and what a comment says are review conventions, not gates.
- The lints model only the Cargo features lablet uses. Member globs, `[workspace] exclude` or `default-members`, a `[package]` in the workspace root, `[patch]` or `[replace]`, and `patch`, `paths`, or `source` in a `.cargo/config.toml` fail the gates as unsupported. Using one means extending `xtask` deliberately, not working around the message.
- A comment says why something non-obvious is the way it's written, or states a contract a caller relies on. Nothing else. Don't narrate how the code came to be, don't record facts that drift (versions, dates, counts, build phases), and don't restate the code or these docs. Doc comments stay on public items because `missing_docs` is enforced; one sentence where one will do. This is a review convention.
- Test names are sentences: `a_tool_error_does_not_consume_the_retry_budget`. This is a review convention, not a gate.
- In the crates with a coverage floor (`lablet-model`, `lablet-policy`, `lablet-run`), unit tests live in a sibling file declared as `#[cfg(test)] mod tests;`: `src/foo.rs` and `src/foo/tests.rs`, or `src/tests.rs`, with helper modules below that file. Never an inline `mod tests { ... }`, and no other attribute that mentions `test` in a production file: no bare `#[test]` or `#[tokio::test]` function, no `#[cfg(test)]` on another item or on a `#[path]` module, no `#[cfg(all(test, ...))]`, `#[cfg_attr(test, ...)]`, or `#[cfg(not(test))]`. Coverage tells test code from production code by file name (`tests.rs` files and `tests/` directories are left out), so test code anywhere else would count as covered production lines; `cargo xtask coverage`, `cargo xtask mutants`, and an `xtask` test (pre-push) fail on such an attribute. The check reads lines that start with `#[` or `#![`; an attribute rustfmt has wrapped over several lines is left to review. Other crates may keep unit tests inline.
- One integration test target per crate: `tests/it/main.rs` declaring modules. This is a review convention, not a gate.
- Test doubles are hand-written fakes in the consuming crate. No mocking framework (the known ones are banned in `lablet/deny.toml`).

## Telemetry is contract-first

Every span, event, and attribute is declared in the Weaver registry under `lablet/telemetry/registry/` before it's emitted: `attributes.yaml` holds lablet's own attributes, `spans.yaml` and `events.yaml` the signals and the semantic-convention attributes they refer to, and `resource.yaml` the imported resource entities. Three rules hold, the first by review and the others by `weaver check`:

- A core or GenAI semantic-convention attribute is used wherever one exists. A `lablet.*` attribute is the last resort.
- Every `lablet.*` attribute is `development` and has a `note` that opens with `Justification:` and says why no convention covers it.
- Every attribute on a span or event states its requirement level, with the condition when it's conditional. Captured content is `opt_in`.

The sources of the `telemetry-registry` crate (attribute name constants, enums for lablet's closed value sets, and the required and complete key lists of each span and event) and the reference under `lablet/docs/telemetry/` are generated from the registry and checked in. Everything in those two directories is generated: don't edit it, and don't write an attribute name as a string literal in an adapter. To add an attribute, edit the registry, run `cargo xtask weaver generate`, and use the constant. The Rust templates are in `lablet/telemetry/templates/registry/rust/`; the pages come from the vendored upstream templates.

The lablet policy holds four rules: a `lablet.*` attribute has a note that begins `Justification:`, it's `development` until 1.0, nothing is defined here outside `lablet.*`, and every attribute on a span or event states `required`, `conditionally_required`, or `opt_in`. An omitted level resolves to `recommended` and would drop the key from the generated required list, so lablet doesn't use that level. Constants carry the conventions' own briefs as doc comments. When one trips clippy's `doc_markdown`, add the identifier to `doc-valid-idents` in `clippy.toml`.

```bash
cargo xtask weaver check               # the registry against the lablet, naming, and stability policies
cargo xtask weaver generate            # write the crate sources and the reference again
cargo xtask weaver generate --check    # fail when they differ from what the registry renders to
```

Both forms of `generate` render into `lablet/target/` first, so a failed run leaves the tree alone, and `--check`, which pre-commit runs, never writes outside it.

`weaver check` reads only the working tree. Weaver clones a git dependency on every run and keeps no cache, so everything it reads from upstream is vendored under `lablet/telemetry/deps/`: the `model/` trees of the core and GenAI semantic conventions, and the naming policies, stability policies, and Markdown templates from `opentelemetry-weaver-packages`. `lablet/telemetry/vendor.sh` holds the pinned commits and is the only thing that writes that directory. Each tree has a `SOURCES` file naming its repository, commit, and copied paths. The files are byte-identical to their source with one exception, which `SOURCES` records: the GenAI manifest names the core conventions by git URL, and the script points that line at the vendored copy, because Weaver would otherwise clone it on every run.

```bash
cargo xtask weaver vendor            # after changing a pin in lablet/telemetry/vendor.sh
cargo xtask weaver vendor --check    # fetch the pins again and fail on any difference
```

Both need the network, so neither is part of a gate. Changing a pin is its own commit.

Weaver's diagnostics go through lablet's templates in `lablet/telemetry/templates/diagnostics/`: `text` on a terminal and `github`, which writes workflow commands, when `CI=true`. They drop the warning Weaver prints for every file in the v2 format and the warnings about vendored files, and nothing more severe than a warning.

## Gates

Every rule on this page is enforced by a gate. If it's not enforced, it's a suggestion, not a rule.

```bash
cargo xtask pre-commit    # fmt (rustfmt and dprint), clippy, lint-layers, lint-manifests, weaver check, weaver generate --check, lint-shell, lint-prose
```

```bash
cargo xtask pre-push      # pre-commit plus cargo deny, changelog, rustdoc without warnings, tests
```

Run `cargo xtask help` for every task, grouped in the order a developer works: development, telemetry contract, quality checks, quality gates, analysis, project. A green gate prints one closing line; the step table appears only when something failed, and it names the task that runs each failed step alone.

CI runs on every pushed branch as a matrix of `cargo xtask ci` (the pre-push list), `cargo xtask coverage` (floor: 90% lines on `lablet-model`, `lablet-policy`, `lablet-run`), and `cargo xtask mutants` (floor: 80% caught on the same crates). A floor crate may measure nothing only while its `src/` defines no function; after that, a run with no lines or no mutants for it fails. A trait method without a body counts as a function, so the first function with a body lands with its test in the same push as the first port. Later phases add `cargo xtask weaver live-check` (phase 6), `cargo xtask bench` with a 20% regression threshold, and the release builds (phase 11). `xtask` is a root-level crate reached through the alias, which is defined twice, in `.cargo/config.toml` and `lablet/.cargo/config.toml`; an xtask test keeps the two in step. It doesn't work from a crate directory below `lablet/`.

## Shell scripts

`cargo xtask lint-shell` runs shellcheck over every tracked shell script: `.sh` files and the git hooks, which have no extension and are found by their shebang. `product/research/` is left out, as Vale and dprint leave it out, because it's a frozen record, and so is the vendored `lablet/telemetry/deps/`.

## Editor and agent setup

`.vscode/` recommends rust-analyzer, dprint, Vale, and Claude Code, and points rust-analyzer at both Cargo workspaces. `.claude/settings.json` runs `.claude/hooks/session-start.sh` when a Claude Code session starts. It says when the session is on `main`, when a branch is behind `origin/main` and so can't be fast-forwarded to, when the branch tracks an unexpected upstream, and when the git hooks aren't installed. It only advises; it never stops a session.

## Formatting

`cargo xtask fmt` formats Rust with rustfmt and JSON, TOML, Markdown, and YAML with dprint, configured in `dprint.json`; `fmt --check` verifies both and runs in pre-commit. The vendored and generated trees are excluded there, because they must stay byte-identical to their source. dprint downloads its pinned plugins on first use, so the first run needs the network.

## Prose

`cargo xtask lint-prose` runs Vale with the Microsoft writing style package over `README.md`, `CLAUDE.md`, `CHANGELOG.md`, `contributing/`, `product/` (not `product/research/`), `lablet/README.md`, and `lablet/docs/` (not the generated `lablet/docs/telemetry/`). The gate fails on errors only; `cargo xtask lint-prose --all` also shows warnings and suggestions. `.vale.ini` names the package by its release URL, so `vale sync` fetches a fixed version into `.vale/styles/`, which git ignores apart from our vocabulary. `scripts/setup.sh` runs the sync, and `lint-prose` runs it when the styles are missing, so only the first run needs the network. To move to a newer package, change the URL and run `cargo xtask setup`. Vale's own notes for agents are at <https://vale.sh/AGENTS.md>. A legitimate technical term that Vale flags as a misspelling goes in `.vale/styles/config/vocabularies/Lablet/accept.txt`, one per line, sorted. Fix an ordinary misspelling in the text. If a rule makes the docs worse, turn that one rule down in `.vale.ini` with a one-line reason.

## Reviews

A phase-end review is scaled to risk and has a budget.

- Scaffolding, configuration, and generated code get the gates, the builder's own read of the diff, and one reviewer. Logic-heavy code (the loop, policy, exporters, adapters) gets two or three focused reviewers.
- Each reviewer reports at most five findings, high and medium severity only.
- The builder sorts the findings before anything is verified or fixed. A finding that isn't worth handling in a lightweight project is dropped, or the fix is to reject the input rather than to model it.
- One verifier for each surviving finding, reasoning from the code first. Reproduce only when the claim is disputed or cheap to run.
- Each phase states a review budget of about 10 to 15 percent of the build's token cost, and the phase report gives the actual figure.
- Every gate or feature is exercised once with real input on a cold clone before the phase closes.

## Versioning

One workspace version. Keep-a-changelog format in `CHANGELOG.md`. A change to `lablet/schema.json`, `lablet/telemetry/registry/`, or `lablet/tests/fixtures/outcome.json` without an `Unreleased` entry fails `cargo xtask changelog`; CI checks out full history for it. Third-party crates are pinned to exact versions in `[workspace.dependencies]` and bumped only in dedicated commits; a crate `xtask` pins too carries the same version in both places. MSRV is `rust-version` in the workspace manifest: the pinned toolchain minus two minor versions, raised only in a minor release. Windows isn't supported.

On a fresh clone run `scripts/setup.sh` once: it installs the pinned Rust toolchain, trusts and installs the `mise.toml` tools, and installs the git hooks (`scripts/install-hooks.sh` does only the last step). Plain cargo commands run from `lablet/`.

## License

Lablet is dual licensed under MIT OR Apache-2.0. Every crate's `Cargo.toml` sets `license = "MIT OR Apache-2.0"`. Contributions are accepted under the same terms, as stated in the root README; no contributor agreement is needed. `cargo deny` checks that dependencies are compatible with both.

## Git and pull requests

- Work on a branch off `main`. When a logical piece is complete and `cargo xtask pre-push` passes locally, fast-forward `main` and push. No pull requests for now; this will be revisited as the process is learned.
- CI runs after the push. Check it; a red `main` is fixed forward before anything else lands.
- One logical change per commit. Moves and content edits in separate commits.
- A spec clarification (filling a gap, fixing an inconsistency, adding a missing test) goes in the same commit series with a note in the message.
- Architectural decisions may be made by whoever is building. Each gets an entry in `product/decisions.md` when it's made and is listed in the end-of-phase report for human review.
- Each build phase ends with a review of its diff, then a stop for human review. The next phase starts only on an explicit go-ahead.
