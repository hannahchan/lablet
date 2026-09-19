//! Layer rules over the workspace (spec §2, contributing "Architecture
//! rules"). Each crate's ring is read from its path, and its `[dependencies]`
//! and `[build-dependencies]` are checked against what that ring may reach:
//! which workspace crates, matched by package name, and which external crate
//! families. `[dev-dependencies]` are exempt from the ring rules, but not from
//! the rule that every internal crate is a listed member: cargo makes any path
//! dependency under the workspace root a member, listed or not, and an
//! unlisted one is in no ring and read by no lint. One sweep reports every
//! finding.

use std::fmt;

use crate::workspace::{DependencyKind, DependencySpec, Member, Workspace, normalise};

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
    /// domain"). A test pins domain as a superset of application.
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

/// The ring a member path belongs to, by its leading directories.
pub fn classify(member_path: &str) -> Option<Ring> {
    RINGS
        .iter()
        .find(|(prefix, _)| {
            member_path
                .strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty())
        })
        .map(|(_, ring)| *ring)
}

/// Whether a crate is an adapter shared kernel: it sits under
/// `crates/adapters/secondary/shared/`. A kernel is in the adapter ring, may
/// be depended on by the adapters beside it, and implements no port.
pub fn is_kernel(member_path: &str) -> bool {
    member_path
        .strip_prefix("crates/adapters/secondary/shared/")
        .is_some_and(|rest| !rest.is_empty())
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

// --- Findings ---

/// What a finding is about, which decides the heading it is reported under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A dependency on a workspace crate that the ring rules do not permit.
    Layer,
    /// An external crate from a family the crate's ring forbids.
    ForbiddenDependency,
    /// A gap the rules cannot judge: a member in no ring, a path dependency
    /// that is not a member, or an inherited entry the workspace lacks.
    Classification,
}

impl Kind {
    /// Every kind, in reporting order.
    pub const ALL: [Self; 3] = [Self::Layer, Self::ForbiddenDependency, Self::Classification];

    /// The heading findings of this kind are reported under.
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Layer => "Layer dependency violations:",
            Self::ForbiddenDependency => "Forbidden external dependencies:",
            Self::Classification => "Classification gaps:",
        }
    }
}

/// One finding. `Display` names the crate, its ring, the dependency, and the
/// rule.
#[derive(Debug)]
pub struct Violation {
    /// The heading this finding belongs under.
    pub kind: Kind,
    /// The offending crate's package name.
    pub crate_name: String,
    /// The offending crate's directory in the workspace.
    pub crate_path: String,
    /// The offending crate's ring, when it has one.
    pub ring: Option<Ring>,
    /// What is wrong, and the rule it breaks.
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A root manifest with a `[package]` table is the member at no path.
        let path = if self.crate_path.is_empty() {
            "the workspace root"
        } else {
            &self.crate_path
        };
        write!(f, "{} ({path}", self.crate_name)?;
        if let Some(ring) = self.ring {
            write!(f, ", {ring}")?;
        }
        write!(f, "): {}", self.detail)
    }
}

// --- The sweep ---

/// Every finding in the workspace, in member order.
pub fn lint(workspace: &Workspace) -> Vec<Violation> {
    let mut violations = Vec::new();
    for member in &workspace.members {
        lint_member(workspace, member, &mut violations);
    }
    violations
}

