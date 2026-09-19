//! Layer rules over the workspace (spec §2, contributing "Architecture
//! rules"). Each crate's ring is read from its path, and its `[dependencies]`
//! and `[build-dependencies]` are checked against what that ring may reach:
//! which workspace crates, and which external crate families.
//! `[dev-dependencies]` are exempt from both. One sweep reports every finding.
//!
//! `lint-manifests` makes every dependency of a member an inherited entry of
//! `[workspace.dependencies]`, so that table is the one place a crate's real
//! name and path are read from. An entry this lint cannot judge (one a member
//! declares for itself, one the table lacks) is a finding here too, never a
//! pass.

use std::fmt;

use crate::workspace::{Member, Workspace, inherits_workspace, normalise};

/// A ring of the architecture. A crate's ring is decided by its directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ring {
    /// `crates/domain/*`: types and pure functions.
    Domain,
    /// `crates/application/*`: use cases and the ports they consume.
    Application,
    /// `crates/adapters/secondary/*`: driven adapters and their shared kernels.
    SecondaryAdapter,
    /// `apps/*`: the composition root.
    CompositionRoot,
    /// `tests/*`: conformance suites and test servers, reached only through
    /// `[dev-dependencies]`.
    TestSupport,
}

impl fmt::Display for Ring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Domain => "Domain",
            Self::Application => "Application",
            Self::SecondaryAdapter => "Secondary Adapter",
            Self::CompositionRoot => "Composition Root",
            Self::TestSupport => "Test Support",
        })
    }
}

/// The rings, by the directory that places a crate in them. The prefixes are
/// disjoint, so a member falls in at most one ring; one that falls in none is
/// a finding. There is no primary adapter ring: spec §2 has no primary
/// adapters, so a crate under `crates/adapters/primary/` is in no ring until
/// the spec gains one.
const RINGS: &[(&str, Ring)] = &[
    ("crates/domain/", Ring::Domain),
    ("crates/application/", Ring::Application),
    ("crates/adapters/secondary/", Ring::SecondaryAdapter),
    ("apps/", Ring::CompositionRoot),
    ("tests/", Ring::TestSupport),
];

impl Ring {
    /// The rings a crate in this ring may depend on, its own shared kernels
    /// aside (see [`edge_permitted`]).
    const fn may_depend_on(self) -> &'static [Self] {
        match self {
            Self::Domain | Self::Application => &[Self::Domain],
            Self::SecondaryAdapter => &[Self::Application, Self::Domain],
            Self::CompositionRoot => &[
                Self::Domain,
                Self::Application,
                Self::SecondaryAdapter,
                Self::CompositionRoot,
            ],
            Self::TestSupport => &[
                Self::Domain,
                Self::Application,
                Self::SecondaryAdapter,
                Self::CompositionRoot,
                Self::TestSupport,
            ],
        }
    }

    const fn is_adapter(self) -> bool {
        matches!(self, Self::SecondaryAdapter)
    }

    /// External crate families this ring may not use. Domain and application
    /// stay free of the runtime, transport, and telemetry frameworks that
    /// belong to adapters and the composition root; domain also gives up the
    /// `tracing` facade, which application may use. `serde` and `serde_json`
    /// are allowed everywhere (decisions.md, "serde derives allowed in the
    /// domain"). Each list is pinned by a test on the rule a finding quotes.
    const fn forbidden_families(self) -> &'static [&'static str] {
        match self {
            Self::Domain => &[
                "tokio",
                "reqwest",
                "tracing",
                "opentelemetry",
                "rmcp",
                "tonic",
                "axum",
                "hyper",
            ],
            Self::Application => &[
                "tokio",
                "reqwest",
                "opentelemetry",
                "rmcp",
                "tonic",
                "axum",
                "hyper",
            ],
            Self::SecondaryAdapter | Self::CompositionRoot | Self::TestSupport => &[],
        }
    }
}

