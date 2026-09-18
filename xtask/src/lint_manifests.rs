//! Manifest rules (contributing "Code conventions" and "Versioning", spec §8).
//!
//! - Every member opts into `[workspace.lints]` with `[lints] workspace = true`.
//!   A crate that omits the table still compiles clean under a gate that lints
//!   every other crate, so the omission would stay invisible.
//! - Every member inherits `version`, `edition`, `rust-version`, and `license`
//!   from `[workspace.package]`: one workspace version, one MSRV, one licence.
//! - Every third-party dependency of a member is `workspace = true`, and every
//!   third-party entry of `[workspace.dependencies]` is an exact `=x.y.z` pin,
//!   so a version lives in one place and moves only in a dedicated commit.
//! - Every dependency carries a comment saying why it is there: on the line
//!   above it, above the run of dependency lines it belongs to (a group
//!   header), or at the end of its own line.
//! - `xtask/` is its own workspace and cannot inherit, so its `[lints]` must
//!   equal `[workspace.lints]` with the two print lints allowed, its
//!   dependencies follow the same pin and comment rules, and a crate it shares
//!   with `[workspace.dependencies]` is pinned to the same version there.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use crate::workspace::{DependencySpec, Manifest, Workspace};

/// The rule a finding breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// `[lints] workspace = true` is missing.
    LintsInherited,
    /// A `[package]` key is not inherited from `[workspace.package]`.
    PackageInherited,
    /// A third-party dependency of a member carries its own version or source.
    DependencyInherited,
    /// A third-party dependency is not pinned to an exact version.
    ExactPin,
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
    pub const ALL: [Self; 8] = [
        Self::Unreadable,
        Self::LintsInherited,
        Self::PackageInherited,
        Self::DependencyInherited,
        Self::ExactPin,
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
            Self::DependencyInherited => {
                "Third-party dependencies not inherited from the workspace:"
            }
            Self::ExactPin => "Third-party dependencies not pinned to an exact version:",
            Self::DependencyComment => "Dependencies without a comment saying why:",
            Self::XtaskLints => "xtask's copy of the workspace lint set has drifted:",
            Self::XtaskPin => "xtask pins that differ from the workspace's pin of the same crate:",
        }
    }
}

/// One finding: the manifest, the rule, and what to change.
#[derive(Debug)]
pub struct Violation {
    /// The manifest at fault, relative to the repository where possible.
    pub manifest: String,
    /// The rule broken.
    pub rule: Rule,
    /// What is wrong and how to fix it.
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A comment finding leads with its line number: `Cargo.toml:12: ...`.
        let separator = if self.rule == Rule::DependencyComment {
            ":"
        } else {
            ": "
        };
        write!(f, "{}{separator}{}", self.manifest, self.detail)
    }
}

/// The `[package]` keys every member inherits from `[workspace.package]`.
const INHERITED_PACKAGE_KEYS: [&str; 4] = ["version", "edition", "rust-version", "license"];

/// The lints xtask sets to `allow` where the workspace warns: printing is
/// xtask's job.
const XTASK_ALLOWED_LINTS: [&str; 2] = ["print_stdout", "print_stderr"];

/// Every finding over the workspace's manifests and xtask's.
pub fn lint(workspace: &Workspace, xtask_manifest: &Path) -> Vec<Violation> {
    let prefix = workspace
        .root
        .file_name()
        .map(|name| format!("{}/", name.to_string_lossy()))
        .unwrap_or_default();
    let mut violations =
        check_workspace_dependencies(&format!("{prefix}Cargo.toml"), &workspace.text, workspace);
    for member in &workspace.members {
        violations.extend(check_member(
            &format!("{prefix}{}/Cargo.toml", member.path),
            &member.text,
            &member.manifest,
        ));
    }
    let label = "xtask/Cargo.toml";
    match std::fs::read_to_string(xtask_manifest) {
        Ok(text) => violations.extend(check_xtask(label, &text, &workspace.document)),
        Err(e) => violations.push(Violation {
            manifest: label.to_owned(),
            rule: Rule::Unreadable,
            detail: format!("could not read {}: {e}", xtask_manifest.display()),
        }),
    }
    violations
}

