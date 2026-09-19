//! Manifest rules (contributing "Architecture rules", "Code conventions" and
//! "Versioning", spec §8).
//!
//! - Every member opts into `[workspace.lints]` with `[lints] workspace = true`.
//!   A crate that omits the table still compiles clean under a gate that lints
//!   every other crate, so the omission would stay invisible.
//! - Every member inherits `version`, `edition`, `rust-version`, and `license`
//!   from `[workspace.package]`: one workspace version, one MSRV, one licence.
//! - Every member's package is named after its directory, and its integration
//!   tests are the one target `tests/it/main.rs`.
//! - Every third-party dependency of a member is `workspace = true`, and every
//!   third-party entry of `[workspace.dependencies]` is an exact `=x.y.z` pin
//!   from crates.io, so a version lives in one place and moves only in a
//!   dedicated commit. A `path` entry is internal only when the path is a
//!   listed member's directory; anywhere else it is held to the same rules.
//! - Nothing replaces a pinned crate behind the pin: no `[patch]` or
//!   `[replace]` table in a manifest, no `patch`, `paths`, or `source` in the
//!   repository's cargo configuration.
//! - No crate depends directly on `anyhow` or a mocking framework.
//! - Every dependency carries a comment saying why it is there: on the line
//!   above it, above the run of dependency lines it belongs to (a group
//!   header), or at the end of one of its own lines.
//! - `xtask/` is its own workspace and cannot inherit, so its `[lints]` must
//!   equal `[workspace.lints]` with the two print lints allowed, its
//!   dependencies follow the same pin and comment rules, and a crate it shares
//!   with `[workspace.dependencies]` is pinned to the same version there.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use crate::workspace::{DependencySpec, Manifest, Member, Workspace, normalise};

/// The rule a finding breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// `[lints] workspace = true` is missing.
    LintsInherited,
    /// A `[package]` key is not inherited from `[workspace.package]`.
    PackageInherited,
    /// A package is not named after its directory.
    PackageName,
    /// A crate has an integration test target other than `tests/it/main.rs`.
    IntegrationTarget,
    /// A third-party dependency of a member carries its own version or source.
    DependencyInherited,
    /// A third-party dependency is not an exact pin from crates.io.
    ExactPin,
    /// A table or setting that swaps a pinned crate for other code.
    SourceOverride,
    /// A direct dependency contributing/README.md rules out by name.
    BannedDependency,
    /// A dependency has no comment saying why it is there.
    DependencyComment,
    /// xtask's copy of the lint set has drifted from the workspace's.
    XtaskLints,
    /// xtask pins a crate the workspace also pins, at a different version.
    XtaskPin,
    /// A manifest could not be read or parsed.
    Unreadable,
}

impl Rule {
    /// Every rule, in reporting order.
    pub const ALL: [Self; 12] = [
        Self::Unreadable,
        Self::LintsInherited,
        Self::PackageInherited,
        Self::PackageName,
        Self::IntegrationTarget,
        Self::DependencyInherited,
        Self::ExactPin,
        Self::SourceOverride,
        Self::BannedDependency,
        Self::DependencyComment,
        Self::XtaskLints,
        Self::XtaskPin,
    ];

    /// The heading findings under this rule are reported under.
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Unreadable => "Manifests that could not be read:",
            Self::LintsInherited => "Members not inheriting [workspace.lints]:",
            Self::PackageInherited => "Package keys not inherited from [workspace.package]:",
            Self::PackageName => "Packages not named after their directory:",
            Self::IntegrationTarget => {
                "Integration tests outside the one `tests/it/main.rs` target:"
            }
            Self::DependencyInherited => {
                "Third-party dependencies not inherited from the workspace:"
            }
            Self::ExactPin => "Third-party dependencies not pinned to an exact crates.io version:",
            Self::SourceOverride => "Settings that replace a pinned crate with other code:",
            Self::BannedDependency => "Dependencies contributing/README.md rules out:",
            Self::DependencyComment => "Dependencies without a comment saying why:",
            Self::XtaskLints => "xtask's copy of the workspace lint set has drifted:",
            Self::XtaskPin => "xtask pins that differ from the workspace's pin of the same crate:",
        }
    }
}

/// One finding: the manifest, the rule, and what to change.
#[derive(Debug)]
pub struct Violation {
    /// The file at fault, relative to the repository where possible.
    pub manifest: String,
    /// The 1-based line the finding is about, when it has one.
    pub line: Option<usize>,
    /// The rule broken.
    pub rule: Rule,
    /// What is wrong and how to fix it.
    pub detail: String,
}

impl Violation {
    fn new(manifest: &str, rule: Rule, detail: String) -> Self {
        Self {
            manifest: manifest.to_owned(),
            line: None,
            rule,
            detail,
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.manifest)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        write!(f, ": {}", self.detail)
    }
}

/// The `[package]` keys every member inherits from `[workspace.package]`.
const INHERITED_PACKAGE_KEYS: [&str; 4] = ["version", "edition", "rust-version", "license"];

/// The lints xtask sets to `allow` where the workspace warns: printing is
/// xtask's job.
const XTASK_ALLOWED_LINTS: [&str; 2] = ["print_stdout", "print_stderr"];

/// The members whose package is not `lablet-<directory name>` (spec §2).
const PACKAGE_NAME_EXCEPTIONS: [(&str, &str); 3] = [
    ("apps/lablet", "lablet"),
    ("tests/conformance", "lablet-conformance"),
    ("tests/mcp-server", "lablet-test-mcp-server"),
];

/// Crates no manifest may depend on directly, with the rule each breaks
/// (contributing "Code conventions"). Direct dependencies only: what an
/// upstream crate uses inside is its own business (`prost-derive` uses
/// `anyhow`). HTTP fakes such as `wiremock` are servers, not mocking
/// frameworks.
const BANNED_DEPENDENCIES: [(&[&str], &str); 2] = [
    (
        &["anyhow", "eyre", "color-eyre"],
        "errors are `thiserror` enums, one per port or boundary; no `anyhow`",
    ),
    (
        &[
            "mockall",
            "mockall_double",
            "mockiato",
            "mocktopus",
            "faux",
            "unimock",
            "mry",
            "double",
        ],
        "test doubles are hand-written fakes in the consuming crate; no mocking framework",
    ),
];

/// Every finding over the workspace's manifests, xtask's, and the cargo
/// configuration files of the repository at `repo`.
pub fn lint(workspace: &Workspace, repo: &Path) -> Vec<Violation> {
    let prefix = workspace
        .root
        .file_name()
        .map(|name| format!("{}/", name.to_string_lossy()))
        .unwrap_or_default();
    let mut violations =
        check_workspace_dependencies(&format!("{prefix}Cargo.toml"), &workspace.text, workspace);
    for member in &workspace.members {
        let directory = if member.path.is_empty() {
            prefix.clone()
        } else {
            format!("{prefix}{}/", member.path)
        };
        let label = format!("{directory}Cargo.toml");
        violations.extend(check_member(
            &label,
            &member.text,
            &member.manifest,
            &|path| workspace.member_at(&member.path, path).is_some(),
        ));
        violations.extend(check_layout(&label, &workspace.root, member));
    }
    let label = "xtask/Cargo.toml";
    let xtask_manifest = repo.join("xtask").join("Cargo.toml");
    match std::fs::read_to_string(&xtask_manifest) {
        Ok(text) => violations.extend(check_xtask(label, &text, &workspace.document)),
        Err(e) => violations.push(Violation::new(
            label,
            Rule::Unreadable,
            format!("could not read {}: {e}", xtask_manifest.display()),
        )),
    }
    // Cargo reads a `.cargo/config.toml` from the directory it starts in and
    // every one above it; these two are the repository's.
    for (label, path) in [
        (
            ".cargo/config.toml".to_owned(),
            repo.join(".cargo/config.toml"),
        ),
        (
            format!("{prefix}.cargo/config.toml"),
            workspace.root.join(".cargo/config.toml"),
        ),
    ] {
        if let Ok(text) = std::fs::read_to_string(&path) {
            violations.extend(check_cargo_config(&label, &text));
        }
    }
    violations
}

