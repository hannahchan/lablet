//! Layer rules over the workspace (spec §2, contributing "Architecture
//! rules"). A crate's ring is read from its path, and its dependencies, dev
//! ones aside, are checked against what that ring may reach. The rings are
//! those of contributing "Architecture rules", the adapters' shared kernels
//! being a part of the adapter ring, and a forbidden family may
//! let through the one crate named like the family: every ring but the
//! domain, the CLI root and test support forbids the `opentelemetry` family
//! and allows `opentelemetry` itself, the API, since instrumented code
//! depends on the API and only the process that runs it installs the SDK,
//! which in lablet is the CLI root alone. The two composition roots never
//! depend on each other: what both build alike lives in the composition
//! roots' kernels under `apps/shared/`.
//!
//! `lint-manifests` makes every dependency of a member an inherited entry of
//! `[workspace.dependencies]`, so that table is where a crate's real name and
//! path are read. An entry this lint cannot judge is a finding, never a pass.

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
    /// `apps/shared/*`: what both composition roots construct alike, one
    /// capability to a crate. A kernel wires nothing and has no `main()`.
    RootKernel,
    /// `apps/lablet`: the library root, lablet's public library API, which
    /// runs on the OpenTelemetry its host provides and configures no SDK.
    LibraryRoot,
    /// `apps/lablet-cli`: the CLI root, `main`, the command line, and the
    /// SDK it configures from the config and the environment.
    CliRoot,
    /// `tests/*`: test support, reached only through `[dev-dependencies]`.
    TestSupport,
}

impl fmt::Display for Ring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Domain => "Domain",
            Self::Application => "Application",
            Self::SecondaryAdapter => "Secondary Adapter",
            Self::RootKernel => "Root Kernel",
            Self::LibraryRoot => "Library Root",
            Self::CliRoot => "CLI Root",
            Self::TestSupport => "Test Support",
        })
    }
}

/// Whether the crate named exactly like a forbidden family is let through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Namesake {
    Forbidden,
    Allowed,
}

/// The prefixes are disjoint, so a member falls in at most one ring; one that
/// falls in none is a finding. Spec §2 has no primary adapters, so there is no
/// ring for `crates/adapters/primary/`. The two roots are named whole, so a
/// third crate under `apps/` is in no ring until a rule places it.
const RINGS: &[(&str, Ring)] = &[
    ("crates/domain/", Ring::Domain),
    ("crates/application/", Ring::Application),
    ("crates/adapters/secondary/", Ring::SecondaryAdapter),
    ("apps/shared/", Ring::RootKernel),
    ("apps/lablet/", Ring::LibraryRoot),
    ("apps/lablet-cli/", Ring::CliRoot),
    ("tests/", Ring::TestSupport),
];