fn lint_member(workspace: &Workspace, member: &Member, violations: &mut Vec<Violation>) {
    let ring = classify(&member.path);
    let mut report = |kind, detail: String| {
        violations.push(Violation {
            kind,
            crate_name: member.name.clone(),
            crate_path: member.path.clone(),
            ring,
            detail,
        });
    };
    let Some(ring) = ring else {
        report(
            Kind::Classification,
            format!(
                "in no ring. Rule: every workspace member lives under one of {}",
                ring_directories()
            ),
        );
        return;
    };

    for section in member.manifest.dependency_sections() {
        // Dev-dependencies are exempt from the ring and family rules only.
        let shipped = section.kind != DependencyKind::Dev;
        let header = &section.header;
        for (key, spec) in section.entries {
            let resolved = match resolve(workspace, &member.path, key, spec) {
                Ok(resolved) => resolved,
                Err(detail) => {
                    report(Kind::Classification, format!("{header} {detail}"));
                    continue;
                }
            };
            let name = resolved.name;
            let declared = if name == *key {
                String::new()
            } else {
                format!(" (declared as `{key}`)")
            };
            if let Some(target) = workspace.member_named(name) {
                // The name alone is not the member: the entry must also point
                // at the member's directory, or cargo builds something else.
                let points_at_target = resolved.path.is_some_and(|path| {
                    workspace
                        .member_at(resolved.declared_in, path)
                        .is_some_and(|found| found.path == target.path)
                });
                if !points_at_target {
                    let source = resolved.path.map_or_else(
                        || "from a registry or git source".to_owned(),
                        |path| format!("at `{path}`"),
                    );
                    report(
                        Kind::Classification,
                        format!(
                            "{header} depends on {name}{declared} {source}, which shares the \
                             name of the workspace member in {} but is not that member. Rule: \
                             every internal crate is a member in exactly one ring",
                            target.path
                        ),
                    );
                    continue;
                }
                let Some(target_ring) = classify(&target.path) else {
                    // The target is reported once, as a member in no ring.
                    continue;
                };
                if shipped && !edge_permitted(ring, target_ring, is_kernel(&target.path)) {
                    report(
                        Kind::Layer,
                        format!(
                            "{header} depends on {name}{declared} ({}, {target_ring}). Rule: {}",
                            target.path,
                            edge_rule(ring, target_ring)
                        ),
                    );
                }
            } else if let Some(path) = resolved.path {
                report(
                    Kind::Classification,
                    format!(
                        "{header} depends on {name}{declared} at `{path}`, which is not a \
                         workspace member. Rule: every internal crate is a member in exactly \
                         one ring"
                    ),
                );
            } else if let Some(family) = forbidden_family(ring, name).filter(|_| shipped) {
                report(
                    Kind::ForbiddenDependency,
                    format!(
                        "{header} depends on {name}{declared}, of the `{family}` family. \
                         Rule: {ring} may not use {}",
                        ring.forbidden_families().join(", ")
                    ),
                );
            }
        }
    }
}

/// A dependency's real crate name and, for a path dependency, its path and
/// the directory that path is relative to.
struct Resolved<'a> {
    name: &'a str,
    path: Option<&'a str>,
    /// The directory of the manifest that wrote `path`, relative to the
    /// workspace root: the member's, or the root for an inherited entry.
    declared_in: &'a str,
}

/// Follows `workspace = true` into `[workspace.dependencies]` and a renamed
/// key to its `package`. The error is a finding: an inherited key the
/// workspace table lacks, which cargo rejects and the rules cannot judge.
fn resolve<'a>(
    workspace: &'a Workspace,
    member_path: &'a str,
    key: &'a str,
    spec: &'a DependencySpec,
) -> Result<Resolved<'a>, String> {
    let (spec, declared_in) = if spec.inherits_workspace() {
        let inherited = workspace.table.dependencies.get(key).ok_or_else(|| {
            format!("inherits `{key}` with `workspace = true`, but [workspace.dependencies] has no `{key}`")
        })?;
        (inherited, "")
    } else {
        (spec, member_path)
    };
    Ok(Resolved {
        name: spec.package().unwrap_or(key),
        path: spec.path(),
        declared_in,
    })
}