/// The member rules: lint and package inheritance, inherited third-party
/// dependencies, and a comment on every dependency. `is_member` says whether
/// a dependency path, as this manifest writes it, is a workspace member's
/// directory.
fn check_member(
    label: &str,
    text: &str,
    manifest: &Manifest,
    is_member: &dyn Fn(&str) -> bool,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut report = |rule, detail: String| violations.push(Violation::new(label, rule, detail));

    if !manifest.lints.as_ref().is_some_and(|lints| lints.workspace) {
        report(
            Rule::LintsInherited,
            "no `[lints]` table with `workspace = true`; add one so the crate inherits \
             [workspace.lints]"
                .to_owned(),
        );
    }

    if let Some(package) = &manifest.package {
        let values = [
            &package.version,
            &package.edition,
            &package.rust_version,
            &package.license,
        ];
        for (key, value) in INHERITED_PACKAGE_KEYS.into_iter().zip(values) {
            if !value.as_ref().is_some_and(inherits_workspace) {
                report(
                    Rule::PackageInherited,
                    format!(
                        "[package] `{key}` is not inherited; write `{key}.workspace = true` so \
                         the value comes from [workspace.package]"
                    ),
                );
            }
        }
    }

    for section in manifest.dependency_sections() {
        let header = &section.header;
        for (key, spec) in section.entries {
            if let Some(rule) = banned(key, spec) {
                report(Rule::BannedDependency, format!("{header} `{key}`: {rule}"));
            }
            if spec.inherits_workspace() {
                continue;
            }
            match spec.path() {
                Some(path) if is_member(path) => {}
                Some(path) => report(
                    Rule::DependencyInherited,
                    format!(
                        "{header} `{key}` is a path dependency on `{path}`, which is not a \
                         workspace member's directory; an internal crate is listed in \
                         [workspace] members, and anything else is pinned in \
                         [workspace.dependencies] and written `{key}.workspace = true`"
                    ),
                ),
                None => report(
                    Rule::DependencyInherited,
                    format!(
                        "{header} `{key}` carries its own version or source; pin it in \
                         [workspace.dependencies] and write `{key}.workspace = true`"
                    ),
                ),
            }
        }
    }

    // A root manifest that is also a package holds [workspace.dependencies],
    // which `check_workspace_dependencies` reads.
    let lines: Vec<DependencyLine> = dependency_lines(text)
        .into_iter()
        .filter(|line| line.path.first().is_none_or(|scope| scope != "workspace"))
        .collect();
    let declared = manifest.dependency_sections();
    let declared = declared.iter().flat_map(|section| {
        section.entries.keys().map(move |key| {
            (
                section.path.as_slice(),
                section.header.as_str(),
                key.as_str(),
            )
        })
    });
    violations.extend(comment_findings(label, &lines, declared));
    violations
}

/// The rules about where a member's files are: the package is named after its
/// directory, and its integration tests are the one target `tests/it/main.rs`.
fn check_layout(label: &str, workspace_root: &Path, member: &Member) -> Vec<Violation> {
    let mut violations = Vec::new();
    if let Some(expected) = expected_package_name(&member.path)
        && expected != member.name
    {
        violations.push(Violation::new(
            label,
            Rule::PackageName,
            format!(
                "[package] `name` is \"{}\", but the crate in `{}` is package \"{expected}\": \
                 directory `foo/bar/` is package `lablet-bar`, apart from the three exceptions \
                 in spec §2",
                member.name, member.path
            ),
        ));
    }

    let mut report = |detail: String| {
        violations.push(Violation::new(label, Rule::IntegrationTarget, detail));
    };
    if !member.manifest.test_targets.is_empty() {
        report(
            "a `[[test]]` table declares an integration target by hand; the one target is \
             `tests/it/main.rs`, which cargo finds by itself"
                .to_owned(),
        );
    }
    let tests = workspace_root.join(&member.path).join("tests");
    let mut entries: Vec<String> = std::fs::read_dir(&tests)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    entries.sort();
    for name in &entries {
        if name != "it" {
            report(format!(
                "`tests/{name}` is outside the one integration target; move it under \
                 `tests/it/` and declare it as a module in `tests/it/main.rs`"
            ));
        }
    }
    if entries.iter().any(|name| name == "it") && !tests.join("it").join("main.rs").is_file() {
        report("`tests/it/` has no `main.rs`, so cargo builds no target from it".to_owned());
    }
    violations
}

/// The package name a member's directory calls for; `None` for a package in
/// the workspace root, which `lint-layers` reports as in no ring.
fn expected_package_name(member_path: &str) -> Option<String> {
    if let Some((_, name)) = PACKAGE_NAME_EXCEPTIONS
        .iter()
        .find(|(path, _)| *path == member_path)
    {
        return Some((*name).to_owned());
    }
    member_path
        .rsplit('/')
        .next()
        .filter(|directory| !directory.is_empty())
        .map(|directory| format!("lablet-{directory}"))
}

/// The rule a direct dependency on this crate breaks, if it is ruled out. The
/// crate is the entry's `package`, or its key.
fn banned(key: &str, spec: &DependencySpec) -> Option<&'static str> {
    let name = normalise(spec.package().unwrap_or(key));
    BANNED_DEPENDENCIES
        .iter()
        .find(|(crates, _)| crates.iter().any(|banned| normalise(banned) == name))
        .map(|(_, rule)| *rule)
}

/// The `[workspace.dependencies]` rules: every third-party entry is an exact
/// pin with a comment, and the root manifest replaces no crate. Workspace
/// crates are exempt from the pin and the comment: the entries whose path is
/// a listed member's directory.
fn check_workspace_dependencies(label: &str, text: &str, workspace: &Workspace) -> Vec<Violation> {
    let third_party: BTreeMap<&String, &DependencySpec> = workspace
        .table
        .dependencies
        .iter()
        .filter(|(_, spec)| {
            spec.path()
                .is_none_or(|path| workspace.member_at("", path).is_none())
        })
        .collect();
    let table = "[workspace.dependencies]";
    let mut violations = check_source_overrides(label, &workspace.document);
    violations.extend(check_pins(label, table, &third_party));
    for (key, spec) in &workspace.table.dependencies {
        if let Some(rule) = banned(key, spec) {
            violations.push(Violation::new(
                label,
                Rule::BannedDependency,
                format!("{table} `{key}`: {rule}"),
            ));
        }
    }
    let path = ["workspace".to_owned(), "dependencies".to_owned()];
    let lines: Vec<DependencyLine> = dependency_lines(text)
        .into_iter()
        .filter(|line| line.path == path && third_party.contains_key(&line.key))
        .collect();
    let declared = third_party
        .keys()
        .map(|key| (path.as_slice(), table, key.as_str()));
    violations.extend(comment_findings(label, &lines, declared));
    violations
}

/// The xtask rules: the lint copy, exact pins, a comment on every
/// dependency, and no replaced crate. xtask has no workspace crates, so every
/// entry is third-party, a `path` one included.
fn check_xtask(label: &str, text: &str, workspace_document: &toml::Value) -> Vec<Violation> {
    let violation = |rule, detail| Violation::new(label, rule, detail);
    let (document, manifest) = match (toml::from_str::<toml::Value>(text), Manifest::parse(text)) {
        (Ok(document), Ok(manifest)) => (document, manifest),
        (Err(e), _) => return vec![violation(Rule::Unreadable, format!("could not parse: {e}"))],
        (_, Err(e)) => return vec![violation(Rule::Unreadable, format!("could not parse: {e}"))],
    };
    let mut violations = check_source_overrides(label, &document);
    if let Err(detail) = lint_copy_matches(workspace_document, &document) {
        violations.push(violation(Rule::XtaskLints, detail));
    }
    let sections = manifest.dependency_sections();
    for section in &sections {
        let entries: BTreeMap<&String, &DependencySpec> = section.entries.iter().collect();
        violations.extend(check_pins(label, &section.header, &entries));
        for (key, spec) in entries {
            if let Some(rule) = banned(key, spec) {
                violations.push(violation(
                    Rule::BannedDependency,
                    format!("{} `{key}`: {rule}", section.header),
                ));
            }
            let workspace_pin = table_at(workspace_document, &["workspace", "dependencies", key])
                .and_then(|entry| pinned_version(&entry));
            if let (Some(ours), Some(theirs)) = (spec.version(), workspace_pin)
                && ours != theirs
            {
                violations.push(violation(
                    Rule::XtaskPin,
                    format!(
                        "{} `{key}` is pinned to \"{ours}\", but [workspace.dependencies] pins \
                         it to \"{theirs}\"; bump both in the same dedicated commit, so the \
                         repository reviews one version of a crate both pin",
                        section.header
                    ),
                ));
            }
        }
    }
    let declared = sections.iter().flat_map(|section| {
        section.entries.keys().map(move |key| {
            (
                section.path.as_slice(),
                section.header.as_str(),
                key.as_str(),
            )
        })
    });
    violations.extend(comment_findings(label, &dependency_lines(text), declared));
    violations
}