/// The ring a member path belongs to, by its leading directories. A prefix is
/// never the whole path: [`Workspace::load`] refuses a trailing `/`.
pub fn classify(member_path: &str) -> Option<Ring> {
    RINGS
        .iter()
        .find(|(prefix, _)| member_path.starts_with(prefix))
        .map(|(_, ring)| *ring)
}

/// Whether a crate is an adapter shared kernel: it sits under
/// `crates/adapters/secondary/shared/`. A kernel is in the adapter ring, may
/// be depended on by the adapters beside it, and implements no port.
pub fn is_kernel(member_path: &str) -> bool {
    member_path.starts_with("crates/adapters/secondary/shared/")
}

/// Whether a crate in ring `from` may depend on a crate in ring `to`: the
/// inward rule, plus the one intra-ring edge, from an adapter (or a kernel) to
/// a shared kernel of its own ring. Nothing but test support reaches test
/// support.
fn edge_permitted(from: Ring, to: Ring, to_is_kernel: bool) -> bool {
    from.may_depend_on().contains(&to) || (from.is_adapter() && to == from && to_is_kernel)
}

/// The rule an impermissible edge breaks, in words.
fn edge_rule(from: Ring, to: Ring) -> String {
    if to == Ring::TestSupport {
        return "only tests/ crates and [dev-dependencies] may depend on a tests/ crate".to_owned();
    }
    if from.is_adapter() && to == from {
        return "an adapter may not depend on a sibling adapter; code two adapters share \
                belongs in a shared kernel under shared/"
            .to_owned();
    }
    let allowed: Vec<String> = from
        .may_depend_on()
        .iter()
        .map(ToString::to_string)
        .collect();
    let kernels = if from.is_adapter() {
        ", and shared kernels of its own ring"
    } else {
        ""
    };
    format!("{from} may depend only on {}{kernels}", allowed.join(", "))
}

/// The forbidden family `name` belongs to, if any. A crate belongs to a family
/// when any `-` or `_` separated part of its name is the family name, so
/// `opentelemetry` covers `opentelemetry_sdk`, `opentelemetry-otlp`, and
/// `tracing-opentelemetry` alike.
fn forbidden_family(ring: Ring, name: &str) -> Option<&'static str> {
    let normalised = normalise(name);
    ring.forbidden_families()
        .iter()
        .find(|family| normalised.split('_').any(|part| part == **family))
        .copied()
}

// --- The sweep ---

/// Every finding in the workspace, in member order. A finding is one line: the
/// crate, its directory and ring, the dependency, and the rule it breaks.
pub fn lint(workspace: &Workspace) -> Vec<String> {
    let mut findings = Vec::new();
    for member in &workspace.members {
        let Some(ring) = classify(&member.path) else {
            let directories: Vec<&str> = RINGS.iter().map(|(prefix, _)| *prefix).collect();
            findings.push(format!(
                "{} ({}): in no ring. Rule: every workspace member lives under one of {}",
                member.name,
                member.path,
                directories.join(", ")
            ));
            continue;
        };
        let mut report = |detail: String| {
            findings.push(format!(
                "{} ({}, {ring}): {detail}",
                member.name, member.path
            ));
        };
        for section in member.manifest.dependency_sections() {
            let header = &section.header;
            for (key, spec) in section.entries {
                match resolve(workspace, key, spec) {
                    Err(gap) => report(format!("{header} `{key}` {gap}")),
                    Ok(_) if section.dev => {}
                    // A target in no ring is reported once, as that member.
                    Ok(Dependency::Internal(target)) => {
                        if let Some(target_ring) = classify(&target.path)
                            && !edge_permitted(ring, target_ring, is_kernel(&target.path))
                        {
                            report(format!(
                                "{header} depends on {} ({}, {target_ring}). Rule: {}",
                                target.name,
                                target.path,
                                edge_rule(ring, target_ring)
                            ));
                        }
                    }
                    Ok(Dependency::External(name)) => {
                        if let Some(family) = forbidden_family(ring, name) {
                            let declared = if name == key {
                                String::new()
                            } else {
                                format!(" (declared as `{key}`)")
                            };
                            report(format!(
                                "{header} depends on {name}{declared}, of the `{family}` \
                                 family. Rule: {ring} may not use {}",
                                ring.forbidden_families().join(", ")
                            ));
                        }
                    }
                }
            }
        }
    }
    findings
}