/// The member rules: lint and package inheritance, inherited third-party
/// dependencies, and a comment on every dependency.
fn check_member(label: &str, text: &str, manifest: &Manifest) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut report = |rule, detail: String| {
        violations.push(Violation {
            manifest: label.to_owned(),
            rule,
            detail,
        });
    };

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
        for (key, spec) in section.entries {
            if !spec.inherits_workspace() && spec.path().is_none() {
                report(
                    Rule::DependencyInherited,
                    format!(
                        "{} `{key}` carries its own version or source; pin it in \
                         [workspace.dependencies] and write `{key}.workspace = true`",
                        section.header
                    ),
                );
            }
        }
    }

    for line in dependency_lines(text).iter().filter(|line| !line.commented) {
        report(Rule::DependencyComment, uncommented(line));
    }
    violations
}

/// The `[workspace.dependencies]` rules: every third-party entry is an exact
/// pin with a comment. Workspace crates (path entries) are exempt from both.
fn check_workspace_dependencies(label: &str, text: &str, workspace: &Workspace) -> Vec<Violation> {
    let third_party: BTreeMap<&String, &DependencySpec> = workspace
        .table
        .dependencies
        .iter()
        .filter(|(_, spec)| spec.path().is_none())
        .collect();
    let mut violations = check_pins(label, "[workspace.dependencies]", &third_party);
    violations.extend(
        dependency_lines(text)
            .iter()
            .filter(|line| !line.commented && third_party.contains_key(&line.key))
            .map(|line| Violation {
                manifest: label.to_owned(),
                rule: Rule::DependencyComment,
                detail: uncommented(line),
            }),
    );
    violations
}

/// The xtask rules: the lint copy, exact pins, and a comment on every
/// dependency.
fn check_xtask(label: &str, text: &str, workspace_document: &toml::Value) -> Vec<Violation> {
    let violation = |rule, detail| Violation {
        manifest: label.to_owned(),
        rule,
        detail,
    };
    let (document, manifest) = match (toml::from_str::<toml::Value>(text), Manifest::parse(text)) {
        (Ok(document), Ok(manifest)) => (document, manifest),
        (Err(e), _) => return vec![violation(Rule::Unreadable, format!("could not parse: {e}"))],
        (_, Err(e)) => return vec![violation(Rule::Unreadable, format!("could not parse: {e}"))],
    };
    let mut violations = Vec::new();
    if let Err(detail) = lint_copy_matches(workspace_document, &document) {
        violations.push(violation(Rule::XtaskLints, detail));
    }
    for section in manifest.dependency_sections() {
        let third_party: BTreeMap<&String, &DependencySpec> = section
            .entries
            .iter()
            .filter(|(_, spec)| spec.path().is_none())
            .collect();
        violations.extend(check_pins(label, &section.header, &third_party));
        for (key, spec) in third_party {
            let workspace_pin = table_at(workspace_document, &["workspace", "dependencies", key])
                .and_then(|entry| pinned_version(&entry));
            if let (Some(ours), Some(theirs)) = (spec.version(), workspace_pin)
                && ours != theirs
            {
                violations.push(violation(
                    Rule::XtaskPin,
                    format!(
                        "{} `{key}` is pinned to \"{ours}\", but [workspace.dependencies] pins \
                         it to \"{theirs}\"; a crate both build has one version",
                        section.header
                    ),
                ));
            }
        }
    }
    violations.extend(
        dependency_lines(text)
            .iter()
            .filter(|line| !line.commented)
            .map(|line| violation(Rule::DependencyComment, uncommented(line))),
    );
    violations
}

/// The version of a raw dependency entry: the string itself, or its `version`.
fn pinned_version(entry: &toml::Value) -> Option<String> {
    entry
        .as_str()
        .or_else(|| entry.get("version").and_then(toml::Value::as_str))
        .map(str::to_owned)
}