/// A finding for each table of a root manifest that swaps a crate for other
/// code. `[patch]` and `[replace]` leave the pin in `[workspace.dependencies]`
/// reading as it did while the build uses something else, and cargo-deny's
/// source check does not look at path sources. Cargo reads both tables from a
/// workspace root only, which is where this looks.
fn check_source_overrides(label: &str, document: &toml::Value) -> Vec<Violation> {
    ["patch", "replace"]
        .into_iter()
        .filter(|table| document.get(table).is_some())
        .map(|table| {
            Violation::new(
                label,
                Rule::SourceOverride,
                format!(
                    "`[{table}]` replaces a crate behind its exact pin; remove it, and if a \
                     dependency really must be patched, make that its own reviewed decision"
                ),
            )
        })
        .collect()
}

/// The same for a cargo configuration file, where `patch`, `paths`, and
/// `source` replacement do what `[patch]` does in a manifest.
fn check_cargo_config(label: &str, text: &str) -> Vec<Violation> {
    let document: toml::Value = match toml::from_str(text) {
        Ok(document) => document,
        Err(e) => {
            return vec![Violation::new(
                label,
                Rule::Unreadable,
                format!("could not parse: {e}"),
            )];
        }
    };
    ["patch", "paths", "source"]
        .into_iter()
        .filter(|key| document.get(key).is_some())
        .map(|key| {
            Violation::new(
                label,
                Rule::SourceOverride,
                format!(
                    "`{key}` replaces a crate behind its exact pin, as `[patch]` in a manifest \
                     would; remove it"
                ),
            )
        })
        .collect()
}

/// The version of a raw dependency entry: the string itself, or its `version`.
fn pinned_version(entry: &toml::Value) -> Option<String> {
    entry
        .as_str()
        .or_else(|| entry.get("version").and_then(toml::Value::as_str))
        .map(str::to_owned)
}

/// A finding for each entry of `table` that is not an exact pin from
/// crates.io. The source is judged before the version: a git or
/// alternative-registry entry with `version = "=x.y.z"` beside it is still
/// not the crates.io artefact the pin names.
fn check_pins(
    label: &str,
    table: &str,
    third_party: &BTreeMap<&String, &DependencySpec>,
) -> Vec<Violation> {
    third_party
        .iter()
        .filter_map(|(key, spec)| {
            let found = if spec.is_git() {
                "is a git dependency".to_owned()
            } else if spec.is_alternative_registry() {
                "comes from a registry other than crates.io".to_owned()
            } else if let Some(path) = spec.path() {
                format!(
                    "is a path dependency on `{path}`, which is not a workspace member's directory"
                )
            } else {
                match spec.version() {
                    Some(version) if is_exact_pin(version) => return None,
                    Some(version) => format!("has the requirement \"{version}\""),
                    None => "has no version".to_owned(),
                }
            };
            Some(Violation::new(
                label,
                Rule::ExactPin,
                format!(
                    "{table} `{key}` {found}; third-party crates come from crates.io, pinned to \
                     an exact version, written \"=x.y.z\""
                ),
            ))
        })
        .collect()
}

/// Whether a raw `[package]` value is `{ workspace = true }`.
fn inherits_workspace(value: &toml::Value) -> bool {
    value.get("workspace").and_then(toml::Value::as_bool) == Some(true)
}

/// Whether a version requirement is `=x.y.z`, with an optional pre-release or
/// build suffix, and nothing else.
fn is_exact_pin(requirement: &str) -> bool {
    let Some(version) = requirement.strip_prefix('=') else {
        return false;
    };
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && !version.contains([',', ' ', '*'])
}

/// The comment findings of one manifest: each dependency line no comment
/// covers, then each dependency the parser found (`declared`: table path,
/// header, key) that the text scan found no line for. The scan reads keys
/// under a `[...dependencies]` header and dotted keys that reach into one; a
/// dependency written any other way, such as an inline `dependencies = { .. }`
/// table, has no line a comment could be checked on, and must not pass unread.
fn comment_findings<'a>(
    label: &str,
    lines: &[DependencyLine],
    declared: impl Iterator<Item = (&'a [String], &'a str, &'a str)>,
) -> Vec<Violation> {
    let mut violations: Vec<Violation> = lines
        .iter()
        .filter(|line| !line.commented)
        .map(|line| Violation {
            line: Some(line.line),
            ..Violation::new(
                label,
                Rule::DependencyComment,
                format!(
                    "{} `{}` has no comment; say why the dependency is there on the line above \
                     it, under a group header comment, or at the end of its line",
                    line.table, line.key
                ),
            )
        })
        .collect();
    for (path, header, key) in declared {
        if !lines
            .iter()
            .any(|line| line.path == path && line.key == key)
        {
            violations.push(Violation::new(
                label,
                Rule::DependencyComment,
                format!(
                    "{header} `{key}`: could not find this dependency's line to check its \
                     comment; write it as a key under a `{header}` header"
                ),
            ));
        }
    }
    violations
}

// --- Reading comments, which a TOML parser drops ---

/// One dependency as it is written in a manifest.
#[derive(Debug, PartialEq, Eq)]
struct DependencyLine {
    /// The header of the table the dependency is in, as written where the
    /// header names the table itself.
    table: String,
    /// The key path of that table: `["target", "cfg(unix)", "dependencies"]`.
    path: Vec<String>,
    /// The dependency key.
    key: String,
    /// The 1-based line the dependency starts on.
    line: usize,
    /// Whether a comment covers it: one earlier in the same run of non-blank
    /// lines of its table, or one at the end of a line of its own.
    commented: bool,
}

/// The names of cargo's dependency tables.
const DEPENDENCY_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// Where a key path reaches a dependency table: the length of the table's own
/// path, and the dependency key after it when the path goes that far. A
/// dependency table is `dependencies` (or its dev and build forms) at the top
/// level, under `workspace`, or under `target.<spec>`.
fn dependency_at(path: &[String]) -> Option<(usize, Option<&String>)> {
    let is_table = |name: &String| DEPENDENCY_TABLES.contains(&name.as_str());
    let table = match path {
        [table, ..] if is_table(table) => 0,
        [scope, table, ..] if scope == "workspace" && is_table(table) => 1,
        [scope, _, table, ..] if scope == "target" && is_table(table) => 2,
        _ => return None,
    };
    Some((table + 1, path.get(table + 1)))
}