/// What a member's dependency is, once followed into `[workspace.dependencies]`.
enum Dependency<'a> {
    /// The listed member the entry's `path` names.
    Internal(&'a Member),
    /// A registry crate, by its real name: the entry's `package`, or its key.
    External(&'a str),
}

/// Follows `key.workspace = true` to its entry. The error is a gap: a
/// dependency the rules cannot judge, which `lint-manifests` or cargo rejects.
fn resolve<'a>(
    workspace: &'a Workspace,
    key: &'a str,
    spec: &toml::Value,
) -> Result<Dependency<'a>, String> {
    if !inherits_workspace(spec) {
        return Err(format!(
            "is not inherited with `workspace = true`, so its real name and path are not in \
             [workspace.dependencies], the one place this lint reads them; declare it once in \
             that table and write `{key}.workspace = true` here"
        ));
    }
    let entry = workspace.dependencies.get(key).ok_or_else(|| {
        format!(
            "is inherited, but [workspace.dependencies] has no such entry; add `{key}` to that \
             table in the workspace Cargo.toml, or remove it here"
        )
    })?;
    let name = entry
        .get("package")
        .and_then(toml::Value::as_str)
        .unwrap_or(key);
    match entry.get("path") {
        Some(path) => path
            .as_str()
            .and_then(|path| workspace.member_at(path))
            .map(Dependency::Internal)
            .ok_or_else(|| {
                format!(
                    "has the path {path}, which is not a directory listed in [workspace] \
                     members. Rule: every internal crate is a member in exactly one ring"
                )
            }),
        None if workspace.member_named(name).is_some() => Err(format!(
            "is a registry crate that shares the name of the workspace member {name}. Rule: \
             every internal crate is a member in exactly one ring"
        )),
        None => Ok(Dependency::External(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::{FixtureWorkspace, assert_findings};

    const MODEL: &str = "crates/domain/model";
    const POLICY: &str = "crates/domain/policy";
    const RUN: &str = "crates/application/run";
    const FAKE: &str = "crates/adapters/secondary/provider-fake";
    const OTEL: &str = "crates/adapters/secondary/telemetry-otel";
    const REGISTRY: &str = "crates/adapters/secondary/shared/telemetry-registry";
    const ALL_RINGS: [Ring; 5] = [
        Ring::Domain,
        Ring::Application,
        Ring::SecondaryAdapter,
        Ring::CompositionRoot,
        Ring::TestSupport,
    ];

    // --- Classification ---

    #[test]
    fn a_member_path_classifies_by_its_leading_directories() {
        for (path, ring) in [
            (MODEL, Some(Ring::Domain)),
            (RUN, Some(Ring::Application)),
            (FAKE, Some(Ring::SecondaryAdapter)),
            // A shared kernel classifies into its ring like any sibling.
            (REGISTRY, Some(Ring::SecondaryAdapter)),
            ("apps/lablet", Some(Ring::CompositionRoot)),
            ("tests/mcp-server", Some(Ring::TestSupport)),
            ("xtask", None),
            ("crates/foo", None),
            // Spec §2 has no primary adapters, so there is no such ring.
            ("crates/adapters/primary/http", None),
        ] {
            assert_eq!(classify(path), ring, "{path}");
        }
    }

    #[test]
    fn only_a_shared_directory_in_the_adapter_ring_is_a_kernel() {
        assert!(is_kernel(REGISTRY));
        for path in [
            FAKE,
            "crates/adapters/primary/shared/http-util",
            "crates/domain/shared/model",
            "apps/shared/config",
        ] {
            assert!(!is_kernel(path), "{path}");
        }
    }

    // --- The edge table ---

    #[test]
    fn edges_point_inward_and_a_kernel_opens_only_its_own_adapter_ring() {
        let reachable = |from| match from {
            Ring::Domain | Ring::Application => &ALL_RINGS[..1],
            Ring::SecondaryAdapter => &ALL_RINGS[..2],
            Ring::CompositionRoot => &ALL_RINGS[..4],
            Ring::TestSupport => &ALL_RINGS[..],
        };
        for from in ALL_RINGS {
            for to in ALL_RINGS {
                let inward = reachable(from).contains(&to);
                assert_eq!(edge_permitted(from, to, false), inward, "{from} -> {to}");
                let own_kernel = from == Ring::SecondaryAdapter && to == from;
                assert_eq!(
                    edge_permitted(from, to, true),
                    inward || own_kernel,
                    "{from} -> {to} kernel"
                );
            }
        }
    }

    // --- Forbidden families ---

    #[test]
    fn a_family_covers_both_spellings_and_every_member_crate() {
        for name in [
            "opentelemetry",
            "opentelemetry_sdk",
            "opentelemetry-otlp",
            "tracing-opentelemetry",
        ] {
            assert_eq!(
                forbidden_family(Ring::Application, name),
                Some("opentelemetry"),
                "{name}"
            );
        }
        assert_eq!(forbidden_family(Ring::Domain, "tokio-util"), Some("tokio"));
        assert_eq!(forbidden_family(Ring::Domain, "hyper_util"), Some("hyper"));
        // A name that only contains a family name is not in the family.
        for name in ["hyperloglog", "axum2", "thiserror", "async-trait", "tower"] {
            assert_eq!(forbidden_family(Ring::Domain, name), None, "{name}");
        }
    }

    #[test]
    fn serde_is_allowed_everywhere_and_only_the_inner_rings_forbid_anything() {
        for ring in ALL_RINGS {
            assert_eq!(forbidden_family(ring, "serde"), None, "{ring}");
            assert_eq!(forbidden_family(ring, "serde_json"), None, "{ring}");
            let inner = matches!(ring, Ring::Domain | Ring::Application);
            assert_eq!(ring.forbidden_families().is_empty(), !inner, "{ring}");
        }
    }

    // --- The sweep, over fixture workspaces on disk ---

    /// What every fixture workspace offers for inheritance. An internal entry
    /// is read only when a member inherits it.
    const ENTRIES: &str = r#"
tokio = "=1.0.0"
tracing = "=0.1.0"
tonic-build = "=0.14.0"
otel = { package = "opentelemetry", version = "=0.30.0" }
lablet-model = { path = "crates/domain/model", version = "0.1.0" }
lablet-run = { path = "crates/application/run", version = "0.1.0" }
lablet-provider-fake = { path = "crates/adapters/secondary/provider-fake", version = "0.1.0" }
lablet-telemetry-registry = { path = "crates/adapters/secondary/shared/telemetry-registry", version = "0.1.0" }
lablet-http-util = { path = "crates/adapters/secondary/shared/http-util", version = "0.1.0" }
lablet-conformance = { path = "tests/conformance", version = "0.1.0" }
lablet-test-mcp-server = { path = "tests/mcp-server", version = "0.1.0" }
"#;

    /// A dependency table inheriting each of `keys`.
    fn inherit(table: &str, keys: &[&str]) -> String {
        let entries: Vec<String> = keys
            .iter()
            .map(|key| format!("{key}.workspace = true\n"))
            .collect();
        format!("[{table}]\n{}\n", entries.concat())
    }

    /// The base every fixture starts from: one clean crate per inner ring.
    fn base() -> FixtureWorkspace {
        FixtureWorkspace::new(ENTRIES)
            .member(MODEL, "lablet-model", "")
            .member(
                RUN,
                "lablet-run",
                &inherit("dependencies", &["lablet-model"]),
            )
    }

    /// The findings with a second domain crate, `lablet-policy`, added.
    fn lint_policy(tables: &str) -> Vec<String> {
        lint(&base().member(POLICY, "lablet-policy", tables).load())
    }

    #[test]
    fn a_clean_fixture_has_no_findings() {
        let app = ["lablet-provider-fake", "lablet-run", "tokio"];
        let workspace = base()
            .member(
                FAKE,
                "lablet-provider-fake",
                &inherit("dependencies", &["lablet-run", "lablet-model", "tokio"]),
            )
            .member("apps/lablet", "lablet", &inherit("dependencies", &app))
            .load();
        assert_findings(&lint(&workspace), &[]);
    }

    #[test]
    fn a_domain_crate_depending_on_an_application_crate_fails() {
        assert_findings(
            &lint_policy(&inherit("dependencies", &["lablet-run"])),
            &[
                "lablet-policy (crates/domain/policy, Domain): [dependencies] depends on \
                 lablet-run (crates/application/run, Application). Rule: Domain may depend only \
                 on Domain",
            ],
        );
    }

    #[test]
    fn a_domain_crate_depending_on_a_forbidden_family_fails_under_its_real_name() {
        assert_findings(
            &lint_policy(&inherit("dependencies", &["tokio", "otel"])),
            &[
                "depends on opentelemetry (declared as `otel`), of the `opentelemetry` family",
                "lablet-policy (crates/domain/policy, Domain): [dependencies] depends on tokio, \
                 of the `tokio` family. Rule: Domain may not use tokio, reqwest, tracing, \
                 opentelemetry, rmcp, tonic, axum, hyper",
            ],
        );
    }

    #[test]
    fn an_application_crate_may_use_tracing_and_none_of_the_other_families() {
        let workspace = base()
            .member(
                "crates/application/x",
                "lablet-x",
                &inherit("dependencies", &["tracing", "tokio"]),
            )
            .load();
        assert_findings(
            &lint(&workspace),
            &[
                "lablet-x (crates/application/x, Application): [dependencies] depends on tokio, \
                 of the `tokio` family. Rule: Application may not use tokio, reqwest, \
                 opentelemetry, rmcp, tonic, axum, hyper",
            ],
        );
    }

    #[test]
    fn build_and_target_dependencies_are_judged_like_dependencies() {
        let tables = format!(
            "{}{}",
            inherit("build-dependencies", &["tonic-build"]),
            inherit("target.'cfg(unix)'.dependencies", &["lablet-run"])
        );
        assert_findings(
            &lint_policy(&tables),
            &[
                "[build-dependencies] depends on tonic-build, of the `tonic` family",
                "[target.'cfg(unix)'.dependencies] depends on lablet-run",
            ],
        );
    }

    #[test]
    fn a_dev_dependency_pointing_outward_passes() {
        let outward = ["lablet-run", "lablet-conformance", "tokio"];
        let workspace = base()
            .member("tests/conformance", "lablet-conformance", "")
            .member(
                POLICY,
                "lablet-policy",
                &inherit("dev-dependencies", &outward),
            )
            .load();
        assert_findings(&lint(&workspace), &[]);
    }

    #[test]
    fn an_adapter_reaches_a_shared_kernel_but_not_a_sibling_adapter() {
        let http_util = "crates/adapters/secondary/shared/http-util";
        let adapters = |otel: &[&str], registry: &[&str]| {
            let registry = inherit("dependencies", registry);
            let workspace = base()
                .member(FAKE, "lablet-provider-fake", "")
                .member(http_util, "lablet-http-util", "")
                .member(REGISTRY, "lablet-telemetry-registry", &registry)
                .member(
                    OTEL,
                    "lablet-telemetry-otel",
                    &inherit("dependencies", otel),
                )
                .load();
            lint(&workspace)
        };
        // Deliberate (spec §2): adapters and kernels alike reach "the shared
        // kernels of their own ring". No kernel can reach an adapter, so the
        // worst case is a chain of kernels.
        assert_findings(
            &adapters(
                &["lablet-telemetry-registry", "lablet-run"],
                &["lablet-http-util"],
            ),
            &[],
        );
        let sibling = "depends on lablet-provider-fake (crates/adapters/secondary/provider-fake, \
                       Secondary Adapter). Rule: an adapter may not depend on a sibling adapter";
        assert_findings(&adapters(&["lablet-provider-fake"], &[]), &[sibling]);
        assert_findings(&adapters(&[], &["lablet-provider-fake"]), &[sibling]);
    }

    #[test]
    fn only_a_tests_crate_may_ship_a_dependency_on_a_tests_crate() {
        let anything = ["lablet-test-mcp-server", "lablet-run", "tokio"];
        let conformance = ["lablet-conformance"];
        let workspace = base()
            .member("tests/mcp-server", "lablet-test-mcp-server", "")
            .member(
                "tests/conformance",
                "lablet-conformance",
                &inherit("dependencies", &anything),
            )
            .member(
                FAKE,
                "lablet-provider-fake",
                &inherit("build-dependencies", &conformance),
            )
            .member(
                "apps/lablet",
                "lablet",
                &inherit("dependencies", &conformance),
            )
            .load();
        let rule = "only tests/ crates and [dev-dependencies] may depend on a tests/ crate";
        assert_findings(
            &lint(&workspace),
            &[
                &format!(
                    "lablet-provider-fake ({FAKE}, Secondary Adapter): [build-dependencies] depends on lablet-conformance (tests/conformance, Test Support). Rule: {rule}"
                ),
                &format!(
                    "lablet (apps/lablet, Composition Root): [dependencies] depends on lablet-conformance (tests/conformance, Test Support). Rule: {rule}"
                ),
            ],
        );
    }

    #[test]
    fn a_member_in_no_ring_is_an_error() {
        let workspace = base().member("crates/misc", "lablet-misc", "").load();
        assert_findings(
            &lint(&workspace),
            &["lablet-misc (crates/misc): in no ring."],
        );
    }

    #[test]
    fn a_dependency_the_rules_cannot_judge_is_a_gap_in_any_table_never_a_pass() {
        // What lint-manifests or cargo rejects must not read as clean here.
        let not_a_member = "which is not a directory listed in [workspace] members";
        for (workspace_table, entry, gap) in [
            (
                ENTRIES,
                "lablet-run = { path = \"../../application/run\" }",
                "is not inherited",
            ),
            (
                ENTRIES,
                "rt = { package = \"tokio\", version = \"=1.0.0\" }",
                "is not inherited with `workspace = true`, so its real name and path are not in \
                 [workspace.dependencies], the one place this lint reads them; declare it once \
                 in that table and write `rt.workspace = true` here",
            ),
            (
                ENTRIES,
                "missing.workspace = true",
                "has no such entry; add `missing` to that table",
            ),
            (
                "helper = { path = \"../outside/helper\" }\n",
                "helper.workspace = true",
                not_a_member,
            ),
            (
                "lablet-model = { path = \"./crates/domain/model\" }\n",
                "lablet-model.workspace = true",
                not_a_member,
            ),
            (
                "lablet-model = \"=9.9.9\"\n",
                "lablet-model.workspace = true",
                "shares the name of the workspace member lablet-model",
            ),
        ] {
            for table in ["dependencies", "dev-dependencies"] {
                let workspace = FixtureWorkspace::new(workspace_table)
                    .member(MODEL, "lablet-model", "")
                    .member(RUN, "lablet-run", &format!("[{table}]\n{entry}\n"))
                    .load();
                let key = entry.split([' ', '.']).next().unwrap();
                let place = format!("Application): [{table}] `{key}` ");
                let findings = lint(&workspace);
                assert_findings(&findings, &[gap]);
                assert_findings(&findings, &[&place]);
            }
        }
    }

    #[test]
    fn the_real_workspace_has_no_violations() {
        let workspace = Workspace::load(&crate::workspace::workspace_root()).unwrap();
        assert!(!workspace.members.is_empty());
        assert_findings(&lint(&workspace), &[]);
    }
}