/// The ring directories, for the diagnostic of a member in none of them.
fn ring_directories() -> String {
    RINGS
        .iter()
        .map(|(prefix, _)| *prefix)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::FixtureWorkspace;

    const MODEL: &str = "crates/domain/model";
    const RUN: &str = "crates/application/run";
    const FAKE: &str = "crates/adapters/secondary/provider-fake";
    const OTEL: &str = "crates/adapters/secondary/telemetry-otel";
    const REGISTRY: &str = "crates/adapters/secondary/shared/telemetry-registry";

    /// The third-party pins every fixture workspace offers for inheritance.
    const PINS: &str = r#"
tokio = "=1.0.0"
tracing = "=0.1.0"
opentelemetry_sdk = "=0.30.0"
serde = "=1.0.0"
serde_json = "=1.0.0"
otel = { package = "opentelemetry", version = "=0.30.0" }
lablet-model = { path = "crates/domain/model" }
"#;

    // --- Classification ---

    #[test]
    fn a_member_path_classifies_by_its_leading_directories() {
        for (path, ring) in [
            (MODEL, Ring::Domain),
            ("crates/domain/policy", Ring::Domain),
            (RUN, Ring::Application),
            (FAKE, Ring::SecondaryAdapter),
            // A shared kernel classifies into its ring like any sibling.
            (REGISTRY, Ring::SecondaryAdapter),
            ("apps/lablet", Ring::CompositionRoot),
            ("tests/conformance", Ring::TestSupport),
            ("tests/mcp-server", Ring::TestSupport),
        ] {
            assert_eq!(classify(path), Some(ring), "{path}");
        }
    }

    #[test]
    fn a_path_outside_the_rings_classifies_as_nothing() {
        for path in [
            "xtask",
            "crates/foo",
            "crates/adapters/foo",
            // Spec §2 has no primary adapters, so there is no such ring.
            "crates/adapters/primary/http",
            "crates/domain/",
            "telemetry",
        ] {
            assert_eq!(classify(path), None, "{path}");
        }
    }

    #[test]
    fn only_a_shared_directory_in_an_adapter_ring_is_a_kernel() {
        assert!(is_kernel(REGISTRY));
        assert!(!is_kernel("crates/adapters/primary/shared/http-util"));
        assert!(!is_kernel(FAKE));
        assert!(!is_kernel("crates/domain/shared/model"));
        assert!(!is_kernel("crates/application/shared/retry"));
        assert!(!is_kernel("apps/shared/config"));
        assert!(!is_kernel("crates/adapters/secondary/shared/"));
    }

    // --- The edge table ---

    #[test]
    fn domain_and_application_reach_only_domain() {
        for from in [Ring::Domain, Ring::Application] {
            assert!(edge_permitted(from, Ring::Domain, false));
            for to in [
                Ring::Application,
                Ring::SecondaryAdapter,
                Ring::CompositionRoot,
                Ring::TestSupport,
            ] {
                assert!(!edge_permitted(from, to, false), "{from} -> {to}");
                assert!(!edge_permitted(from, to, true), "{from} -> {to} kernel");
            }
        }
    }

    #[test]
    fn an_adapter_reaches_inward_and_its_own_kernels_only() {
        let ring = Ring::SecondaryAdapter;
        assert!(edge_permitted(ring, Ring::Application, false));
        assert!(edge_permitted(ring, Ring::Domain, false));
        assert!(edge_permitted(ring, ring, true));
        assert!(!edge_permitted(ring, ring, false));
        assert!(!edge_permitted(ring, Ring::CompositionRoot, false));
        assert!(!edge_permitted(ring, Ring::TestSupport, false));
    }

    #[test]
    fn the_composition_root_reaches_everything_but_test_support() {
        for to in [
            Ring::Domain,
            Ring::Application,
            Ring::SecondaryAdapter,
            Ring::CompositionRoot,
        ] {
            assert!(edge_permitted(Ring::CompositionRoot, to, false), "{to}");
        }
        assert!(!edge_permitted(
            Ring::CompositionRoot,
            Ring::TestSupport,
            false
        ));
    }

    #[test]
    fn test_support_reaches_anything() {
        for (_, to) in RINGS {
            assert!(edge_permitted(Ring::TestSupport, *to, false), "{to}");
        }
    }

    // --- Forbidden families ---

    #[test]
    fn a_family_covers_both_spellings_and_every_member_crate() {
        for name in [
            "opentelemetry",
            "opentelemetry_sdk",
            "opentelemetry-otlp",
            "opentelemetry-proto",
            "opentelemetry-semantic-conventions",
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
        assert_eq!(
            forbidden_family(Ring::Domain, "tracing-subscriber"),
            Some("tracing")
        );
    }

    #[test]
    fn a_name_that_only_contains_a_family_name_is_not_in_the_family() {
        for name in ["hyperloglog", "axum2", "thiserror", "async-trait", "tower"] {
            assert_eq!(forbidden_family(Ring::Domain, name), None, "{name}");
        }
    }

    #[test]
    fn serde_is_allowed_in_every_ring() {
        for (_, ring) in RINGS {
            assert_eq!(forbidden_family(*ring, "serde"), None, "{ring}");
            assert_eq!(forbidden_family(*ring, "serde_json"), None, "{ring}");
        }
    }

    #[test]
    fn application_forbids_what_domain_does_except_tracing() {
        let domain = Ring::Domain.forbidden_families();
        let application = Ring::Application.forbidden_families();
        for family in application {
            assert!(domain.contains(family), "`{family}` is missing from domain");
        }
        let only_domain: Vec<_> = domain
            .iter()
            .filter(|family| !application.contains(family))
            .collect();
        assert_eq!(only_domain, [&"tracing"]);
    }

    #[test]
    fn the_outer_rings_forbid_nothing() {
        for ring in [
            Ring::SecondaryAdapter,
            Ring::CompositionRoot,
            Ring::TestSupport,
        ] {
            assert!(ring.forbidden_families().is_empty(), "{ring}");
        }
    }

    // --- The sweep, over fixture workspaces on disk ---

    /// The base every fixture starts from: one clean crate per inner ring.
    fn base() -> FixtureWorkspace {
        FixtureWorkspace::new(PINS)
            .member(MODEL, "lablet-model", "")
            .member(
                RUN,
                "lablet-run",
                "[dependencies]\nlablet-model = { path = \"../../domain/model\" }\n",
            )
    }

    fn one(violations: &[Violation]) -> &Violation {
        assert_eq!(violations.len(), 1, "{violations:#?}");
        &violations[0]
    }

    #[test]
    fn a_clean_fixture_has_no_findings() {
        let workspace = base()
            .member(
                FAKE,
                "lablet-provider-fake",
                "[dependencies]\nlablet-run = { path = \"../../../application/run\" }\n\
                 lablet-model.workspace = true\ntokio.workspace = true\n",
            )
            .member(
                "apps/lablet",
                "lablet",
                "[dependencies]\nlablet-provider-fake = { path = \"../../crates/adapters/secondary/provider-fake\" }\n\
                 lablet-run = { path = \"../../crates/application/run\" }\ntokio.workspace = true\n",
            )
            .load();
        let violations = lint(&workspace);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn a_domain_crate_depending_on_an_application_crate_fails() {
        let workspace = base()
            .member(
                "crates/domain/policy",
                "lablet-policy",
                "[dependencies]\nlablet-run = { path = \"../../application/run\" }\n",
            )
            .load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::Layer);
        assert_eq!(violation.crate_name, "lablet-policy");
        assert_eq!(violation.ring, Some(Ring::Domain));
        let message = violation.to_string();
        assert_eq!(
            message,
            "lablet-policy (crates/domain/policy, Domain): [dependencies] depends on lablet-run \
             (crates/application/run, Application). Rule: Domain may depend only on Domain"
        );
    }

    #[test]
    fn a_domain_crate_depending_on_tokio_fails() {
        let workspace = base()
            .member(
                "crates/domain/policy",
                "lablet-policy",
                "[dependencies]\ntokio.workspace = true\n",
            )
            .load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::ForbiddenDependency);
        assert!(
            violation
                .detail
                .starts_with("[dependencies] depends on tokio, of the `tokio` family"),
            "{violation}"
        );
        let message = violation.to_string();
        assert!(
            message.contains("lablet-policy (crates/domain/policy, Domain)"),
            "{message}"
        );
        assert!(
            message.contains("Rule: Domain may not use tokio, reqwest, tracing"),
            "{message}"
        );
    }

    #[test]
    fn an_adapter_depending_on_a_sibling_adapter_fails() {
        let workspace = base()
            .member(FAKE, "lablet-provider-fake", "")
            .member(
                OTEL,
                "lablet-telemetry-otel",
                "[dependencies]\nlablet-provider-fake = { path = \"../provider-fake\" }\n",
            )
            .load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::Layer);
        assert_eq!(violation.crate_name, "lablet-telemetry-otel");
        assert!(
            violation
                .detail
                .contains("depends on lablet-provider-fake ("),
            "{violation}"
        );
        assert!(
            violation
                .detail
                .contains("an adapter may not depend on a sibling adapter"),
            "{violation}"
        );
    }

    #[test]
    fn an_adapter_depending_on_a_shared_kernel_passes() {
        let workspace = base()
            .member(REGISTRY, "lablet-telemetry-registry", "")
            .member(
                OTEL,
                "lablet-telemetry-otel",
                "[dependencies]\nlablet-telemetry-registry = { path = \"../shared/telemetry-registry\" }\n\
                 lablet-run = { path = \"../../../application/run\" }\n",
            )
            .load();
        let violations = lint(&workspace);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn a_shared_kernel_depending_on_a_sibling_adapter_fails() {
        let workspace = base()
            .member(FAKE, "lablet-provider-fake", "")
            .member(
                REGISTRY,
                "lablet-telemetry-registry",
                "[dependencies]\nlablet-provider-fake = { path = \"../../provider-fake\" }\n",
            )
            .load();
        assert_eq!(one(&lint(&workspace)).kind, Kind::Layer);
    }

    #[test]
    fn a_shared_kernel_depending_on_another_shared_kernel_passes() {
        // Deliberate (spec §2): adapters and kernels alike reach "the shared
        // kernels of their own ring". Cargo refuses cycles, and no kernel can
        // reach an adapter, so the worst case is a chain of kernels.
        let workspace = base()
            .member(
                "crates/adapters/secondary/shared/http-util",
                "lablet-http-util",
                "",
            )
            .member(
                REGISTRY,
                "lablet-telemetry-registry",
                "[dependencies]\nlablet-http-util = { path = \"../http-util\" }\n",
            )
            .load();
        let violations = lint(&workspace);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn a_dev_dependency_pointing_outward_passes() {
        let workspace = base()
            .member("tests/conformance", "lablet-conformance", "")
            .member(
                "crates/domain/policy",
                "lablet-policy",
                "[dev-dependencies]\nlablet-run = { path = \"../../application/run\" }\n\
                 lablet-conformance = { path = \"../../../tests/conformance\" }\n\
                 tokio.workspace = true\n",
            )
            .load();
        let violations = lint(&workspace);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn a_renamed_forbidden_dependency_fails_under_its_real_name() {
        let workspace = base()
            .member(
                "crates/domain/policy",
                "lablet-policy",
                "[dependencies]\nrt = { package = \"tokio\", version = \"=1.0.0\" }\n",
            )
            .load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::ForbiddenDependency);
        assert!(
            violation
                .detail
                .contains("depends on tokio (declared as `rt`)"),
            "{violation}"
        );
    }

    #[test]
    fn a_forbidden_dependency_renamed_in_the_workspace_table_fails() {
        let workspace = base()
            .member(
                "crates/domain/policy",
                "lablet-policy",
                "[dependencies]\notel.workspace = true\n",
            )
            .load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::ForbiddenDependency);
        assert!(
            violation
                .detail
                .contains("depends on opentelemetry (declared as `otel`)"),
            "{violation}"
        );
    }

    #[test]
    fn a_workspace_inherited_internal_crate_is_judged_by_its_ring() {
        let workspace =
            FixtureWorkspace::new("lablet-run = { path = \"crates/application/run\" }\n")
                .member(RUN, "lablet-run", "")
                .member(
                    MODEL,
                    "lablet-model",
                    "[dependencies]\nlablet-run.workspace = true\n",
                )
                .load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::Layer);
        assert!(
            violation.detail.contains("depends on lablet-run ("),
            "{violation}"
        );
    }

    #[test]
    fn build_and_target_dependencies_are_judged_like_dependencies() {
        let workspace = base()
            .member(
                "crates/domain/policy",
                "lablet-policy",
                "[build-dependencies]\ntonic-build = \"=0.14.0\"\n\n\
                 [target.'cfg(unix)'.dependencies]\nlablet-run = { path = \"../../application/run\" }\n",
            )
            .load();
        let violations = lint(&workspace);
        assert_eq!(violations.len(), 2, "{violations:#?}");
        assert!(violations.iter().any(|v| {
            v.kind == Kind::ForbiddenDependency
                && v.detail
                    .starts_with("[build-dependencies] depends on tonic-build")
        }));
        assert!(violations.iter().any(|v| {
            v.kind == Kind::Layer && v.detail.starts_with("[target.'cfg(unix)'.dependencies]")
        }));
    }

    #[test]
    fn a_shipped_dependency_on_a_tests_crate_fails_outside_tests() {
        let workspace = base()
            .member("tests/conformance", "lablet-conformance", "")
            .member(
                FAKE,
                "lablet-provider-fake",
                "[build-dependencies]\nlablet-conformance = { path = \"../../../../tests/conformance\" }\n",
            )
            .member(
                "apps/lablet",
                "lablet",
                "[dependencies]\nlablet-conformance = { path = \"../../tests/conformance\" }\n",
            )
            .load();
        let violations = lint(&workspace);
        assert_eq!(violations.len(), 2, "{violations:#?}");
        for violation in &violations {
            assert_eq!(violation.kind, Kind::Layer);
            assert!(
                violation.detail.contains(
                    "only tests/ crates and [dev-dependencies] may depend on a tests/ crate"
                ),
                "{violation}"
            );
        }
    }

    #[test]
    fn a_tests_crate_may_depend_on_anything() {
        let workspace = base()
            .member(FAKE, "lablet-provider-fake", "")
            .member("tests/mcp-server", "lablet-test-mcp-server", "")
            .member(
                "tests/conformance",
                "lablet-conformance",
                "[dependencies]\nlablet-provider-fake = { path = \"../../crates/adapters/secondary/provider-fake\" }\n\
                 lablet-test-mcp-server = { path = \"../mcp-server\" }\ntokio.workspace = true\n",
            )
            .load();
        let violations = lint(&workspace);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn a_member_in_no_ring_is_an_error() {
        let workspace = base().member("crates/misc", "lablet-misc", "").load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::Classification);
        assert_eq!(violation.ring, None);
        assert!(
            violation
                .to_string()
                .starts_with("lablet-misc (crates/misc): in no ring."),
            "{violation}"
        );
    }

    #[test]
    fn a_path_dependency_that_is_not_a_member_is_a_classification_gap() {
        let workspace = base()
            .member(
                "crates/domain/policy",
                "lablet-policy",
                "[dependencies]\nhelper = { path = \"../../../../helper\" }\n",
            )
            .load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::Classification);
        assert!(
            violation.detail.contains("not a workspace member"),
            "{violation}"
        );
    }

    #[test]
    fn a_dev_path_dependency_that_is_not_a_member_is_a_classification_gap() {
        // Cargo makes the first an unlisted workspace member and builds the
        // second from outside the repository; neither is in a ring.
        for path in ["../../../tests/testkit", "../../../../outside/wiremock"] {
            let workspace = base()
                .member(
                    "crates/domain/policy",
                    "lablet-policy",
                    &format!("[dev-dependencies]\nhelper = {{ path = \"{path}\" }}\n"),
                )
                .load();
            let violations = lint(&workspace);
            let violation = one(&violations);
            assert_eq!(violation.kind, Kind::Classification);
            assert!(
                violation
                    .detail
                    .starts_with("[dev-dependencies] depends on helper at `"),
                "{violation}"
            );
            assert!(
                violation.detail.contains("not a workspace member"),
                "{violation}"
            );
        }
    }

    #[test]
    fn an_inherited_dev_dependency_whose_path_is_not_a_member_is_a_classification_gap() {
        let workspace = FixtureWorkspace::new("lablet-testkit = { path = \"tests/testkit\" }\n")
            .member(
                MODEL,
                "lablet-model",
                "[dev-dependencies]\nlablet-testkit.workspace = true\n",
            )
            .load();
        let violations = lint(&workspace);
        assert_eq!(one(&violations).kind, Kind::Classification);
    }

    #[test]
    fn a_dependency_with_a_members_name_that_is_not_that_member_is_a_classification_gap() {
        // Each entry is named like the domain crate lablet-model, which the
        // application ring may depend on, and is something else.
        for (workspace_table, entry, source) in [
            // A path outside the workspace.
            (
                "",
                "lablet-model = { path = \"../../../../outside/model\" }",
                "at `../../../../outside/model`",
            ),
            // The same, behind a renamed key.
            (
                "",
                "helper = { package = \"lablet-model\", path = \"../../../../outside/model\" }",
                "at `../../../../outside/model`",
            ),
            // Another member's directory.
            (
                "",
                "lablet-model = { path = \"../../domain/policy\" }",
                "at `../../domain/policy`",
            ),
            // The workspace entry repointed outside the workspace.
            (
                "lablet-model = { path = \"../outside/model\" }\n",
                "lablet-model.workspace = true",
                "at `../outside/model`",
            ),
            // A registry pin under a member's name.
            (
                "lablet-model = \"=9.9.9\"\n",
                "lablet-model.workspace = true",
                "from a registry or git source",
            ),
        ] {
            let workspace = FixtureWorkspace::new(workspace_table)
                .member(MODEL, "lablet-model", "")
                .member("crates/domain/policy", "lablet-policy", "")
                .member(RUN, "lablet-run", &format!("[dependencies]\n{entry}\n"))
                .load();
            let violations = lint(&workspace);
            let violation = one(&violations);
            assert_eq!(violation.kind, Kind::Classification, "{entry}");
            assert!(
                violation.detail.contains(&format!(
                    "{source}, which shares the name of the workspace member in \
                     crates/domain/model but is not that member"
                )),
                "{violation}"
            );
        }
    }

    #[test]
    fn a_root_package_is_a_member_in_no_ring() {
        let dir = crate::workspace::fixture::TempDir::new("root-package");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"lablet-root\"\n\n[workspace]\nmembers = []\n",
        );
        let workspace = Workspace::load(dir.path()).unwrap();
        let violations = lint(&workspace);
        assert!(
            one(&violations)
                .to_string()
                .starts_with("lablet-root (the workspace root): in no ring."),
            "{violations:#?}"
        );
    }

    #[test]
    fn an_inherited_key_missing_from_the_workspace_table_is_reported() {
        let workspace = base()
            .member(
                "crates/domain/policy",
                "lablet-policy",
                "[dependencies]\nmissing.workspace = true\n",
            )
            .load();
        let violations = lint(&workspace);
        let violation = one(&violations);
        assert_eq!(violation.kind, Kind::Classification);
        assert!(
            violation
                .detail
                .contains("[workspace.dependencies] has no `missing`"),
            "{violation}"
        );
    }

    // --- The real workspace must be clean ---

    #[test]
    fn the_real_workspace_has_no_violations() {
        let root = crate::workspace::workspace_root();
        let workspace = Workspace::load(&root).unwrap();
        let violations = lint(&workspace);
        let listed: Vec<String> = violations.iter().map(ToString::to_string).collect();
        assert!(
            listed.is_empty(),
            "the workspace breaks the layer rules:\n  {}",
            listed.join("\n  ")
        );
    }
}