/// A finding for each entry of `table` that is not an exact pin.
fn check_pins(
    label: &str,
    table: &str,
    third_party: &BTreeMap<&String, &DependencySpec>,
) -> Vec<Violation> {
    third_party
        .iter()
        .filter_map(|(key, spec)| {
            let found = match spec.version() {
                Some(version) if is_exact_pin(version) => return None,
                Some(version) => format!("has the requirement \"{version}\""),
                None if spec.is_git() => "is a git dependency".to_owned(),
                None => "has no version".to_owned(),
            };
            Some(Violation {
                manifest: label.to_owned(),
                rule: Rule::ExactPin,
                detail: format!(
                    "{table} `{key}` {found}; third-party crates are pinned to an exact \
                     version, written \"=x.y.z\""
                ),
            })
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

/// The finding for a dependency no comment covers, led by its line number so
/// the whole reads `path/Cargo.toml:12: ...`.
fn uncommented(line: &DependencyLine) -> String {
    format!(
        "{}: {} `{}` has no comment; say why the dependency is there on the line above it, \
         or put it under a group header comment",
        line.line, line.table, line.key
    )
}

// --- Reading comments, which a TOML parser drops ---

/// One dependency as it is written in a manifest.
#[derive(Debug, PartialEq, Eq)]
struct DependencyLine {
    /// The header of the table the dependency is in, as written.
    table: String,
    /// The dependency key.
    key: String,
    /// The 1-based line the dependency starts on.
    line: usize,
    /// Whether a comment covers it: one earlier in the same run of non-blank
    /// lines of its table, or one at the end of its own line.
    commented: bool,
}

/// The names of cargo's dependency tables.
const DEPENDENCY_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// What a table header means to the comment check.
enum Header {
    /// A dependency table: the key lines under it are dependencies.
    Table,
    /// A dependency written as its own table, `[dependencies.foo]`.
    Entry(String),
    /// Anything else.
    Other,
}

/// Classifies a header's key path. A dependency table is `dependencies` (or
/// its dev and build forms) at the top level, under `workspace`, or under
/// `target.<spec>`.
fn classify_header(path: &[String]) -> Header {
    let is_scope = |scope: &[String]| match scope {
        [] => true,
        [only] => only == "workspace",
        [first, _] => first == "target",
        _ => false,
    };
    let is_table = |name: &String| DEPENDENCY_TABLES.contains(&name.as_str());
    match path {
        [scope @ .., table] if is_table(table) && is_scope(scope) => Header::Table,
        [scope @ .., table, key] if is_table(table) && is_scope(scope) => {
            Header::Entry(key.clone())
        }
        _ => Header::Other,
    }
}

/// Every dependency in a manifest's text, in order, with whether a comment
/// covers it.
fn dependency_lines(text: &str) -> Vec<DependencyLine> {
    let mut found = Vec::new();
    let mut scanner = ValueScanner::default();
    // The header of the dependency table being read, if one is.
    let mut table: Option<String> = None;
    // Whether a comment has been seen in the current run of non-blank lines.
    let mut covered = false;
    // The last key seen, so `foo.version` and `foo.features` count once.
    let mut last_key: Option<String> = None;

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if scanner.inside_value() {
            scanner.scan(line);
        } else if trimmed.is_empty() {
            covered = false;
        } else if trimmed.starts_with('#') {
            covered = true;
        } else if trimmed.starts_with('[') {
            // `[table]` or `[[array-of-tables]]`, rebuilt without what follows it.
            let inner = trimmed.trim_start_matches('[');
            let brackets = trimmed.len() - inner.len();
            let (path, end) = key_path(inner, ']');
            let header = format!(
                "{}{}{}",
                "[".repeat(brackets),
                &inner[..end],
                "]".repeat(brackets)
            );
            table = None;
            match classify_header(&path) {
                Header::Table => table = Some(header),
                Header::Entry(key) => found.push(DependencyLine {
                    table: header,
                    key,
                    line: index + 1,
                    commented: covered || inner[end..].contains('#'),
                }),
                Header::Other => {}
            }
            covered = false;
            last_key = None;
        } else {
            let trailing_comment = scanner.scan(line);
            let Some(table) = &table else {
                continue;
            };
            let (path, _) = key_path(trimmed, '=');
            let Some(key) = path.into_iter().next() else {
                continue;
            };
            if last_key.as_ref() == Some(&key) {
                continue;
            }
            found.push(DependencyLine {
                table: table.clone(),
                key: key.clone(),
                line: index + 1,
                commented: covered || trailing_comment,
            });
            last_key = Some(key);
        }
    }
    found
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
                let Some(close) = line.get(at..).and_then(|rest| rest.find(delimiter)) else {
                    return false;
                };
                at += close + delimiter.len();
                self.multiline = None;
                continue;
            }
            match bytes[at] {
                b'#' => return true,
                quote @ (b'"' | b'\'') => {
                    let triple = if quote == b'"' { "\"\"\"" } else { "'''" };
                    if line.get(at..).is_some_and(|rest| rest.starts_with(triple)) {
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
        let violations = check_member("m", &text, &manifest);
        assert!(violations.is_empty(), "{violations:#?}");
    }

    #[test]
    fn a_member_with_no_lints_table_is_reported() {
        let (text, manifest) = member("");
        assert_eq!(
            rules(&check_member("m", &text, &manifest)),
            [Rule::LintsInherited]
        );
    }

    #[test]
    fn a_lints_table_that_opts_out_is_reported() {
        let (text, manifest) = member("[lints]\nworkspace = false\n");
        assert_eq!(
            rules(&check_member("m", &text, &manifest)),
            [Rule::LintsInherited]
        );
    }

    #[test]
    fn crate_local_lints_without_the_inheritance_flag_are_reported() {
        // The near-miss the check most needs to catch.
        let (text, manifest) = member("[lints.clippy]\nunwrap_used = \"warn\"\n");
        assert_eq!(
            rules(&check_member("m", &text, &manifest)),
            [Rule::LintsInherited]
        );
    }

    // --- Package inheritance ---

    #[test]
    fn each_package_key_written_out_instead_of_inherited_is_reported() {
        let text = format!(
            "[package]\nname = \"lablet-x\"\nversion = \"0.1.0\"\nedition.workspace = true\n\
             license = \"MIT\"\n\n{LINTS}"
        );
        let manifest = Manifest::parse(&text).unwrap();
        let violations = check_member("m", &text, &manifest);
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
        let violations = check_member("m", &text, &manifest);
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
        assert!(check_member("m", &text, &manifest).is_empty());
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
    fn an_uncommented_member_dependency_is_reported_with_its_line() {
        let (text, manifest) = member(&format!(
            "{LINTS}\n[dependencies]\nthiserror.workspace = true\n"
        ));
        let violations = check_member("m", &text, &manifest);
        assert_eq!(rules(&violations), [Rule::DependencyComment]);
        assert!(
            violations[0]
                .to_string()
                .starts_with("m:12: [dependencies] `thiserror` has no comment; say why"),
            "{}",
            violations[0]
        );
    }

    // --- The xtask copy ---

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
        let dir = TempDir::new("xtask-manifest");
        dir.write("Cargo.toml", "[package]\nname = \"xtask\"\n");
        let violations = lint(&workspace, &dir.path().join("Cargo.toml"));
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
        assert_eq!(violations.len(), 3, "{violations:#?}");
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
        let root = crate::workspace::workspace_root();
        if !root.join("Cargo.toml").is_file() {
            eprintln!(
                "skipped: {} does not exist yet, so there are no real manifests to lint",
                root.join("Cargo.toml").display()
            );
            return;
        }
        let workspace = Workspace::load(&root).unwrap();
        let violations = lint(&workspace, &crate::workspace::xtask_manifest());
        let listed: Vec<String> = violations.iter().map(ToString::to_string).collect();
        assert!(
            listed.is_empty(),
            "the manifests break the rules:\n  {}",
            listed.join("\n  ")
        );
    }
}
