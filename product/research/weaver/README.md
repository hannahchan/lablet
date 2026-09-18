# OpenTelemetry Weaver for lablet: research and spike

Date: 2026-09-19. Weaver evaluated: **v0.26.1** (released 2026-09-03), installed as the GitHub release binary (`weaver-aarch64-apple-darwin.tar.xz`, sha256 verified) and, as a second route, `mise use "ubi:open-telemetry/weaver@v0.26.1"` (works; plain `mise use weaver@…` does not, there is no registry entry). `cargo install --git` was not needed. Working spike files are under [`spike/`](spike/).

## Sources

| Source | Version / ref | Used for |
| --- | --- | --- |
| [open-telemetry/weaver](https://github.com/open-telemetry/weaver) release notes and source | v0.26.1 | `weaver_common/src/vdir.rs`, `weaver_live_check/src/live_checker.rs`, `weaver_resolver/src/lib.rs` |
| Weaver docs: [define-your-own-telemetry-schema](https://github.com/open-telemetry/weaver/blob/main/docs/define-your-own-telemetry-schema.md), [semconv-syntax.v2](https://github.com/open-telemetry/weaver/blob/main/schemas/semconv-syntax.v2.md), [weaver-config](https://github.com/open-telemetry/weaver/blob/main/docs/weaver-config.md), [weaver_forge](https://github.com/open-telemetry/weaver/blob/main/crates/weaver_forge/README.md), [weaver_live_check](https://github.com/open-telemetry/weaver/blob/main/crates/weaver_live_check/README.md), [weaver_config](https://github.com/open-telemetry/weaver/blob/main/crates/weaver_config/README.md), [validate](https://github.com/open-telemetry/weaver/blob/main/docs/validate.md) | v0.26.1 | syntax, config, policies, live-check |
| [open-telemetry/semantic-conventions](https://github.com/open-telemetry/semantic-conventions) | v1.44.0 (`schema_url` `…/schemas/1.44.0`, pins `otel/weaver:v0.25.1`) | dependency, markdown templates, Makefile |
| [open-telemetry/semantic-conventions-genai](https://github.com/open-telemetry/semantic-conventions-genai) | commit `c88d504ab3d9879f8e50d3cc87e69775e11db234` (2026-09-16), `schema_url` `…/schemas/gen-ai-dev/1.42.0-dev`, depends on semconv v1.44.0, pins weaver v0.26.1 | dependency, example of a v2 dependent registry |
| [open-telemetry/opentelemetry-weaver-packages](https://github.com/open-telemetry/opentelemetry-weaver-packages) | commit `587265869c37180940d3bae29d23ca385e6baa00` (2026-09-02) | shared policies (`naming_conventions`, `stability`, `backwards-compatibility`, `entity_associations`) and the `templates/docs/markdown` package |
| [opentelemetry-rust `opentelemetry-semantic-conventions/scripts`](https://github.com/open-telemetry/opentelemetry-rust/tree/main/opentelemetry-semantic-conventions/scripts) | main (2026-09-17), pins weaver v0.24.1, spec 1.42.0 | Rust templates (v1 `groups` context) |
| Weaver issues [#1456](https://github.com/open-telemetry/weaver/issues/1456), [#1558](https://github.com/open-telemetry/weaver/issues/1558), [#994](https://github.com/open-telemetry/weaver/issues/994), [#1743](https://github.com/open-telemetry/weaver/issues/1743), [#1362](https://github.com/open-telemetry/weaver/issues/1362), [#1705](https://github.com/open-telemetry/weaver/issues/1705) | open, 2026-09-19 | Q9 |
| [otel-arrow PR #3830](https://github.com/open-telemetry/otel-arrow/pull/3830), [collector RFC #13454](https://github.com/open-telemetry/opentelemetry-collector/issues/13454), [OTel blog: Observability by Design](https://opentelemetry.io/blog/2025/otel-weaver/), [TelemetryDrops: Weaver from zero to hero](https://telemetrydrops.com/blog/weaver-from-zero-to-hero/), [Honeycomb on Weaver](https://www.honeycomb.io/blog/making-semantic-conventions-work-opentelemetry-weaver) | 2026 | Q9 |

## Findings

### 1. Versions and stability

| Item | State in v0.26.1 |
| --- | --- |
| Releases, last six months | v0.22.1 (2026-03-13), v0.23.0 (04-22), v0.24.x (06-19..23), v0.25.x (07-24, 07-29), v0.26.x (09-02, 09-03). Monthly-ish minors, each with a flagged breaking change. |
| v1 registry syntax (`groups:`) | Stable, still the bulk of core semconv v1.44.0 (which already mixes in v2 files). |
| `file_format: definition/2` | **Alpha** per `semconv-syntax.v2.md`; each v2 file loaded prints `⚠ File format definition/2 is not yet stable` (75 lines per run from the deps; `--quiet` does not hide them). It is what the GenAI SIG ships. #994 still lists refinement stats and forward-compatible versioning as open. |
| `registry check`, `generate`, `update-markdown`, `diff`, `emit`, `package`, `live-check` | Shipped, no experimental marker. `--v2` switches the template and policy input to the v2 materialized schema. |
| `registry resolve`, `registry search` | **Deprecated** (v0.22.1): "use `generate` or `package`". Still works; used in the spike. |
| `serve`, `registry infer`, `registry mcp`, `.weaver.toml` | Experimental (`serve` says so in `--help`; loading a `.weaver.toml` prints `⚠ Experimental! - Found config`). |
| Breaking changes since March 2026 | `version: "2"` → `file_format: definition/2`; `registry_manifest.yaml` deprecated for **`manifest.yaml`** (warning); `schema_url` mandatory on manifest dependencies and must end in semver; template auto-escaping off by default; `entity_associations` leaves became `{type, provenance}` objects; attribute-group refs became `{base, requirement_level}`; `stability`/`deprecated` disallowed on v2 attribute refs; live-check and infer bind `127.0.0.1` (v0.26.0); live-check finding attribute renames (v0.24.0). |

The spec currently says `registry_manifest.yaml` and `weaver registry resolve`; both should become `manifest.yaml` and `generate`/`package`.

### 2. Registry authoring: v1 vs v2, mixed dependencies

Use **v2**: the GenAI registry is v2-only, refinements and `imports` are v2 features, and the shared policy packages read the v2 materialized schema. Without `--v2` weaver converts v2 files to v1 groups internally, so v1-only tooling still works on a v2 registry (used below for live-check).

A v2 registry depending on a v1 registry (core v1.44.0) and a v2 registry (GenAI) in one manifest works. The manifest (`spike/registry/manifest.yaml`):

```yaml
name: lablet
schema_url: https://lablet.dev/schemas/0.1.0        # must end in a semver segment
stability: development
dependencies:
  - schema_url: https://opentelemetry.io/schemas/1.44.0            # must equal the dependency's own manifest schema_url
    registry_path: https://github.com/open-telemetry/semantic-conventions.git@v1.44.0[model]
  - schema_url: https://opentelemetry.io/schemas/gen-ai-dev/1.42.0-dev
    registry_path: https://github.com/open-telemetry/semantic-conventions-genai.git@c88d504ab3d9879f8e50d3cc87e69775e11db234[model]
```

GenAI's own dependency on semconv v1.44.0 deduplicates with lablet's (`dependency_graph` shows both edges). Commands and results:

| Command | Result |
| --- | --- |
| `weaver registry check --v2 -r registry` (git deps) | exit 0, 6.4 s, three clones into `~/.weaver/vdir_cache/repo*` (semconv twice: for lablet and for GenAI's dependency). First attempt: YAML `mapping values are not allowed` from an unquoted `: ` in a `brief`. |
| `weaver registry check --v2 -r registry-local -p policies` (vendored deps) | exit 0, 2.0 s. Warning: event without `requirement_level` "will be required in the future" (added). |
| `weaver registry resolve --v2 -r registry-local -f json -o out/resolved-v2.json` | exit 0, 10 MB JSON: `schema_url`, `registry{attributes,attribute_groups,metrics,spans,events,entities}`, `refinements{spans,events,metrics,entities}`, `dependencies` (full closure keyed by schema URL), `dependency_graph`. |

Two v2 details that shaped the recommendation:

- A `span_refinements` entry refining `gen_ai.invoke_agent.internal` resolves (26 inherited attributes inlined, each with `provenance.source`) but lands in `refinements.spans`, not `registry.spans`: the shared docs package gives it no page and the v1 conversion drops refinements (v0.26.0), so v1-mode live-check never sees its attributes. Declaring lablet's spans as **its own `spans:` entries** that `ref` the `gen_ai.*` keys (`spike/registry/lablet.yaml`) avoids both.
- `imports:` works for `spans` and `entities`, but importing the GenAI events fails: `event 'event.gen_ai.client.operation.exception' is marked as excluded from dependency resolution and cannot be used by 'imports'`. The excluded definition is core semconv's deprecated copy (`model/gen-ai/deprecated/`, `dependency_resolution: exclude: true`); the matcher stops there instead of falling through to the GenAI registry's live one. Reference the attributes on a lablet-owned event instead.

`template[int]` works as specified: `lablet.tool.calls` resolves with `type: template[int]`, docs render it, live-check recognises `lablet.tool.calls.bash` as a template instance and type-checks its value.

### 3. Dependency fetching and vendoring

From `crates/weaver_common/src/vdir.rs` and observation:

| Question | Answer |
| --- | --- |
| How is a git `registry_path` fetched? | `gix` clone into `tempfile::tempdir_in(~/.weaver/vdir_cache)`, prefix `repo`. Shallow (depth 1) only when no `@ref` is given; with a tag or SHA it is a full clone (gitoxide shallow+tag bug). A SHA is fetched then checked out directly. Git config and credential helpers are isolated unless `--allow-git-credentials`. |
| Is anything cached? | **No.** The directory is a `TempDir` deleted on drop; `~/.weaver/vdir_cache` is empty after every run. Every command re-clones. Offline runs with git paths fail. |
| Archive URLs? | Supported (`.zip`, `.tar.gz`, `[sub-folder]`, GitHub release assets via API, `[[auth]]` tokens). As the primary registry it works and is fast (0.46 s for the GenAI commit zip). As a manifest *dependency* with `[semantic-conventions-1.44.0/model]` it downloaded both zips but did not find their manifests and failed with an unresolved `extends`; not investigated further. |
| `[resolve.schema_url_overrides]` in `.weaver.toml`? | Redirects the content (the vendored manifests were loaded) **but still clones the git `registry_path` first** (`weaver_resolver/src/lib.rs` computes `loaded` before consulting the override); 8 s and four clones in the spike. Not an offline mechanism in 0.26.1. |
| Local path dependency? | Yes: `registry_path: deps/semantic-conventions/model`. Relative paths resolve against the **working directory**, not the manifest (`../deps/...` failed from the repo root, `deps/...` worked). Provenance then records `path:` instead of a git URL. |

Vendoring is therefore required for offline and deterministic CI. The two `model/` trees are 1.8 MB + 208 KB, 271 files (YAML plus eight GenAI JSON schemas). Neither project publishes a resolved-registry release asset yet (`weaver registry package` exists; GenAI has no releases).

### 4. Rust code generation

`weaver registry generate [--v2] -r <registry> -t <templates-root> <target> <out>` reads `<templates-root>/registry/<target>/weaver.yaml`. Filters are **jq** (`jaq`; not JMESPath as the plan says), templates MiniJinja. `weaver.yaml` holds `templates: [{template, filter, application_mode: single|each, file_name, when}]`, `params`, `comment_formats`, `whitespace_control`, `acronyms`, `text_maps`. Helpers: `semconv_grouped_attributes({"v2": true})`, `semconv_signal("span"; {"v2": true})`; Jinja `screaming_snake_case`, `pascal_case`, `snake_case`, `comment`, `concat_if`; tests `template_type`, `enum_type`, `deprecated`.

With `--v2` the template context is the materialized schema above. Two consequences: `.registry.attributes` holds only lablet-owned attributes, and attribute objects use `key` (v1 uses `name`). Imported `gen_ai.*`/`error.type` attributes are only reachable inlined on the signals that reference them, so the spike filter unions both:

```yaml
filter: >
  [ .registry.attributes[],
    ( (.registry.spans[]?, .registry.events[]?, .registry.metrics[]?,
       .refinements.spans[]?, .refinements.events[]?, .refinements.metrics[]?) | .attributes[]? ) ]
  | unique_by(.key) | sort_by(.key)
  | map(. + {root_namespace: (.key | split(".")[0])})
  | group_by(.root_namespace) | map({root_namespace: .[0].root_namespace, attributes: .})
```

Existing Rust templates: only opentelemetry-rust's `scripts/templates/registry/rust/` (v1 context, `attr.name`, constants behind a `semconv_experimental` cfg, plus a shell script that sed-patches rustdoc problems). Unmodified in v1 mode they generate three lablet constants and an empty `trace.rs`: a style reference, not a drop-in. The spike templates (`spike/templates/registry/rust/`) produce `lib.rs` (`SCHEMA_URL`), `attribute.rs` (one `pub const` per key, 8 in the final registry; template attributes also get `lablet_tool_calls(suffix) -> String`), `enums.rs` (one `#[non_exhaustive]` enum per enum attribute with `Unlisted(String)` and `as_str()`; a first-draft `Other` variant collided with `error.type`'s `_OTHER`), and `signals.rs` (`&[&str]` of declared keys per span and event, for tests). `cargo build`, `clippy -D warnings` and `cargo doc` are clean; regeneration is diff-free. Output is unformatted, so `cargo fmt` belongs in the xtask.

### 5. Policies

`weaver registry check -p <dir-or-file-or-git-url>` runs Rego (`import rego.v1`) in packages `before_resolution` (raw files) and `after_resolution` (resolved schema; with `--v2` the input is the materialized v2 schema, so rules read `input.registry.attributes[].key`). Findings are `{id, level: violation|improvement|information, message, context, signal_type?, signal_name?}`; any violation exits 1; `--diagnostic-format json|gh_workflow_command` for CI. Core semconv and the GenAI repo pull their policies from `opentelemetry-weaver-packages` by git URL and sub-folder: `naming_conventions` (namespace, format, collisions, metric brief), `stability` (deprecation blocks, signal never more stable than its attributes, `annotations.stability.policy_exceptions`), `backwards-compatibility` (needs `--baseline-registry`), `entity_associations`. Policies are only loaded from `-p`/`.weaver.toml`; nothing in the registry directory is picked up implicitly.

`spike/policies/justification.rego` requires every `lablet.*` attribute's `note` to start with `Justification:` and to declare `development` stability. Positive run passed; stripping one note produced `Policy violation: id=lablet_attribute_missing_justification, context={"attribute":"lablet.run.turns"}` and exit 1.

### 6. Documentation generation

Core semconv keeps `templates/registry/markdown/` in-repo, but the canonical set now lives in `opentelemetry-weaver-packages/templates/docs/markdown` ("requires weaver 0.25.0", v2 only) and the GenAI repo uses local v2 templates with the same shape. Both work on a custom registry without copying: `weaver registry generate --v2 -r registry -t '<packages-git-url>[templates/docs]' --param registry_base_url=/docs/telemetry markdown docs` produced `README.md`, `lablet/{README,spans,events}.md`, `service/entities.md`, `telemetry/entities.md` (staged under `spike/docs/`). `weaver registry update-markdown … --target markdown docs-handwritten` fills `<!-- weaver <jq> --><!-- endweaver -->` markers in a hand-written page with the same tables (`spike/docs-handwritten/telemetry.md`); the filter must select one object (`.registry.spans[] | select(.type == "lablet.invoke_agent")`). Legacy `<!-- semconv -->` markers make the command fail with no detail. Refined spans get no page from the package; a snippet over `.refinements.spans[]` does render.

### 7. live-check

| Aspect | Fact (v0.26.1) |
| --- | --- |
| Listener | OTLP **gRPC only**, `--otlp-grpc-port` 4317, bound to `127.0.0.1` by default (`--otlp-grpc-address 0.0.0.0` to change). No OTLP/HTTP, so `curl` cannot feed it; `weaver registry emit` or an SDK exporter can. |
| Admin | HTTP on `--admin-port` 4320, same bind address: `GET /health` → `{"status":"ready"}`, `POST /stop`. With `--output http` the `/stop` response body is the report (used by the composite actions). |
| Stop conditions | `/stop`, SIGINT, SIGHUP, `--inactivity-timeout <s>` (0 = never). |
| Output | `--format` json, jsonl, yaml or ansi (ansi streams per sample); `--output` a directory, `none` or `http`; `--emit-otlp-logs` as `weaver.live_check.finding` log records. Report = `{samples: [...augmented with live_check_result], statistics: {advice_level_counts, advice_type_counts, registry_coverage, seen_*}}`. |
| Exit code | 1 if any finding at or above `--fail-on` (default `violation`; `improvement`, `information`, `none`). |
| Config | `.weaver.toml` `[live_check]` plus `[[live-check.finding_filters]]` (drop by `exclude` ids, `min_level`, `exclude_samples`/`sample_names` globs, `signal_type`) and `[[live-check.finding_level_overrides]]`. Both worked in the spike (`not_stable` dropped, `undefined_enum_variant` promoted to violation). |
| CI | Composite actions in the weaver repo: `setup-weaver`, `weaver-live-check-start` (waits on `/health`, records PID), `weaver-live-check-stop` (posts `/stop`, parses, `fail-on`). No network is needed at run time when the registry and deps are local paths. |

What it validates, per `live_checker.rs` and the spike: attribute existence (`missing_attribute`, violation), type (`type_mismatch`, violation; `int` accepted for `double`), enum values (`undefined_enum_variant`, **information** by default), template instances (`template_attribute`, information, matched by `starts_with` without a dot check, issue #1743), deprecation, stability (`not_stable`, improvement), naming (`missing_namespace`, `invalid_format`, `illegal_namespace`, `extends_namespace`), and for **events and metrics matched by name**: `missing_event`/`missing_metric`, `required_attribute_not_present`, `recommended_attribute_not_present`, unit and instrument. Entities via `entity_associations`. What it cannot validate: **spans are never matched to a span definition** (there is no `find_span`; `SampleSpan` gets attribute-level advice only), so span names, kinds and required span attributes are unchecked; message-content `any` attributes are not schema-checked (#1681); custom `--advice-policies` replace the built-in Rego rather than extend it (#1362).

The important gotcha is which attributes the checker indexes. With `--v2` the index is built from `registry.registry.attributes` only, i.e. lablet-owned keys; every `gen_ai.*`, `error.type`, `service.name` and `telemetry.sdk.*` sample was a `missing_attribute` violation (26 of them), even when referenced on lablet's own event. Issue #1456 describes exactly this; its workaround `--include-unreferenced` fails here with `Ambiguous reference 'gen_ai.agent.id' found in multiple dependencies` (core's deprecated copy vs the GenAI registry). Without `--v2`, the index is built from every v1 group's inlined attributes, so anything lablet references on its own spans/events, plus attributes of imported entities, is known. Session J below is the working configuration.

Spike sessions (all: `weaver registry live-check -r <registry> --format json --output http --otlp-grpc-port 4317 --admin-port 4320 --inactivity-timeout 60 &`, wait on `/health`, send, `curl -X POST :4320/stop`):

| Session | Registry / flags | Source | Result |
| --- | --- | --- | --- |
| A | `--v2` | `weaver registry emit --v2 -r registry-local --skip-policies` | exit 1: 11 `missing_attribute` (resource `service.name`, `telemetry.sdk.*`, `gen_ai.agent.name`, `gen_ai.usage.input_tokens`), template recognised, `lablet.run` seen |
| B | `--v2` | Rust client (`spike/otlp-client`, `opentelemetry-otlp` 0.31 tonic; needs `#[tokio::main]`) | exit 1: all deliberate faults found (`type_mismatch` ×3, `undefined_enum_variant`, `lablet.undeclared`, `required_attribute_not_present` ×3, `missing_event`) plus 26 false `missing_attribute` |
| D/E | `--v2 --include-unreferenced` | — | failed to start: ambiguous `gen_ai.agent.id` |
| **J** | no `--v2`, `spike/registry` (own spans + `imports: entities: [service, telemetry.sdk]`), `spike/.weaver.toml` | client | exit 1 with **only** the deliberate faults; the conforming span and log record produced zero violations ([`spike/live-check-findings.txt`](spike/live-check-findings.txt)) |
| K | same registry, `--v2` | client | 26 false `missing_attribute` again; G/H (v1 mode, refinement-based registry) sat in between: event attributes known, refinement-only and resource attributes not |

### 8. Other signals

Log records: yes. A log record with `event_name` is matched to a v2 `events:` entry by name; the checker validates presence of required/recommended attributes and each attribute's type and enum value, reports `missing_event` for unknown names, and counts coverage (`seen_registry_events`). Records without an event name are attribute-checked only. So lablet's wide event should be declared as an event named `lablet.run` and emitted with that `event_name`. Metrics: matched by name; unit, instrument, data-point attributes and required attributes checked (`unit_mismatch`, `unexpected_instrument`). Span events and links: attribute-level only. Profiles: added in v0.26.0.

### 9. Who else uses it

| Project | Use | Learned |
| --- | --- | --- |
| semantic-conventions (core) | check with weaver-packages policies, `update-markdown`, Docker-pinned `otel/weaver:v0.25.1` | Policies fetched by git URL each run; a temp `~/.weaver` is mounted. |
| semantic-conventions-genai | v2 dependent registry, `--v2` everywhere, `versions.env` pins weaver and the policy SHA, local v2 markdown templates, `package-dev` | Their `weaver.yaml`: `.registry.attributes` "contains only locally-owned attributes (verified empirically)"; a TODO tracks note overrides lost through `ref_group` (weaver #1407). |
| opentelemetry-rust | constants crate from core v1.42.0 with weaver v0.24.1 in Docker, v1 templates, sed post-processing for rustdoc | Doc comments need care (`<key>`, `[0,n]`, bare URLs, unlabelled code fences); `cargo fmt` after generation. |
| otel-arrow (PR #3830, open) | 388 metrics / 443 events as a v2 registry, `weaver registry check --v2` in CI, Rust codegen deferred | Reviewer asked for the check to live in `xtask`; the PR went stale. |
| opentelemetry-collector RFC #13454 | proposal to base mdatagen on weaver | open, no owner. |
| opentelemetry-go, semantic-conventions-java, promconv, MrAlias/semconv-go | generated constants and type-safe APIs (OTel blog) | Go/Java are the mature codegen targets; Rust has no shared template package. |
| TelemetryDrops guide (weaver 0.22.1, Go) | custom v2 registry, no upstream dependency | Jinja blank-line noise needs a formatter; unquoted `:` in jq filters inside YAML breaks parsing; v1 context uses `attr.name`. |

Open issues that touch lablet: #1456 (live-check cannot see dependency attributes in v2 mode), #1558 (explicit `imports` vs `--include-unreferenced` inconsistency), #1743 (template prefix matching without a dot), #1705 (annotations on a `ref` replace instead of merge), #1362 (custom live-check policies replace built-ins), #1681 (`any` JSON schema `$ref`), #994 (v2 completeness and forward-compatible versioning).

### 10. Risks and fallback (summary; detail below)

Weaver is usable for lablet today, with three sharp edges: the v2 format is Alpha and each minor release has broken something; no dependency cache, so vendoring is mandatory; and live-check must run in v1 mode with lablet-owned spans until #1456 is fixed. The fallback is small because `weaver registry resolve --v2 -f json` (10 MB, includes the full dependency closure) is a plain document a `build.rs`/xtask can consume with `serde_json`.

## Spike log

Working directory: the scratch `weaver-spike/`; `bin/weaver` = v0.26.1 release binary.

1. `curl` the release tarball and `.sha256`, verify, extract: `weaver 0.26.1`. `mise use "ubi:open-telemetry/weaver@v0.26.1"` also works; `mise use weaver@0.26.1` errors.
2. Shallow-clone weaver v0.26.1, semconv v1.44.0, semconv-genai (`c88d504a`), opentelemetry-rust, weaver-packages as reference trees.
3. Write `registry/manifest.yaml` (git deps) and `registry/lablet.yaml` (3 attributes incl. enum and `template[int]`, a `span_refinements` of `gen_ai.invoke_agent.internal`, one event). `check --v2` → YAML error; fix → exit 0 in 6.4 s.
4. Vendor both `model/` trees under `deps/`; manifest with relative paths: `../deps/...` → `IO error … No such file` from the repo root, `deps/...` → exit 0 in 2.0 s. `resolve --v2 -f json` → 10 MB.
5. `.weaver.toml [resolve.schema_url_overrides]` with `--config` and by auto-discovery: overrides applied but git clones still happened (8 s).
6. `policies/justification.rego`; `check --v2 -p policies` passes; copy with a note removed fails with the expected finding id.
7. Write the Rust templates; `generate --v2 … rust generated/lablet-telemetry-registry`. Two build fixes (enum variant collision with `_OTHER`; duplicate `EVENT_LABLET_RUN_ATTRIBUTES` because `lablet.run` appears in both `registry.events` and `refinements.events`, deduplicated with `unique_by`). Then build, clippy, doc clean; regeneration diff-free.
8. Docs from weaver-packages `templates/docs`; `update-markdown` failed while a legacy `<!-- semconv -->` marker was present, then worked for event and refinement snippets.
9. Without `--v2`: `resolve` yields v1 groups `event.lablet.run` and `registry.registry-local.lablet`; opentelemetry-rust templates give 3 constants and an empty `trace.rs`.
10. Archive dependencies: as manifest deps → unresolved `extends`; as the primary registry → 0.46 s. Git-deps check re-timed at 6.8 s.
11. live-check sessions A, B (client needed `#[tokio::main]`), D/E (fail), G/H, then `registry-own` (own spans, entity imports): policies pass; sessions J and K. Regenerated crate and docs from `registry-own`; staged under `spike/`.

## Recommended approach for lablet

- **Syntax**: v2 (`file_format: definition/2`) in `lablet/telemetry/registry/`, manifest named `manifest.yaml`. Declare `attributes:` (each `lablet.*` with `note: "Justification: …"`), lablet-owned `spans:` (`lablet.invoke_agent`, `lablet.chat`, `lablet.execute_tool`, each with `name.note` and `ref`s to the `gen_ai.*`/`mcp.*`/`error.type` keys the spec lists, `requirement_level` set), `events:` (`lablet.run` wide event, `lablet.*` for anything else; do not try to import or refine the GenAI events) and `imports: entities: [service, telemetry.sdk]` for the resource. Use `span_refinements` only for documentation cross-links, not as the primary definition, until v1 conversion keeps refinements.
- **Pins**: weaver v0.26.1 in `mise.toml` (`"ubi:open-telemetry/weaver" = "v0.26.1"`) or the release tarball with sha256 in the xtask; semconv v1.44.0; semconv-genai `c88d504ab3d9879f8e50d3cc87e69775e11db234`; weaver-packages `587265869c37180940d3bae29d23ca385e6baa00`.
- **Vendoring**: copy the two `model/` trees to `lablet/telemetry/deps/semantic-conventions/model` and `…/deps/semantic-conventions-genai/model` with a `SOURCES` file recording repo, ref and date; `manifest.yaml` uses those relative paths, and the xtask always runs weaver from the repo root. Vendor the weaver-packages policy and template folders the same way (they are fetched by git on every run otherwise). Nothing in CI then needs the network.
- **Templates**: start from `spike/templates/registry/rust/` (v2 context; constants, enums, signal key lists) and keep opentelemetry-rust's `comment_formats` and its post-processing list as a checklist for rustdoc breakage as the attribute set grows. Add `cargo fmt` to the generate step.
- **Docs**: vendored `templates/docs/markdown` from weaver-packages for `lablet/docs/telemetry/` plus `update-markdown` markers in hand-written pages.
- **xtask commands**: `cargo xtask weaver check` = `weaver registry check --v2 -r lablet/telemetry/registry -p lablet/telemetry/policies -p lablet/telemetry/deps/weaver-packages/policies/check/naming_conventions -p …/stability --diagnostic-format gh_workflow_command` (in CI); `cargo xtask weaver generate` = `generate --v2 … rust crates/lablet-telemetry-registry` + `cargo fmt` + docs `generate`/`update-markdown`, with `--check` comparing against the tree; `cargo xtask weaver live-check` = start `live-check -r lablet/telemetry/registry --config lablet/telemetry/.weaver.toml --format json --output http` **without `--v2`**, poll `/health`, run the fake-provider config with `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317` and `OTEL_EXPORTER_OTLP_PROTOCOL=grpc`, `POST /stop`, save the report, fail on exit 1. Use a random free port pair rather than 4317 (another process on this machine already listens on `*:4317`).
- **`.weaver.toml`** (`spike/.weaver.toml`): exclude `not_stable`, promote `undefined_enum_variant` to violation; keep `fail_on = "violation"`. Do not set `[registry] v2 = true` there, because it would apply to live-check.
- **CI**: check + generate-diff gate on every PR; live-check job on Linux using the vendored registry; a weekly job that runs `check` against the git URLs at the pinned refs to catch vendoring drift, and `weaver registry diff --baseline-registry` when bumping the GenAI SHA to list renamed `gen_ai.*` attributes.

## Risks and fallback

| Risk | Likelihood / impact | Mitigation |
| --- | --- | --- |
| v2 format or materialized-schema shape changes in a weaver minor (it did in 0.24, 0.25, 0.26) | High / medium: templates, policies and the jq filters break on upgrade | Pin weaver; treat upgrades as a phase task; the regenerate-diff gate catches silent output changes. |
| live-check v2 index ignores dependency attributes (#1456); `--include-unreferenced` deprecated and broken for our pair of deps | Certain today / high for the CI gate | Run live-check in v1 mode with lablet-owned spans and entity imports (verified). Revisit when #1456 closes; a `finding_filters` entry with `sample_names: ["gen_ai.*", "service.*", "telemetry.sdk.*"]` is the crude fallback. |
| Spans are not matched to definitions | Certain / medium: span names and required span attributes are unchecked | Unit tests against `signals.rs` key lists and span names in `telemetry-otel`; treat live-check as attribute-level. |
| No dependency cache; git clone per run; archive deps unreliable | Certain / low once vendored | Vendor; never use git paths in the committed manifest. |
| GenAI conventions rename attributes at a SHA bump | Medium / medium | `weaver registry diff` plus the generated-crate diff makes every rename a visible PR change. |
| Template attribute prefix matching without a dot (#1743) | Low / low | Keep `lablet.tool.*` template names unique prefixes. |
| Weaver abandoned or unusable | Low / medium | Fallback below. |

Fallback: keep the registry YAML and policies (the contract is the asset), replace `weaver registry generate` with a ~200-line `xtask` step that reads `weaver registry resolve --v2 -f json` (or the committed `resolved.json` from `weaver registry package`) with `serde_json` and emits the same four Rust files; replace live-check with an in-process test observer that checks emitted keys against `signals.rs`. If weaver disappears entirely, the resolved JSON is checked in, so codegen keeps working; only re-resolution needs a replacement parser.

## Open questions

1. Will #1456 be fixed so `--v2` live-check indexes referenced dependency attributes? Until then the registry shape (own spans, not refinements) is chosen for the checker, not the conventions.
2. Should the wide event be a lablet-owned event that `ref`s `gen_ai.*` keys (works) or should lablet wait for the GenAI registry to stop excluding its events from dependents (blocks `imports`/refinements today)?
3. Archive-URL dependencies with sub-folders failed in the manifest; worth a minimal repro upstream if anyone wants zip-based vendoring instead of copied trees.
4. `weaver registry resolve` is deprecated; confirm `weaver registry package` output (`resolved.yaml` + publication manifest) is the stable artefact the fallback should read.
5. How to silence the 75-line `not yet stable` warning noise (`--quiet` keeps it); `--diagnostic-format json` filtered in the xtask is the current answer.
6. Whether to run the shared `stability` policy with `--baseline-registry` from the previous tag once lablet has released a registry version.