/// Every dependency in a manifest's text, in order, with whether a comment
/// covers it. A dependency is any key path that reaches into a dependency
/// table, whether the table is the header (`[dependencies]` then `foo = ..`),
/// the header goes further (`[dependencies.foo]`), or the key does
/// (`dependencies.foo = ..` under another header or none).
fn dependency_lines(text: &str) -> Vec<DependencyLine> {
    let mut found: Vec<DependencyLine> = Vec::new();
    let mut scanner = ValueScanner::default();
    // The header being read under: as written, and its key path.
    let mut header = String::new();
    let mut header_path: Vec<String> = Vec::new();
    // Whether a comment has been seen in the current run of non-blank lines.
    let mut covered = false;
    // The dependency whose value the scanner is inside of, as an index.
    let mut open: Option<usize> = None;

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if scanner.inside_value() {
            // A comment that closes the value is at the end of the dependency.
            let trailing_comment = scanner.scan(line);
            if !scanner.inside_value()
                && let Some(entry) = open.take().and_then(|at| found.get_mut(at))
            {
                entry.commented |= trailing_comment;
            }
        } else if trimmed.is_empty() {
            covered = false;
        } else if trimmed.starts_with('#') {
            covered = true;
        } else if trimmed.starts_with('[') {
            // `[table]` or `[[array-of-tables]]`, rebuilt without what follows it.
            let inner = trimmed.trim_start_matches('[');
            let brackets = trimmed.len() - inner.len();
            let (path, end) = key_path(inner, ']');
            header = format!(
                "{}{}{}",
                "[".repeat(brackets),
                &inner[..end],
                "]".repeat(brackets)
            );
            header_path = path;
            if let Some((table, Some(key))) = dependency_at(&header_path) {
                sight(
                    &mut found,
                    DependencyLine {
                        table: header.clone(),
                        path: header_path[..table].to_vec(),
                        key: key.clone(),
                        line: index + 1,
                        commented: covered || inner[end..].contains('#'),
                    },
                );
            }
            covered = false;
        } else {
            let trailing_comment = scanner.scan(line);
            let (key, _) = key_path(trimmed, '=');
            let mut path = header_path.clone();
            path.extend(key);
            let Some((table, Some(key))) = dependency_at(&path) else {
                continue;
            };
            let written = if header_path.len() == table {
                header.clone()
            } else {
                format!("[{}]", path[..table].join("."))
            };
            let at = sight(
                &mut found,
                DependencyLine {
                    table: written,
                    path: path[..table].to_vec(),
                    key: key.clone(),
                    line: index + 1,
                    commented: covered,
                },
            );
            if let Some(entry) = found.get_mut(at) {
                entry.commented |= trailing_comment;
            }
            open = scanner.inside_value().then_some(at);
        }
    }
    found
}

/// Records a sighting of a dependency, once per table and key, so
/// `foo.version` and `foo.features` count once wherever they stand, and
/// returns its index in `found`.
fn sight(found: &mut Vec<DependencyLine>, entry: DependencyLine) -> usize {
    let seen = found
        .iter()
        .position(|line| line.path == entry.path && line.key == entry.key);
    seen.unwrap_or_else(|| {
        found.push(entry);
        found.len() - 1
    })
}

/// Splits a dotted TOML key path, quotes honoured, up to `terminator`. Returns
/// the segments and the byte offset the path ended at.
fn key_path(text: &str, terminator: char) -> (Vec<String>, usize) {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut end = text.len();
    for (offset, c) in text.char_indices() {
        match quote {
            Some(open) if c == open => quote = None,
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '.' => segments.push(std::mem::take(&mut current)),
            None if c == terminator => {
                end = offset;
                break;
            }
            None if c.is_whitespace() => {}
            // Inside quotes everything is literal; outside, the rest is a bare key.
            Some(_) | None => current.push(c),
        }
    }
    segments.push(current);
    (segments, end)
}

/// Tracks whether the text so far has left a value open across lines: an
/// array or inline table not yet closed, or a multi-line string. Lines inside
/// one are neither comments nor dependency keys.
#[derive(Default)]
struct ValueScanner {
    depth: usize,
    multiline: Option<&'static str>,
}

impl ValueScanner {
    fn inside_value(&self) -> bool {
        self.depth > 0 || self.multiline.is_some()
    }

    /// Reads one line; returns whether it ends in a comment.
    fn scan(&mut self, line: &str) -> bool {
        let bytes = line.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            if let Some(delimiter) = self.multiline {
                let Some(after) = multiline_close(bytes, at, delimiter) else {
                    return false;
                };
                at = after;
                self.multiline = None;
                continue;
            }
            match bytes[at] {
                b'#' => return true,
                quote @ (b'"' | b'\'') => {
                    let triple = if quote == b'"' { "\"\"\"" } else { "'''" };
                    if bytes[at..].starts_with(triple.as_bytes()) {
                        self.multiline = Some(triple);
                        at += triple.len();
                        continue;
                    }
                    at += 1;
                    while at < bytes.len() && bytes[at] != quote {
                        // A basic string escapes its quote; a literal one cannot.
                        if quote == b'"' && bytes[at] == b'\\' {
                            at += 1;
                        }
                        at += 1;
                    }
                    at += 1;
                }
                b'[' | b'{' => {
                    self.depth += 1;
                    at += 1;
                }
                b']' | b'}' => {
                    self.depth = self.depth.saturating_sub(1);
                    at += 1;
                }
                _ => at += 1,
            }
        }
        false
    }
}

/// Where a multi-line string that is open at `from` closes on this line: the
/// offset just past its closing delimiter, or `None` when it stays open. In a
/// basic string (`"""`) a backslash escapes the next character, so `\"""`
/// closes nothing; a literal string (`'''`) has no escapes. TOML also lets up
/// to two quotes stand directly before the delimiter (`""""` ends a string
/// with a quote in it), and they belong to the string, not to what follows.
fn multiline_close(bytes: &[u8], from: usize, delimiter: &str) -> Option<usize> {
    let quote = delimiter.as_bytes()[0];
    let escapes = quote == b'"';
    let mut at = from;
    while at < bytes.len() {
        if escapes && bytes[at] == b'\\' {
            at += 2;
        } else if bytes[at..].starts_with(delimiter.as_bytes()) {
            at += delimiter.len();
            let extra = bytes[at..]
                .iter()
                .take(2)
                .take_while(|byte| **byte == quote)
                .count();
            return Some(at + extra);
        } else {
            at += 1;
        }
    }
    None
}

// --- The xtask lint copy ---

/// Whether the xtask manifest's `[lints]` is the workspace manifest's
/// `[workspace.lints]` with the print lints allowed; the error lists every
/// lint that differs.
fn lint_copy_matches(workspace: &toml::Value, xtask: &toml::Value) -> Result<(), String> {
    let mut expected = table_at(workspace, &["workspace", "lints"])
        .ok_or("the workspace manifest has no [workspace.lints] table to compare with")?;
    let clippy = expected
        .get_mut("clippy")
        .and_then(toml::Value::as_table_mut)
        .ok_or("the workspace manifest has no [workspace.lints.clippy] table to compare with")?;
    for lint in XTASK_ALLOWED_LINTS {
        clippy.insert(lint.to_owned(), toml::Value::String("allow".to_owned()));
    }
    let actual = table_at(xtask, &["lints"]).ok_or("no [lints] table")?;
    let drift = lint_drift(&expected, &actual);
    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "[lints] must be [workspace.lints] with print_stdout and print_stderr allowed:\n    {}",
            drift.join("\n    ")
        ))
    }
}

/// Every lint whose level differs between two lint tables, as
/// `group.lint: expected <level>, found <level>` lines.
fn lint_drift(expected: &toml::Value, actual: &toml::Value) -> Vec<String> {
    let (expected, actual) = (lint_levels(expected), lint_levels(actual));
    let show = |level: Option<&&toml::Value>| {
        level.map_or_else(|| "absent".to_owned(), ToString::to_string)
    };
    let lints: BTreeSet<&String> = expected.keys().chain(actual.keys()).collect();
    lints
        .into_iter()
        .filter(|lint| expected.get(*lint) != actual.get(*lint))
        .map(|lint| {
            format!(
                "{lint}: expected {}, found {}",
                show(expected.get(lint)),
                show(actual.get(lint))
            )
        })
        .collect()
}

/// The lints of a table flattened to `group.lint`, each with its level.
fn lint_levels(table: &toml::Value) -> BTreeMap<String, &toml::Value> {
    table
        .as_table()
        .into_iter()
        .flat_map(toml::Table::iter)
        .filter_map(|(group, lints)| lints.as_table().map(|lints| (group, lints)))
        .flat_map(|(group, lints)| {
            lints
                .iter()
                .map(move |(lint, level)| (format!("{group}.{lint}"), level))
        })
        .collect()
}