impl Ring {
    /// Its own shared kernels aside; see [`edge_permitted`].
    const fn may_depend_on(self) -> &'static [Self] {
        match self {
            Self::Domain | Self::Application => &[Self::Domain],
            Self::SecondaryAdapter => &[Self::Application, Self::Domain],
            // A kernel is the one crate a root shares, so neither root is
            // in the other's list, and a kernel reaches neither.
            Self::RootKernel | Self::LibraryRoot | Self::CliRoot => &[
                Self::Domain,
                Self::Application,
                Self::SecondaryAdapter,
                Self::RootKernel,
            ],
            Self::TestSupport => &[
                Self::Domain,
                Self::Application,
                Self::SecondaryAdapter,
                Self::RootKernel,
                Self::LibraryRoot,
                Self::CliRoot,
                Self::TestSupport,
            ],
        }
    }

    const fn is_adapter(self) -> bool {
        matches!(self, Self::SecondaryAdapter)
    }

    /// External crate families this ring may not use: the runtime, transport,
    /// and telemetry frameworks belong to adapters and the composition roots.
    /// The application, the adapters, the library root and the kernels may
    /// name the OpenTelemetry API, `opentelemetry` itself, and no other
    /// member of its family, so the loop and the adapters instrument
    /// themselves, a library run takes its host's providers, and the SDK is
    /// the CLI root's alone. `serde` and `serde_json` are allowed everywhere
    /// (decisions.md, "serde derives allowed in the domain").
    const fn forbidden_families(self) -> &'static [(&'static str, Namesake)] {
        match self {
            Self::Domain => &[
                ("tokio", Namesake::Forbidden),
                ("reqwest", Namesake::Forbidden),
                ("tracing", Namesake::Forbidden),
                ("opentelemetry", Namesake::Forbidden),
                ("rmcp", Namesake::Forbidden),
                ("tonic", Namesake::Forbidden),
                ("axum", Namesake::Forbidden),
                ("hyper", Namesake::Forbidden),
            ],
            Self::Application => &[
                ("tokio", Namesake::Forbidden),
                ("reqwest", Namesake::Forbidden),
                ("rmcp", Namesake::Forbidden),
                ("tonic", Namesake::Forbidden),
                ("axum", Namesake::Forbidden),
                ("hyper", Namesake::Forbidden),
                ("opentelemetry", Namesake::Allowed),
            ],
            Self::SecondaryAdapter | Self::RootKernel | Self::LibraryRoot => {
                &[("opentelemetry", Namesake::Allowed)]
            }
            Self::CliRoot | Self::TestSupport => &[],
        }
    }

    /// The rule a finding quotes, naming what the ring may not use and the
    /// namesake it may.
    fn forbidden_rule(self) -> String {
        let mut whole = Vec::new();
        let mut excepted = Vec::new();
        for (family, namesake) in self.forbidden_families() {
            match namesake {
                Namesake::Forbidden => whole.push((*family).to_owned()),
                Namesake::Allowed => {
                    excepted.push(format!(
                        "any member of the {family} family but {family} itself"
                    ));
                }
            }
        }
        let excepted = match (whole.is_empty(), excepted.is_empty()) {
            (_, true) => String::new(),
            // A ring that forbids a family's members alone names them without
            // a list of whole families before.
            (true, false) => excepted.join(", or "),
            (false, false) => format!(", or {}", excepted.join(", or ")),
        };
        format!("{self} may not use {}{excepted}", whole.join(", "))
    }
}

/// The ring a member path belongs to. [`Workspace::load`] refuses a
/// trailing `/`, so one is added before matching: a ring's directory then
/// takes a crate it holds, and a root's takes that root and not one whose
/// name only begins like it.
pub fn classify(member_path: &str) -> Option<Ring> {
    let directory = format!("{member_path}/");
    RINGS
        .iter()
        .find(|(prefix, _)| directory.starts_with(prefix))
        .map(|(_, ring)| *ring)
}

/// Whether a crate is a shared kernel, which the adapters beside it may use.
pub fn is_kernel(member_path: &str) -> bool {
    member_path.starts_with("crates/adapters/secondary/shared/")
}

/// The inward rule, plus the one intra-ring edge: from an adapter or a kernel
/// to a shared kernel of its own ring.
fn edge_permitted(from: Ring, to: Ring, to_is_kernel: bool) -> bool {
    from.may_depend_on().contains(&to) || (from.is_adapter() && to == from && to_is_kernel)
}

fn edge_rule(from: Ring, to: Ring) -> String {
    if to == Ring::TestSupport {
        return "only tests/ crates and [dev-dependencies] may depend on a tests/ crate".to_owned();
    }
    if from.is_adapter() && to == from {
        return "an adapter may not depend on a sibling adapter; code two adapters share \
                belongs in a shared kernel under shared/"
            .to_owned();
    }
    if matches!(to, Ring::LibraryRoot | Ring::CliRoot) {
        return "a composition root may not be depended on; construction both roots need \
                belongs in a kernel under apps/shared/"
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

/// A crate belongs to a family when any `-` or `_` separated part of its name
/// is the family name, so `opentelemetry` covers `opentelemetry_sdk` and
/// `tracing-opentelemetry` alike. A family whose namesake is allowed lets
/// through the crate whose whole name is the family's, and nothing else.
fn forbidden_family(ring: Ring, name: &str) -> Option<&'static str> {
    let normalised = normalise(name);
    ring.forbidden_families()
        .iter()
        .find(|(family, namesake)| {
            normalised.split('_').any(|part| part == *family)
                && !(*namesake == Namesake::Allowed && normalised == *family)
        })
        .map(|(family, _)| *family)
}

/// Every finding in the workspace, in member order, one line each.
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
                                 family. Rule: {}",
                                ring.forbidden_rule()
                            ));
                        }
                    }
                }
            }
        }
    }
    findings
}