/// The table under `keys` in a parsed TOML document.
fn table_at(root: &toml::Value, keys: &[&str]) -> Option<toml::Value> {
    keys.iter()
        .try_fold(root, |value, key| value.get(*key))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::{FixtureWorkspace, TempDir};

    fn member(tables: &str) -> (String, Manifest) {
        let text = format!(
            "[package]\nname = \"lablet-x\"\nversion.workspace = true\nedition.workspace = true\n\
             rust-version.workspace = true\nlicense.workspace = true\n\n{tables}"
        );
        let manifest = Manifest::parse(&text).unwrap();
        (text, manifest)
    }

    /// `check_member` for a manifest whose path dependencies are all members.
    fn check(label: &str, text: &str, manifest: &Manifest) -> Vec<Violation> {
        check_member(label, text, manifest, &|_| true)
    }

    fn rules(violations: &[Violation]) -> Vec<Rule> {
        violations.iter().map(|v| v.rule).collect()
    }

    const LINTS: &str = "[lints]\nworkspace = true\n";

    // --- Lint inheritance ---

    #[test]
    fn a_member_that_inherits_everything_is_clean() {
        let (text, manifest) = member(&format!(
            "{LINTS}\n[dependencies]\n# Errors\nthiserror.workspace = true\n"
        ));
        let violations = check("m", &text, &manifest);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn a_member_with_no_lints_table_is_reported() {
        let (text, manifest) = member("");
        assert_eq!(rules(&check("m", &text, &manifest)), [Rule::LintsInherited]);
    }

    #[test]
    fn a_lints_table_that_opts_out_is_reported() {
        let (text, manifest) = member("[lints]\nworkspace = false\n");
        assert_eq!(rules(&check("m", &text, &manifest)), [Rule::LintsInherited]);
    }

    #[test]
    fn crate_local_lints_without_the_inheritance_flag_are_reported() {
        // The near-miss the check most needs to catch.
        let (text, manifest) = member("[lints.clippy]\nunwrap_used = \"warn\"\n");
        assert_eq!(rules(&check("m", &text, &manifest)), [Rule::LintsInherited]);
    }

    // --- Package inheritance ---

    #[test]
    fn each_package_key_written_out_instead_of_inherited_is_reported() {
        let text = format!(
            "[package]\nname = \"lablet-x\"\nversion = \"0.1.0\"\nedition.workspace = true\n\
             license = \"MIT\"\n\n{LINTS}"
        );
        let manifest = Manifest::parse(&text).unwrap();
        let violations = check("m", &text, &manifest);
        assert_eq!(
            rules(&violations),
            [
                Rule::PackageInherited,
                Rule::PackageInherited,
                Rule::PackageInherited
            ]
        );
        let details: Vec<&str> = violations.iter().map(|v| v.detail.as_str()).collect();
        assert!(details[0].contains("`version`"), "{details:?}");
        assert!(details[1].contains("`rust-version`"), "{details:?}");
        assert!(details[2].contains("`license`"), "{details:?}");
    }

    // --- Third-party dependencies are inherited ---

    #[test]
    fn an_ad_hoc_version_in_a_member_is_reported_in_every_table() {
        let (text, manifest) = member(&format!(
            "{LINTS}\n[dependencies]\n# Runtime\ntokio = \"1\"\n\n\
             [dev-dependencies]\n# HTTP fakes\nwiremock = {{ version = \"=0.6.0\" }}\n\n\
             [target.'cfg(unix)'.build-dependencies]\n# Codegen\nprost-build = {{ git = \"https://example.com/prost\" }}\n"
        ));
        let violations = check("m", &text, &manifest);
        assert_eq!(
            rules(&violations),
            [
                Rule::DependencyInherited,
                Rule::DependencyInherited,
                Rule::DependencyInherited
            ]
        );
        assert!(
            violations[0].detail.contains("[dependencies] `tokio`"),
            "{}",
            violations[0]
        );
        assert!(
            violations[2]
                .detail
                .contains("[target.'cfg(unix)'.build-dependencies] `prost-build`"),
            "{}",
            violations[2]
        );
    }

    #[test]
    fn a_path_dependency_on_a_workspace_crate_needs_no_inheritance() {
        let (text, manifest) = member(&format!(
            "{LINTS}\n[dependencies]\n# Domain\nlablet-model = {{ path = \"../../domain/model\" }}\n"
        ));
        assert!(check("m", &text, &manifest).is_empty());
    }

    #[test]
    fn a_path_dependency_that_is_not_a_member_is_held_to_the_third_party_rule() {
        let (text, manifest) = member(&format!(
            "{LINTS}\n[dev-dependencies]\n# Test helpers\nlablet-testkit = {{ path = \"../../../tests/testkit\" }}\n"
        ));
        let violations = check_member("m", &text, &manifest, &|_| false);
        assert_eq!(rules(&violations), [Rule::DependencyInherited]);
        assert!(
            violations[0].detail.contains(
                "`lablet-testkit` is a path dependency on `../../../tests/testkit`, which is \
                 not a workspace member's directory"
            ),
            "{}",
            violations[0]
        );
    }

    // --- Banned direct dependencies ---

    #[test]
    fn a_direct_dependency_on_anyhow_or_a_mocking_framework_is_reported_in_every_table() {
        let (text, manifest) = member(&format!(
            "{LINTS}\n[dependencies]\n# Quick errors\nanyhow.workspace = true\n\n\
             [dev-dependencies]\n# Mocks\nmockall.workspace = true\n\
             doubles = {{ package = \"mockall_double\", workspace = true }}\n\
             wiremock.workspace = true\n"
        ));
        let violations = check("m", &text, &manifest);
        assert_eq!(
            rules(&violations),
            [
                Rule::BannedDependency,
                Rule::BannedDependency,
                Rule::BannedDependency
            ]
        );
        assert!(
            violations[0]
                .detail
                .starts_with("[dependencies] `anyhow`: errors are `thiserror` enums"),
            "{}",
            violations[0]
        );
        assert!(
            violations[1].detail.contains("`doubles`"),
            "{}",
            violations[1]
        );
        assert!(
            violations[2].detail.contains("no mocking framework"),
            "{}",
            violations[2]
        );
    }

    #[test]
    fn a_banned_crate_in_the_workspace_table_or_in_xtask_is_reported() {
        let workspace = FixtureWorkspace::new(
            "# Errors\nerrors = { package = \"anyhow\", version = \"=1.0.100\" }\n",
        )
        .load();
        let violations = check_workspace_dependencies("w", &workspace.text, &workspace);
        assert_eq!(rules(&violations), [Rule::BannedDependency]);

        let xtask = format!(
            "[package]\nname = \"xtask\"\n\n[dependencies]\n# Errors\ncolor_eyre = \"=0.6.5\"\n{XTASK_LINTS}"
        );
        let violations = check_xtask("xtask/Cargo.toml", &xtask, &parse(WORKSPACE_LINTS));
        assert_eq!(rules(&violations), [Rule::BannedDependency]);
    }

    // --- Exact pins ---

    #[test]
    fn only_a_full_version_behind_an_equals_sign_is_an_exact_pin() {
        for pin in ["=1.2.3", "=0.1.0", "=1.0.0-rc.1", "=1.1.6+spec-1.1.0"] {
            assert!(is_exact_pin(pin), "{pin}");
        }
        for loose in [
            "1.2.3",
            "^1.2.3",
            "~1.2",
            "=1.2",
            "=1",
            "*",
            "=1.2.*",
            ">=1.2.3",
            "=1.2.3, <2",
            "= 1.2.3",
            "=1.2.x",
            "",
        ] {
            assert!(!is_exact_pin(loose), "{loose}");
        }
    }

    #[test]
    fn workspace_dependencies_must_be_exact_pins_with_comments() {
        let workspace = FixtureWorkspace::new(
            "# Async ports\nasync-trait = \"=0.1.89\"\n\n\
             # Runtime\ntokio = { version = \"1.47\", features = [\"rt\"] }\n\n\
             serde = \"=1.0.229\"\n\n\
             # Wire types\nprost = { git = \"https://example.com/prost\" }\n\n\
             lablet-model = { path = \"crates/domain/model\" }\n",
        )
        .member("crates/domain/model", "lablet-model", "")
        .load();
        let violations = check_workspace_dependencies("w", &workspace.text, &workspace);
        assert_eq!(
            rules(&violations),
            [Rule::ExactPin, Rule::ExactPin, Rule::DependencyComment]
        );
        assert!(
            violations[0].detail.contains("`prost` is a git dependency"),
            "{}",
            violations[0]
        );
        assert!(
            violations[1]
                .detail
                .contains("`tokio` has the requirement \"1.47\""),
            "{}",
            violations[1]
        );
        assert!(violations[2].detail.contains("`serde` has no comment"));
    }

    #[test]
    fn a_version_beside_another_source_does_not_make_an_exact_pin() {
        let workspace = FixtureWorkspace::new(
            "# Wire types\nprost = { git = \"https://example.com/prost\", branch = \"main\", version = \"=0.14.4\" }\n\
             # Diagnostics\ntracing = { version = \"=0.1.44\", registry = \"corp\" }\n\
             # Runtime\ntokio = { version = \"=1.0.0\", registry-index = \"https://example.com/index\" }\n\
             # A vendored copy\nvendored = { path = \"../vendor/thing\", version = \"=1.0.0\" }\n\
             # Domain, repointed\nlablet-model = { path = \"../outside/model\", version = \"=0.1.0\" }\n",
        )
        .member("crates/domain/model", "lablet-model", "")
        .load();
        let violations = check_workspace_dependencies("w", &workspace.text, &workspace);
        assert!(
            violations.iter().all(|v| v.rule == Rule::ExactPin),
            "{violations:#?}"
        );
        let details: Vec<&str> = violations.iter().map(|v| v.detail.as_str()).collect();
        assert_eq!(details.len(), 5, "{details:#?}");
        assert!(details[0].contains("`lablet-model` is a path dependency on `../outside/model`"));
        assert!(details[1].contains("`prost` is a git dependency"));
        assert!(details[2].contains("`tokio` comes from a registry other than crates.io"));
        assert!(details[3].contains("`tracing` comes from a registry other than crates.io"));
        assert!(details[4].contains("`vendored` is a path dependency"));
    }

    // --- Replaced crates ---

    #[test]
    fn a_patch_or_replace_table_in_a_root_manifest_is_reported() {
        for (table, body) in [
            (
                "patch",
                "[patch.crates-io]\nthiserror = { path = \"../vendor/thiserror\" }\n",
            ),
            (
                "replace",
                "[replace]\n\"serde:1.0.229\" = { path = \"../vendor/serde\" }\n",
            ),
        ] {
            let dir = TempDir::new("override");
            dir.write(
                "Cargo.toml",
                &format!("[workspace]\nmembers = []\n\n{body}"),
            );
            let workspace = Workspace::load(dir.path()).unwrap();
            let violations = check_workspace_dependencies("w", &workspace.text, &workspace);
            assert_eq!(rules(&violations), [Rule::SourceOverride], "{table}");
            assert!(
                violations[0]
                    .detail
                    .starts_with(&format!("`[{table}]` replaces"))
            );

            let xtask = format!("[package]\nname = \"xtask\"\n{XTASK_LINTS}\n{body}");
            let violations = check_xtask("xtask/Cargo.toml", &xtask, &parse(WORKSPACE_LINTS));
            assert_eq!(rules(&violations), [Rule::SourceOverride], "{table}");
        }
    }

    #[test]
    fn a_cargo_config_that_replaces_a_crate_is_reported() {
        let alias_only = "[alias]\nxtask = \"run -q --\"\n";
        assert!(check_cargo_config("c", alias_only).is_empty());
        for config in [
            "[patch.crates-io]\nthiserror = { path = \"../vendor/thiserror\" }\n",
            "paths = [\"../vendor/thiserror\"]\n",
            "[source.crates-io]\nreplace-with = \"mirror\"\n",
        ] {
            let violations = check_cargo_config("c", config);
            assert_eq!(rules(&violations), [Rule::SourceOverride], "{config}");
        }
        assert_eq!(
            rules(&check_cargo_config("c", "[alias")),
            [Rule::Unreadable]
        );
    }

    // --- Where a member's files are ---

    #[test]
    fn a_package_is_named_after_its_directory_with_three_exceptions() {
        for (path, name) in [
            ("crates/domain/model", "lablet-model"),
            (
                "crates/adapters/secondary/shared/telemetry-registry",
                "lablet-telemetry-registry",
            ),
            ("apps/lablet", "lablet"),
            ("tests/conformance", "lablet-conformance"),
            ("tests/mcp-server", "lablet-test-mcp-server"),
        ] {
            assert_eq!(expected_package_name(path).as_deref(), Some(name));
        }
        assert_eq!(expected_package_name(""), None);

        let workspace = FixtureWorkspace::new("")
            .member(
                "crates/adapters/secondary/tools-builtin",
                "builtin_tools",
                "",
            )
            .load();
        let violations = check_layout("m", &workspace.root, &workspace.members[0]);
        assert_eq!(rules(&violations), [Rule::PackageName]);
        assert!(
            violations[0]
                .detail
                .contains("`name` is \"builtin_tools\", but the crate in `crates/adapters/secondary/tools-builtin` is package \"lablet-tools-builtin\""),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn integration_tests_are_the_one_target_tests_it_main_rs() {
        let layout = |files: &[&str], tables: &str| {
            let dir = TempDir::new("layout");
            dir.write(
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/domain/model\"]\n",
            );
            dir.write(
                "crates/domain/model/Cargo.toml",
                &format!("[package]\nname = \"lablet-model\"\n{tables}"),
            );
            for file in files {
                dir.write(&format!("crates/domain/model/{file}"), "");
            }
            let workspace = Workspace::load(dir.path()).unwrap();
            check_layout("m", &workspace.root, &workspace.members[0])
        };
        assert!(layout(&["src/lib.rs"], "").is_empty());
        assert!(
            layout(
                &["tests/it/main.rs", "tests/it/stop.rs", "tests/.DS_Store"],
                ""
            )
            .is_empty()
        );

        let violations = layout(
            &["tests/it/main.rs", "tests/foo.rs", "tests/bar/main.rs"],
            "",
        );
        assert_eq!(
            rules(&violations),
            [Rule::IntegrationTarget, Rule::IntegrationTarget]
        );
        assert!(
            violations[0].detail.starts_with("`tests/bar` is outside"),
            "{}",
            violations[0]
        );
        assert!(
            violations[1]
                .detail
                .starts_with("`tests/foo.rs` is outside"),
            "{}",
            violations[1]
        );

        let violations = layout(&["tests/it/stop.rs"], "");
        assert_eq!(rules(&violations), [Rule::IntegrationTarget]);
        assert!(
            violations[0].detail.contains("has no `main.rs`"),
            "{}",
            violations[0]
        );

        let violations = layout(
            &["other/x.rs"],
            "\n[[test]]\nname = \"x\"\npath = \"other/x.rs\"\n",
        );
        assert_eq!(rules(&violations), [Rule::IntegrationTarget]);
        assert!(
            violations[0].detail.contains("`[[test]]`"),
            "{}",
            violations[0]
        );
    }

    /// The MSRV for a pinned toolchain `major.minor[.patch]`: two minor
    /// versions below it (contributing "Versioning", spec §8).
    fn msrv_for(channel: &str) -> Option<String> {
        let mut parts = channel.split('.');
        let major: u32 = parts.next()?.parse().ok()?;
        let minor: u32 = parts.next()?.parse().ok()?;
        Some(format!("{major}.{}", minor.checked_sub(2)?))
    }

    #[test]
    fn the_msrv_is_the_pinned_toolchain_minus_two_minor_versions() {
        assert_eq!(msrv_for("1.98.1").as_deref(), Some("1.96"));
        assert_eq!(msrv_for("1.98").as_deref(), Some("1.96"));
        assert_eq!(msrv_for("stable"), None);
        assert_eq!(msrv_for("1.1.0"), None);

        let root = crate::workspace::repo_root();
        let read = |path: &str| -> toml::Value {
            toml::from_str(&std::fs::read_to_string(root.join(path)).unwrap()).unwrap()
        };
        let toolchain = read("rust-toolchain.toml");
        let channel = toolchain["toolchain"]["channel"].as_str().unwrap();
        let expected = msrv_for(channel);
        assert!(
            expected.is_some(),
            "rust-toolchain.toml pins `{channel}`, not a version"
        );
        let workspace = read("lablet/Cargo.toml");
        assert_eq!(
            workspace["workspace"]["package"]["rust-version"].as_str(),
            expected.as_deref(),
            "lablet/Cargo.toml: `rust-version` is the MSRV, the toolchain pinned in \
             rust-toolchain.toml ({channel}) minus two minor versions"
        );
        if let Some(own) = read("xtask/Cargo.toml")["package"].get("rust-version") {
            assert_eq!(
                own.as_str(),
                expected.as_deref(),
                "xtask/Cargo.toml: `rust-version`"
            );
        }
    }

    // --- Dependency comments ---

    fn uncommented_keys(text: &str) -> Vec<String> {
        dependency_lines(text)
            .into_iter()
            .filter(|line| !line.commented)
            .map(|line| line.key)
            .collect()
    }

    #[test]
    fn a_comment_on_the_line_above_covers_a_dependency() {
        let text =
            "[dependencies]\n# Errors\nthiserror.workspace = true\n\nserde.workspace = true\n";
        assert_eq!(uncommented_keys(text), ["serde"]);
    }

    #[test]
    fn a_group_header_covers_the_run_of_lines_under_it_and_a_blank_line_ends_it() {
        let text = "[dependencies]\n# Domain\nlablet-model = { path = \"../model\" }\n\
                    lablet-policy = { path = \"../policy\" }\n\n\
                    async-trait.workspace = true\n";
        assert_eq!(uncommented_keys(text), ["async-trait"]);
    }

    #[test]
    fn a_trailing_comment_covers_its_own_line_only() {
        let text = "[dependencies]\nserde.workspace = true # Derives on the model\n\
                    tokio.workspace = true\n";
        assert_eq!(uncommented_keys(text), ["tokio"]);
    }

    #[test]
    fn a_trailing_comment_on_the_closing_line_of_a_value_covers_the_dependency() {
        let text = "[dependencies]\ntokio = { version = \"=1.0.0\", features = [\n  \"rt\",\n] } # Runtime\n\n\
                    serde = { version = \"=1.0.0\", features = [\n  \"derive\", # not at the end of the dependency\n] }\n\n\
                    rmcp.version = \"=3.0.0\"\nrmcp.features = [\n  \"client\",\n] # MCP\n\n\
                    [package.metadata.x]\nlist = [\n] # not a dependency\n";
        assert_eq!(uncommented_keys(text), ["serde"]);
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        let text = "[dependencies]\nodd = { version = \"=1.0.0\", features = [\"a#b\"] }\n";
        assert_eq!(uncommented_keys(text), ["odd"]);
    }

    #[test]
    fn a_comment_above_the_table_header_does_not_cover_what_is_under_it() {
        let text = "# Everything this crate needs\n[dependencies]\nserde.workspace = true\n";
        assert_eq!(uncommented_keys(text), ["serde"]);
    }

    #[test]
    fn every_dependency_table_is_read_and_other_tables_are_not() {
        let text = "[package]\nname = \"x\"\n\n[features]\ndefault = []\n\n\
                    [dependencies]\na = \"1\"\n\n[dev-dependencies]\nb = \"1\"\n\n\
                    [build-dependencies]\nc = \"1\"\n\n\
                    [target.'cfg(unix)'.dependencies]\nd = \"1\"\n\n\
                    [workspace.dependencies]\ne = \"1\"\n\n\
                    [package.metadata.docs.dependencies]\nf = \"1\"\n";
        assert_eq!(uncommented_keys(text), ["a", "b", "c", "d", "e"]);
        let lines = dependency_lines(text);
        assert_eq!(lines[3].table, "[target.'cfg(unix)'.dependencies]");
        assert_eq!(lines[3].line, 17);
    }

    #[test]
    fn a_value_spanning_lines_is_one_dependency() {
        let text = "[dependencies]\n# Runtime\ntokio = { version = \"=1.0.0\", features = [\n\
                    \x20 # not a dependency comment\n  \"rt\",\n\n  \"macros\",\n] }\n\
                    # Errors\nthiserror.workspace = true\n";
        let lines = dependency_lines(text);
        let keys: Vec<&str> = lines.iter().map(|line| line.key.as_str()).collect();
        assert_eq!(keys, ["tokio", "thiserror"]);
        assert!(lines.iter().all(|line| line.commented));
    }

    #[test]
    fn a_dependency_written_as_its_own_table_needs_its_comment_above_the_header() {
        let text = "# Runtime\n[dependencies.tokio]\nversion = \"=1.0.0\"\nfeatures = [\"rt\"]\n\n\
                    [dependencies.serde]\nversion = \"=1.0.0\"\n";
        let lines = dependency_lines(text);
        let keys: Vec<&str> = lines.iter().map(|line| line.key.as_str()).collect();
        assert_eq!(keys, ["tokio", "serde"]);
        assert_eq!(uncommented_keys(text), ["serde"]);
    }

    #[test]
    fn dotted_keys_of_one_dependency_count_once() {
        let text =
            "[dependencies]\n# Runtime\ntokio.version = \"=1.0.0\"\ntokio.features = [\"rt\"]\n";
        assert_eq!(dependency_lines(text).len(), 1);
        // Also with another dependency between them.
        let text = "[dependencies]\ntokio.version = \"=1.0.0\"\nserde = \"=1.0.0\"\n\
                    tokio.features = [\"rt\"]\n";
        let lines: Vec<(usize, String)> = dependency_lines(text)
            .into_iter()
            .map(|line| (line.line, line.key))
            .collect();
        assert_eq!(lines, [(2, "tokio".to_owned()), (3, "serde".to_owned())]);
    }

    #[test]
    fn a_dotted_key_that_reaches_into_a_dependency_table_is_a_dependency() {
        let text = "dependencies.thiserror.workspace = true\n\n[package]\nname = \"x\"\n\n\
                    [target.'cfg(unix)']\ndev-dependencies.wiremock.workspace = true\n\n\
                    [workspace]\n# Errors\ndependencies.anyhow = \"=1.0.0\"\n";
        let lines = dependency_lines(text);
        let seen: Vec<(&str, &str, usize, bool)> = lines
            .iter()
            .map(|line| {
                (
                    line.table.as_str(),
                    line.key.as_str(),
                    line.line,
                    line.commented,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("[dependencies]", "thiserror", 1, false),
                ("[target.cfg(unix).dev-dependencies]", "wiremock", 7, false),
                ("[workspace.dependencies]", "anyhow", 11, true),
            ]
        );
        assert_eq!(lines[1].path, ["target", "cfg(unix)", "dev-dependencies"]);
    }

    #[test]
    fn a_dependency_with_no_line_of_its_own_is_reported_rather_than_passed() {
        // An inline table: valid TOML that cargo accepts and the scan cannot
        // put a comment against.
        let (text, _) = member(LINTS);
        let text = format!("dev-dependencies = {{ wiremock = {{ workspace = true }} }}\n\n{text}");
        let manifest = Manifest::parse(&text).unwrap();
        let violations = check("m", &text, &manifest);
        assert_eq!(rules(&violations), [Rule::DependencyComment]);
        assert_eq!(violations[0].line, None);
        assert!(
            violations[0].to_string().starts_with(
                "m: [dev-dependencies] `wiremock`: could not find this dependency's line"
            ),
            "{}",
            violations[0]
        );
    }

    #[test]
    fn every_commented_spelling_the_scan_reads_is_matched_to_its_parsed_dependency() {
        let (text, manifest) = member(&format!(
            "{LINTS}\n[target.\"cfg(unix)\".dependencies]\n# Signals\nnix.workspace = true\n\n\
             # Runtime\n[dependencies.tokio]\nworkspace = true\nfeatures = [\"rt\"]\n\n\
             [ build-dependencies ]\n# Codegen\n\"prost-build\".workspace = true\n"
        ));
        let violations = check("m", &text, &manifest);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn a_multi_line_string_is_not_read_as_tables_or_keys() {
        let text = "[package]\ndescription = \"\"\"\n[dependencies]\nfake = \"1\"\n\"\"\"\n\n\
                    [dependencies]\n# Real\nreal = \"1\"\n";
        let lines = dependency_lines(text);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].key, "real");
    }

    #[test]
    fn a_multi_line_string_closes_at_its_real_delimiter() {
        for description in [
            // An escaped delimiter inside a basic string.
            "\"\"\"a \\\"\"\" still\n[dependencies]\nfake = \"1\"\n\"\"\"",
            // An escaped backslash, then the real close.
            "\"\"\"a \\\\\"\"\"",
            // A line-ending backslash, and an escape after a multi-byte character.
            "\"\"\"a \\\n  b \u{e9}\\\"\"\" c\n[dependencies]\nfake = \"1\"\n\"\"\"",
            // A literal string has no escapes: the backslash closes nothing off.
            "'''a \\'''",
            // Up to two quotes may stand directly before the delimiter.
            "\"\"\"a\"\"\"\"\"",
            "'''a''''",
        ] {
            let text = format!(
                "[package]\nname = \"x\"\nmetadata = {{ d = {description} }}\n\n[dependencies]\n# Real\nreal = \"1\"\n"
            );
            assert!(
                toml::from_str::<toml::Value>(&text).is_ok(),
                "not TOML: {text}"
            );
            let keys: Vec<String> = dependency_lines(&text)
                .into_iter()
                .map(|line| line.key)
                .collect();
            assert_eq!(keys, ["real"], "{text}");
        }
    }

    #[test]
    fn an_uncommented_member_dependency_is_reported_with_its_line() {
        let (text, manifest) = member(&format!(
            "{LINTS}\n[dependencies]\nthiserror.workspace = true\n"
        ));
        let violations = check("m", &text, &manifest);
        assert_eq!(rules(&violations), [Rule::DependencyComment]);
        assert!(
            violations[0]
                .to_string()
                .starts_with("m:12: [dependencies] `thiserror` has no comment; say why"),
            "{}",
            violations[0]
        );
        assert!(
            violations[0].detail.contains("or at the end of its line"),
            "{}",
            violations[0]
        );
    }

    // --- The xtask copy ---

    /// A faithful copy of `WORKSPACE_LINTS` for an xtask manifest.
    const XTASK_LINTS: &str = "\n[lints.rust]\nunsafe_code = \"forbid\"\n\n[lints.clippy]\n\
                               all = { level = \"warn\", priority = -1 }\nprint_stdout = \"allow\"\n\
                               print_stderr = \"allow\"\n";

    const WORKSPACE_LINTS: &str = r#"
[workspace]
members = []

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
print_stdout = "warn"
"#;

    fn parse(document: &str) -> toml::Value {
        toml::from_str(document).unwrap()
    }

    #[test]
    fn a_faithful_copy_with_the_print_lints_allowed_matches() {
        let xtask = r#"
[lints.rust]
unsafe_code = "forbid"

[lints.clippy]
all = { level = "warn", priority = -1 }
print_stdout = "allow"
print_stderr = "allow"
"#;
        assert_eq!(
            lint_copy_matches(&parse(WORKSPACE_LINTS), &parse(xtask)),
            Ok(())
        );
    }

    #[test]
    fn a_drifted_copy_names_each_lint_that_differs() {
        let xtask = r#"
[lints.rust]
unsafe_code = "warn"

[lints.clippy]
all = { level = "warn", priority = -1 }
print_stdout = "allow"
dbg_macro = "warn"
"#;
        let detail = lint_copy_matches(&parse(WORKSPACE_LINTS), &parse(xtask)).unwrap_err();
        assert!(
            detail.contains(r#"rust.unsafe_code: expected "forbid", found "warn""#),
            "{detail}"
        );
        assert!(
            detail.contains(r#"clippy.dbg_macro: expected absent, found "warn""#),
            "{detail}"
        );
        assert!(
            detail.contains(r#"clippy.print_stderr: expected "allow", found absent"#),
            "{detail}"
        );
    }

    #[test]
    fn xtask_dependencies_follow_the_pin_and_comment_rules() {
        let xtask = "[package]\nname = \"xtask\"\n\n[dependencies]\n# Manifests\ntoml = \"1.1\"\n\n\
                     serde = \"=1.0.229\"\n\n\
                     [lints.rust]\nunsafe_code = \"forbid\"\n\n[lints.clippy]\n\
                     all = { level = \"warn\", priority = -1 }\nprint_stdout = \"allow\"\n\
                     print_stderr = \"allow\"\n";
        let violations = check_xtask("xtask/Cargo.toml", xtask, &parse(WORKSPACE_LINTS));
        assert_eq!(
            rules(&violations),
            [Rule::ExactPin, Rule::DependencyComment]
        );
        assert!(violations[0].detail.contains("`toml`"), "{}", violations[0]);
        assert!(
            violations[1].detail.contains("`serde`"),
            "{}",
            violations[1]
        );
    }

    #[test]
    fn an_xtask_pin_that_differs_from_the_workspace_pin_is_reported() {
        let workspace = format!(
            "{WORKSPACE_LINTS}\n[workspace.dependencies]\n# Manifests\ntoml = \"=1.1.6\"\n\
             # Derives\nserde = {{ version = \"=1.0.229\", features = [\"derive\"] }}\n"
        );
        let xtask = "[package]\nname = \"xtask\"\n\n[dependencies]\n# Manifests\ntoml = \"=1.1.5\"\n\
                     # Views\nserde = \"=1.0.229\"\n# Reports\nserde_json = \"=1.0.151\"\n\n\
                     [lints.rust]\nunsafe_code = \"forbid\"\n\n[lints.clippy]\n\
                     all = { level = \"warn\", priority = -1 }\nprint_stdout = \"allow\"\n\
                     print_stderr = \"allow\"\n";
        let violations = check_xtask("xtask/Cargo.toml", xtask, &parse(&workspace));
        assert_eq!(rules(&violations), [Rule::XtaskPin]);
        assert!(
            violations[0].detail.contains(
                "`toml` is pinned to \"=1.1.5\", but [workspace.dependencies] pins it to \"=1.1.6\""
            ),
            "{}",
            violations[0]
        );
    }

    // --- The sweep ---

    #[test]
    fn the_sweep_names_the_manifest_of_each_finding() {
        let workspace = FixtureWorkspace::new("# Errors\nthiserror = \"=2.0.0\"\n")
            .member(
                "crates/domain/model",
                "lablet-model",
                "[dependencies]\nthiserror = \"2\"\n",
            )
            .load();
        let repo = TempDir::new("repo");
        repo.write("xtask/Cargo.toml", "[package]\nname = \"xtask\"\n");
        repo.write(".cargo/config.toml", "paths = [\"vendor\"]\n");
        let violations = lint(&workspace, repo.path());
        let model: Vec<Rule> = violations
            .iter()
            .filter(|v| v.manifest.ends_with("/crates/domain/model/Cargo.toml"))
            .map(|v| v.rule)
            .collect();
        assert_eq!(model, [Rule::DependencyInherited, Rule::DependencyComment]);
        // The fixture workspace has no [workspace.lints] to compare xtask with.
        let xtask: Vec<Rule> = violations
            .iter()
            .filter(|v| v.manifest == "xtask/Cargo.toml")
            .map(|v| v.rule)
            .collect();
        assert_eq!(xtask, [Rule::XtaskLints]);
        let config: Vec<Rule> = violations
            .iter()
            .filter(|v| v.manifest == ".cargo/config.toml")
            .map(|v| v.rule)
            .collect();
        assert_eq!(config, [Rule::SourceOverride]);
        assert_eq!(violations.len(), 4, "{violations:#?}");
    }

    #[test]
    fn xtask_is_its_own_workspace_with_pinned_and_commented_dependencies() {
        let text = std::fs::read_to_string(crate::workspace::xtask_manifest()).unwrap();
        let document = parse(&text);
        assert!(
            document.get("workspace").is_some(),
            "xtask must stay its own workspace; as a member of lablet/ it would need a \
             `[lints]` table it cannot inherit here"
        );
        // Against a stand-in workspace, only the lint comparison may differ.
        let violations: Vec<Violation> =
            check_xtask("xtask/Cargo.toml", &text, &parse(WORKSPACE_LINTS))
                .into_iter()
                .filter(|v| v.rule != Rule::XtaskLints)
                .collect();
        assert!(violations.is_empty(), "{violations:#?}");
    }

    // --- The real workspace must be clean ---

    #[test]
    fn the_real_manifests_have_no_violations() {
        let workspace = Workspace::load(&crate::workspace::workspace_root()).unwrap();
        let violations = lint(&workspace, &crate::workspace::repo_root());
        let listed: Vec<String> = violations.iter().map(ToString::to_string).collect();
        assert!(
            listed.is_empty(),
            "the manifests break the rules:\n  {}",
            listed.join("\n  ")
        );
    }
}