enum Dependency<'a> {
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
    const MCP: &str = "crates/adapters/secondary/tools-mcp";
    const DOCUMENTS: &str = "crates/adapters/secondary/shared/documents";
    const LIBRARY: &str = "apps/lablet";
    const CLI: &str = "apps/lablet-cli";
    const CONFIG: &str = "apps/shared/config";

    /// Declares `ALL_RINGS` beside a match with an arm for each ring it
    /// names and for nothing else, so a ring the enum gains doesn't compile
    /// here until the list has it, and neither does a ring named twice.
    macro_rules! every_ring {
        ($($ring:ident),+) => {
            const ALL_RINGS: [Ring; [$(stringify!($ring)),+].len()] = [$(Ring::$ring),+];

            #[deny(unreachable_patterns)]
            const _: () = match ALL_RINGS[0] {
                $(Ring::$ring)|+ => (),
            };
        };
    }

    every_ring!(
        Domain,
        Application,
        SecondaryAdapter,
        RootKernel,
        LibraryRoot,
        CliRoot,
        TestSupport
    );

    #[test]
    fn a_member_path_classifies_by_its_leading_directories() {
        for (path, ring) in [
            (MODEL, Some(Ring::Domain)),
            (RUN, Some(Ring::Application)),
            (FAKE, Some(Ring::SecondaryAdapter)),
            // A shared kernel classifies into its ring like any sibling.
            (DOCUMENTS, Some(Ring::SecondaryAdapter)),
            (CONFIG, Some(Ring::RootKernel)),
            (LIBRARY, Some(Ring::LibraryRoot)),
            (CLI, Some(Ring::CliRoot)),
            ("tests/mcp-server", Some(Ring::TestSupport)),
            // A root is named whole, so a crate whose name begins like one,
            // or a third crate under `apps/`, is in no ring.
            ("apps/lablet-server", None),
            ("apps/other", None),
            ("apps", None),
            ("xtask", None),
            ("crates/foo", None),
            ("crates/adapters/primary/http", None),
        ] {
            assert_eq!(classify(path), ring, "{path}");
        }
    }

    #[test]
    fn only_a_shared_directory_in_the_adapter_ring_is_a_kernel() {
        assert!(is_kernel(DOCUMENTS));
        for path in [
            FAKE,
            "crates/adapters/primary/shared/http-util",
            "crates/domain/shared/model",
            // The roots' kernels are a ring of their own, which the lint
            // places by its directory rather than by this test.
            CONFIG,
        ] {
            assert!(!is_kernel(path), "{path}");
        }
    }

    #[test]
    fn edges_point_inward_and_a_kernel_opens_only_its_own_adapter_ring() {
        let reachable = |from| match from {
            Ring::Domain | Ring::Application => &ALL_RINGS[..1],
            Ring::SecondaryAdapter => &ALL_RINGS[..2],
            Ring::RootKernel | Ring::LibraryRoot | Ring::CliRoot => &ALL_RINGS[..4],
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

    #[test]
    fn an_edge_breaks_the_sibling_rule_only_between_two_adapters() {
        assert_eq!(
            edge_rule(Ring::Application, Ring::Application),
            "Application may depend only on Domain"
        );
        assert_eq!(
            edge_rule(Ring::SecondaryAdapter, Ring::RootKernel),
            "Secondary Adapter may depend only on Application, Domain, and shared kernels of \
             its own ring"
        );
        assert!(
            edge_rule(Ring::SecondaryAdapter, Ring::SecondaryAdapter)
                .starts_with("an adapter may not depend on a sibling adapter"),
        );
        for (from, to) in [
            (Ring::CliRoot, Ring::LibraryRoot),
            (Ring::LibraryRoot, Ring::CliRoot),
            (Ring::RootKernel, Ring::LibraryRoot),
            (Ring::RootKernel, Ring::CliRoot),
        ] {
            assert!(
                edge_rule(from, to).starts_with("a composition root may not be depended on"),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn the_two_roots_never_depend_on_each_other_and_both_reach_a_kernel() {
        let roots = |library: &[&str], cli: &[&str], config: &[&str]| {
            let workspace = base()
                .member(CONFIG, "lablet-config", &inherit("dependencies", config))
                .member(LIBRARY, "lablet", &inherit("dependencies", library))
                .member(CLI, "lablet-cli", &inherit("dependencies", cli))
                .load();
            lint(&workspace)
        };
        assert_findings(
            &roots(
                &["lablet-config", "lablet-run"],
                &["lablet-config"],
                &["lablet-run"],
            ),
            &[],
        );
        let rule = "Rule: a composition root may not be depended on";
        assert_findings(
            &roots(&[], &["lablet"], &[]),
            &[&format!(
                "lablet-cli ({CLI}, CLI Root): [dependencies] depends on lablet ({LIBRARY}, \
                 Library Root). {rule}"
            )],
        );
        assert_findings(
            &roots(&["lablet-cli"], &[], &[]),
            &[&format!(
                "lablet ({LIBRARY}, Library Root): [dependencies] depends on lablet-cli ({CLI}, \
                 CLI Root). {rule}"
            )],
        );
        assert_findings(
            &roots(&[], &[], &["lablet"]),
            &[&format!(
                "lablet-config ({CONFIG}, Root Kernel): [dependencies] depends on lablet \
                 ({LIBRARY}, Library Root). {rule}"
            )],
        );
        // A test of either root may drive the other, as any test may.
        let workspace = base()
            .member(LIBRARY, "lablet", "")
            .member(CLI, "lablet-cli", &inherit("dev-dependencies", &["lablet"]))
            .load();
        assert_findings(&lint(&workspace), &[]);
    }

    #[test]
    fn only_the_cli_root_may_hold_the_opentelemetry_sdk() {
        for ring in [Ring::LibraryRoot, Ring::RootKernel] {
            assert_eq!(forbidden_family(ring, "opentelemetry"), None, "{ring}");
            assert_eq!(
                forbidden_family(ring, "opentelemetry_sdk"),
                Some("opentelemetry"),
                "{ring}"
            );
            // The roots' own crates are never forbidden by family.
            assert_eq!(forbidden_family(ring, "tokio"), None, "{ring}");
            assert_eq!(forbidden_family(ring, "tracing"), None, "{ring}");
        }
        for name in [
            "opentelemetry_sdk",
            "opentelemetry-otlp",
            "opentelemetry-propagator-b3",
        ] {
            assert_eq!(forbidden_family(Ring::CliRoot, name), None, "{name}");
        }
        assert_eq!(
            Ring::LibraryRoot.forbidden_rule(),
            "Library Root may not use any member of the opentelemetry family but opentelemetry \
             itself"
        );
        let workspace = base()
            .member(
                LIBRARY,
                "lablet",
                &inherit("dependencies", &["otel", "otel-sdk"]),
            )
            .member(
                CLI,
                "lablet-cli",
                &inherit("dependencies", &["otel", "otel-sdk"]),
            )
            .load();
        assert_findings(
            &lint(&workspace),
            &[&format!(
                "lablet ({LIBRARY}, Library Root): [dependencies] depends on opentelemetry_sdk \
                 (declared as `otel-sdk`), of the `opentelemetry` family. Rule: Library Root may \
                 not use any member of the opentelemetry family but opentelemetry itself"
            )],
        );
    }

    #[test]
    fn a_family_covers_both_spellings_and_every_member_crate() {
        // In the domain `tracing-opentelemetry` is of the `tracing` family
        // first, so the application, which allows `tracing`, shows the match.
        for name in [
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
        assert_eq!(
            forbidden_family(Ring::Domain, "opentelemetry"),
            Some("opentelemetry")
        );
        assert_eq!(forbidden_family(Ring::Domain, "tokio-util"), Some("tokio"));
        assert_eq!(forbidden_family(Ring::Domain, "hyper_util"), Some("hyper"));
        // A name that only contains a family name is not in the family.
        for name in ["hyperloglog", "axum2", "thiserror", "async-trait", "tower"] {
            assert_eq!(forbidden_family(Ring::Domain, name), None, "{name}");
        }
    }

    #[test]
    fn the_application_and_the_adapters_may_name_the_opentelemetry_api_and_no_other_member() {
        for ring in [Ring::Application, Ring::SecondaryAdapter] {
            assert_eq!(forbidden_family(ring, "opentelemetry"), None, "{ring}");
            for name in [
                "opentelemetry_sdk",
                "opentelemetry-otlp",
                "opentelemetry-proto",
                "opentelemetry-http",
                "opentelemetry-appender-tracing",
                "tracing-opentelemetry",
            ] {
                assert_eq!(
                    forbidden_family(ring, name),
                    Some("opentelemetry"),
                    "{ring}: {name}"
                );
            }
        }
        // The namesake is let through only where the ring says so.
        assert_eq!(
            forbidden_family(Ring::Domain, "opentelemetry"),
            Some("opentelemetry")
        );
        assert_eq!(
            Ring::Application.forbidden_rule(),
            "Application may not use tokio, reqwest, rmcp, tonic, axum, hyper, or any member of \
             the opentelemetry family but opentelemetry itself"
        );
        assert_eq!(
            Ring::SecondaryAdapter.forbidden_rule(),
            "Secondary Adapter may not use any member of the opentelemetry family but \
             opentelemetry itself"
        );
    }

    #[test]
    fn serde_is_allowed_everywhere_and_only_the_cli_root_and_test_support_forbid_nothing() {
        for ring in ALL_RINGS {
            assert_eq!(forbidden_family(ring, "serde"), None, "{ring}");
            assert_eq!(forbidden_family(ring, "serde_json"), None, "{ring}");
            let held = matches!(
                ring,
                Ring::Domain
                    | Ring::Application
                    | Ring::SecondaryAdapter
                    | Ring::RootKernel
                    | Ring::LibraryRoot
            );
            assert_eq!(ring.forbidden_families().is_empty(), !held, "{ring}");
        }
    }

    #[test]
    fn an_adapter_may_use_the_otel_api_and_tokio_but_not_the_sdk() {
        let adapter = |dependencies: &[&str]| {
            let workspace = base()
                .member(
                    FAKE,
                    "lablet-provider-fake",
                    &inherit("dependencies", dependencies),
                )
                .load();
            lint(&workspace)
        };
        assert_findings(&adapter(&["lablet-run", "tokio", "tracing", "otel"]), &[]);
        assert_findings(
            &adapter(&["otel-sdk"]),
            &[&format!(
                "lablet-provider-fake ({FAKE}, Secondary Adapter): [dependencies] depends on \
                 opentelemetry_sdk (declared as `otel-sdk`), of the `opentelemetry` family. \
                 Rule: Secondary Adapter may not use any member of the opentelemetry family but \
                 opentelemetry itself"
            )],
        );
        // In tests alone, an adapter may read what it emits from the SDK's
        // in-memory exporters.
        assert_findings(&adapter(&[]), &[]);
        let workspace = base()
            .member(
                FAKE,
                "lablet-provider-fake",
                &inherit("dev-dependencies", &["otel-sdk"]),
            )
            .load();
        assert_findings(&lint(&workspace), &[]);
    }

    /// What every fixture workspace offers for inheritance. An internal entry
    /// is read only when a member inherits it.
    const ENTRIES: &str = r#"
tokio = "=1.0.0"
tracing = "=0.1.0"
tonic-build = "=0.14.0"
otel = { package = "opentelemetry", version = "=0.30.0" }
otel-sdk = { package = "opentelemetry_sdk", version = "=0.30.0" }
opentelemetry = { package = "opentelemetry_sdk", version = "=0.30.0" }
lablet-model = { path = "crates/domain/model", version = "0.1.0" }
lablet-run = { path = "crates/application/run", version = "0.1.0" }
lablet-provider-fake = { path = "crates/adapters/secondary/provider-fake", version = "0.1.0" }
lablet-documents = { path = "crates/adapters/secondary/shared/documents", version = "0.1.0" }
lablet-tools-mcp = { path = "crates/adapters/secondary/tools-mcp", version = "0.1.0" }
lablet-http-util = { path = "crates/adapters/secondary/shared/http-util", version = "0.1.0" }
lablet-config = { path = "apps/shared/config", version = "0.1.0" }
lablet = { path = "apps/lablet", version = "0.1.0" }
lablet-cli = { path = "apps/lablet-cli", version = "0.1.0" }
lablet-conformance = { path = "tests/conformance", version = "0.1.0" }
lablet-test-mcp-server = { path = "tests/mcp-server", version = "0.1.0" }
"#;

    fn inherit(table: &str, keys: &[&str]) -> String {
        let entries: Vec<String> = keys
            .iter()
            .map(|key| format!("{key}.workspace = true\n"))
            .collect();
        format!("[{table}]\n{}\n", entries.concat())
    }

    fn base() -> FixtureWorkspace {
        FixtureWorkspace::new(ENTRIES)
            .member(MODEL, "lablet-model", "")
            .member(
                RUN,
                "lablet-run",
                &inherit("dependencies", &["lablet-model"]),
            )
    }

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
            .member(LIBRARY, "lablet", &inherit("dependencies", &app))
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
    fn an_application_crate_may_use_tracing_and_the_otel_api_and_none_of_the_other_families() {
        let workspace = base()
            .member(
                "crates/application/x",
                "lablet-x",
                &inherit("dependencies", &["tracing", "otel", "otel-sdk", "tokio"]),
            )
            .load();
        assert_findings(
            &lint(&workspace),
            &[
                "lablet-x (crates/application/x, Application): [dependencies] depends on \
                 opentelemetry_sdk (declared as `otel-sdk`), of the `opentelemetry` family. \
                 Rule: Application may not use tokio, reqwest, rmcp, tonic, axum, hyper, or any \
                 member of the opentelemetry family but opentelemetry itself",
                "lablet-x (crates/application/x, Application): [dependencies] depends on tokio, \
                 of the `tokio` family. Rule: Application may not use tokio, reqwest, rmcp, \
                 tonic, axum, hyper, or any member of the opentelemetry family but \
                 opentelemetry itself",
            ],
        );
    }

    #[test]
    fn the_application_exception_is_on_the_real_name_and_not_the_manifest_key() {
        // The key is the family's own name; the package behind it is the SDK.
        let workspace = base()
            .member(
                "crates/application/x",
                "lablet-x",
                &inherit("dependencies", &["opentelemetry"]),
            )
            .load();
        assert_findings(
            &lint(&workspace),
            &[
                "lablet-x (crates/application/x, Application): [dependencies] depends on \
               opentelemetry_sdk (declared as `opentelemetry`), of the `opentelemetry` family",
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
        let adapters = |mcp: &[&str], documents: &[&str]| {
            let documents = inherit("dependencies", documents);
            let workspace = base()
                .member(FAKE, "lablet-provider-fake", "")
                .member(http_util, "lablet-http-util", "")
                .member(DOCUMENTS, "lablet-documents", &documents)
                .member(MCP, "lablet-tools-mcp", &inherit("dependencies", mcp))
                .load();
            lint(&workspace)
        };
        // Deliberate (spec §2): adapters and kernels alike reach "the shared
        // kernels of their own ring". No kernel can reach an adapter, so the
        // worst case is a chain of kernels.
        assert_findings(
            &adapters(&["lablet-documents", "lablet-run"], &["lablet-http-util"]),
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
                    "lablet (apps/lablet, Library Root): [dependencies] depends on lablet-conformance (tests/conformance, Test Support). Rule: {rule}"
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
