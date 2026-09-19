//! The manifest rules of contributing/README.md ("Architecture rules", "Code
//! conventions", "Versioning"). Members only inherit, so a version, a path,
//! or a rename lives in one table, and these rules and `lint-layers` have one
//! place to read. xtask cannot inherit, so its manifest is held to a copy.
//! Whatever swaps a pinned crate for other code is refused here in xtask's
//! manifest and the cargo configuration, and by [`Workspace::load`] in the
//! workspace root.

use std::collections::BTreeMap;
use std::path::Path;

use crate::workspace::{Member, Workspace, inherits_workspace, unsupported};

const INHERITED_PACKAGE_KEYS: [&str; 4] = ["version", "edition", "rust-version", "license"];

const INHERITED_ENTRY_KEYS: [&str; 4] = ["workspace", "features", "optional", "default-features"];

/// Anything else names another source or is not a key lablet uses.
const PIN_KEYS: [&str; 4] = ["version", "features", "default-features", "package"];

const INTERNAL_KEYS: [&str; 2] = ["path", "version"];

/// Printing is xtask's job, so it allows these where the workspace warns.
const XTASK_ALLOWED_LINTS: [&str; 2] = ["print_stdout", "print_stderr"];

/// The members whose package is not `lablet-<directory name>` (spec §2).
const PACKAGE_NAME_EXCEPTIONS: [(&str, &str); 3] = [
    ("apps/lablet", "lablet"),
    ("tests/conformance", "lablet-conformance"),
    ("tests/mcp-server", "lablet-test-mcp-server"),
];

/// Every finding over the workspace's manifests, xtask's, and the cargo
/// configuration of the repository at `repo`, one line each.
pub fn lint(workspace: &Workspace, repo: &Path) -> Vec<String> {
    let prefix = workspace
        .root
        .file_name()
        .map(|name| format!("{}/", name.to_string_lossy()))
        .unwrap_or_default();
    let mut findings = check_workspace_dependencies(&format!("{prefix}Cargo.toml"), workspace);
    for member in &workspace.members {
        let label = format!("{prefix}{}/Cargo.toml", member.path);
        findings.extend(check_member(&label, member));
    }
    let label = "xtask/Cargo.toml";
    match std::fs::read_to_string(repo.join(label)) {
        Ok(text) => findings.extend(check_xtask(label, &text, workspace)),
        Err(e) => findings.push(format!("{label}: could not read: {e}")),
    }
    // Cargo reads a `.cargo/config.toml` from the directory it starts in and
    // every one above it; these two are the repository's.
    let config = ".cargo/config.toml";
    for (label, path) in [
        (config.to_owned(), repo.join(config)),
        (format!("{prefix}{config}"), workspace.root.join(config)),
    ] {
        if let Ok(text) = std::fs::read_to_string(&path) {
            findings.extend(check_cargo_config(&label, &text));
        }
    }
    findings
}

fn check_member(label: &str, member: &Member) -> Vec<String> {
    let mut findings = Vec::new();
    let manifest = &member.manifest;
    if !manifest.lints.as_ref().is_some_and(inherits_workspace) {
        findings.push(format!(
            "{label}: no `[lints]` table with `workspace = true`; add one so the crate inherits \
             [workspace.lints]"
        ));
    }
    for key in INHERITED_PACKAGE_KEYS {
        let inherited = manifest.package.keys.get(key);
        if !inherited.is_some_and(inherits_workspace) {
            findings.push(format!(
                "{label}: [package] `{key}` is not inherited; write `{key}.workspace = true` so \
                 the value comes from [workspace.package]"
            ));
        }
    }
    let expected = expected_package_name(&member.path);
    if expected != member.name {
        findings.push(format!(
            "{label}: [package] `name` is \"{}\", but the crate in `{}` is package \
             \"{expected}\": directory `foo/bar/` is package `lablet-bar`, apart from the three \
             exceptions in spec §2",
            member.name, member.path
        ));
    }
    for section in manifest.dependency_sections() {
        for (key, spec) in section.entries {
            if !inherits_workspace(spec) || foreign_key(spec, &INHERITED_ENTRY_KEYS).is_some() {
                findings.push(format!(
                    "{label}: {} `{key}` is not exactly workspace-inherited; declare the crate \
                     once, in [workspace.dependencies], and write `{key}.workspace = true` here, \
                     with nothing beside it but `features`, `optional`, or `default-features`. \
                     A member names no version, path, git source, registry, or package of its own",
                    section.header
                ));
            }
        }
    }
    findings
}

fn expected_package_name(member_path: &str) -> String {
    let exception = PACKAGE_NAME_EXCEPTIONS
        .iter()
        .find(|(path, _)| *path == member_path);
    let directory = member_path.rsplit('/').next().unwrap_or_default();
    exception.map_or_else(
        || format!("lablet-{directory}"),
        |(_, name)| (*name).to_owned(),
    )
}

fn check_workspace_dependencies(label: &str, workspace: &Workspace) -> Vec<String> {
    let table = "[workspace.dependencies]";
    let mut findings = Vec::new();
    for (key, spec) in &workspace.dependencies {
        let fault = if spec.get("path").is_some() {
            internal_fault(workspace, spec)
        } else {
            pin_fault(spec)
        };
        findings.extend(fault.map(|fault| format!("{label}: {table} `{key}` {fault}")));
    }
    let keys = workspace.dependencies.keys();
    findings.extend(comment_findings(label, &workspace.text, table, keys));
    findings
}

fn foreign_key<'a>(spec: &'a toml::Value, allowed: &[&str]) -> Option<&'a String> {
    let mut keys = spec.as_table()?.keys();
    keys.find(|key| !allowed.contains(&key.as_str()))
}

/// A `path` makes an entry internal: it must be `{ path, version }`, the path
/// a listed member, the version the workspace's.
fn internal_fault(workspace: &Workspace, spec: &toml::Value) -> Option<String> {
    if let Some(key) = foreign_key(spec, &INTERNAL_KEYS) {
        return Some(format!(
            "carries `{key}`; an internal entry is `{{ path, version }}` and nothing else"
        ));
    }
    let path = spec.get("path").and_then(toml::Value::as_str).unwrap_or("");
    if workspace.member_at(path).is_none() {
        return Some(format!(
            "has the path `{path}`, which is not a directory listed in [workspace] members; \
             write the member's path exactly as it is listed there"
        ));
    }
    let package = workspace
        .document
        .get("workspace")
        .and_then(|table| table.get("package"));
    match package.and_then(|package| package.get("version")) {
        None => Some("cannot be checked: [workspace.package] sets no `version`".to_owned()),
        Some(version) if spec.get("version") == Some(version) => None,
        Some(version) => Some(format!(
            "does not carry the workspace version; write `version = {version}` beside the path \
             ([workspace.package] `version`)"
        )),
    }
}

fn pin_fault(spec: &toml::Value) -> Option<String> {
    let found = match (foreign_key(spec, &PIN_KEYS), pinned_version(spec)) {
        (Some(key), _) => format!("carries `{key}`"),
        (None, Some(version)) if is_exact_pin(version) => return None,
        (None, Some(version)) => format!("has the requirement \"{version}\""),
        (None, None) => "has no version".to_owned(),
    };
    Some(format!(
        "{found}; a third-party crate comes from crates.io, pinned to an exact version written \
         \"=x.y.z\", with no git, registry, path, or branch key"
    ))
}

fn pinned_version(entry: &toml::Value) -> Option<&str> {
    entry
        .as_str()
        .or_else(|| entry.get("version").and_then(toml::Value::as_str))
}

/// `=x.y.z`, with an optional pre-release or build suffix, and nothing else.
fn is_exact_pin(requirement: &str) -> bool {
    let Some(version) = requirement.strip_prefix('=') else {
        return false;
    };
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let numeric = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    core.split('.').count() == 3
        && core.split('.').all(numeric)
        && !version.contains([',', ' ', '*'])
}

/// A finding for each of `keys` with no comment line directly above its line
/// under the literal `header` line. The TOML parser names the keys; this scan
/// only finds their lines, and a key written so the scan misses it (quoted,
/// `[header.key]`, dotted from another table) is a finding too, never a pass.
/// A group header that touches the entry below it passes as its comment, so
/// the manifests keep headers apart with a blank line.
fn comment_findings<'a>(
    label: &str,
    text: &str,
    header: &str,
    keys: impl Iterator<Item = &'a String>,
) -> Vec<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let start = lines
        .iter()
        .position(|line| *line == header)
        .map_or(lines.len(), |at| at + 1);
    let end = lines[start..]
        .iter()
        .position(|line| line.starts_with('['))
        .map_or(lines.len(), |length| start + length);
    let starts_entry = |line: &str, key: &str| {
        line.strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with(['=', '.']))
    };
    let mut findings = Vec::new();
    for key in keys {
        match (start..end).find(|at| starts_entry(lines[*at], key)) {
            Some(at) if lines[at - 1].starts_with('#') => {}
            Some(at) => findings.push(format!(
                "{label}:{}: {header} `{key}` has no comment; say why the dependency is there \
                 in a comment on the line directly above it. A trailing comment or a comment \
                 above a blank line does not count, so a group header covers no entry: keep it \
                 apart with a blank line and give every entry its own comment",
                at + 1
            )),
            None => findings.push(format!(
                "{label}: {header} `{key}`: could not find this entry's line to check its \
                 comment; write it as a bare `{key} = ...` line under a literal `{header}` line"
            )),
        }
    }
    findings
}

fn check_xtask(label: &str, text: &str, workspace: &Workspace) -> Vec<String> {
    let document: toml::Value = match toml::from_str(text) {
        Ok(document) => document,
        Err(e) => return vec![format!("{label}: could not parse: {e}")],
    };
    // `target` would hold dependency tables the loop below does not read.
    let mut findings = refused(label, &document, &["patch", "replace", "target"]);
    let drift = lint_copy_drift(&workspace.document, &document);
    if !drift.is_empty() {
        findings.push(format!(
            "{label}: [lints] must be [workspace.lints] with print_stdout and print_stderr \
             allowed:\n    {}",
            drift.join("\n    ")
        ));
    }
    for name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(entries) = document.get(name).and_then(toml::Value::as_table) else {
            continue;
        };
        let table = format!("[{name}]");
        for (key, spec) in entries {
            let fault = pin_fault(spec);
            findings.extend(fault.map(|fault| format!("{label}: {table} `{key}` {fault}")));
            let theirs = workspace.dependencies.get(key).and_then(pinned_version);
            if let (Some(ours), Some(theirs)) = (pinned_version(spec), theirs)
                && ours != theirs
            {
                findings.push(format!(
                    "{label}: {table} `{key}` is pinned to \"{ours}\", but \
                     [workspace.dependencies] pins it to \"{theirs}\"; bump both in the same \
                     dedicated commit"
                ));
            }
        }
        findings.extend(comment_findings(label, text, &table, entries.keys()));
    }
    findings
}

/// `patch`, `paths`, and `source` swap a pinned crate for other code, as
/// `[patch]` in a manifest does.
fn check_cargo_config(label: &str, text: &str) -> Vec<String> {
    match toml::from_str::<toml::Value>(text) {
        Ok(document) => refused(label, &document, &["patch", "paths", "source"]),
        Err(e) => vec![format!("{label}: could not parse: {e}")],
    }
}

fn refused(label: &str, document: &toml::Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter(|key| document.get(**key).is_some())
        .map(|key| format!("{label}: {}", unsupported(&format!("`{key}`"))))
        .collect()
}

/// Every lint whose level differs between `[workspace.lints]`, with the print
/// lints allowed, and xtask's `[lints]`.
fn lint_copy_drift(workspace: &toml::Value, xtask: &toml::Value) -> Vec<String> {
    let lints = workspace
        .get("workspace")
        .and_then(|table| table.get("lints"));
    let mut expected = lint_levels(lints);
    for lint in XTASK_ALLOWED_LINTS {
        expected.insert(format!("clippy.{lint}"), "\"allow\"".to_owned());
    }
    let mut actual = lint_levels(xtask.get("lints"));
    let mut drift = Vec::new();
    for (lint, level) in &expected {
        let found = actual.remove(lint).unwrap_or_else(|| "absent".to_owned());
        if found != *level {
            drift.push(format!("{lint}: expected {level}, found {found}"));
        }
    }
    let unexpected = actual.into_iter();
    drift.extend(unexpected.map(|(lint, level)| format!("{lint}: expected absent, found {level}")));
    drift
}

fn lint_levels(table: Option<&toml::Value>) -> BTreeMap<String, String> {
    let groups = table.and_then(toml::Value::as_table).into_iter().flatten();
    groups
        .filter_map(|(group, lints)| lints.as_table().map(|lints| (group, lints)))
        .flat_map(|(group, lints)| {
            lints
                .iter()
                .map(move |(lint, level)| (format!("{group}.{lint}"), level.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::{FixtureWorkspace, TempDir, assert_findings};
    use crate::workspace::{repo_root, workspace_root};

    const PACKAGE: &str = "[package]\nname = \"lablet-x\"\nversion.workspace = true\n\
                           edition.workspace = true\nrust-version.workspace = true\n\
                           license.workspace = true\n";
    const LINTS: &str = "[lints]\nworkspace = true\n";

    /// The findings for a member in `crates/domain/x` with this manifest.
    fn check(text: &str) -> Vec<String> {
        let manifest: crate::workspace::Manifest = toml::from_str(text).unwrap();
        let member = Member {
            path: "crates/domain/x".to_owned(),
            name: manifest.package.name.clone(),
            manifest,
        };
        check_member("m", &member)
    }

    #[test]
    fn a_member_that_inherits_everything_is_clean() {
        let violations = check(&format!(
            "{PACKAGE}\n{LINTS}\n[dependencies]\nthiserror.workspace = true\n\
             serde = {{ workspace = true, features = [\"rc\"], optional = true }}\n\n\
             [target.'cfg(unix)'.dev-dependencies]\n\
             tokio = {{ workspace = true, default-features = false }}\n"
        ));
        assert_findings(&violations, &[]);
    }

    #[test]
    fn a_member_that_does_not_inherit_the_workspace_lints_is_reported() {
        // The last is the near-miss the check most needs to catch.
        for lints in [
            "",
            "[lints]\nworkspace = false\n",
            "[lints.clippy]\nunwrap_used = \"warn\"\n",
        ] {
            let violations = check(&format!("{PACKAGE}\n{lints}"));
            assert_findings(&violations, &["no `[lints]` table with `workspace = true`"]);
        }
    }

    #[test]
    fn each_package_key_written_out_instead_of_inherited_is_reported() {
        let violations = check(&format!(
            "[package]\nname = \"lablet-x\"\nversion = \"0.1.0\"\nedition.workspace = true\n\
             license = \"MIT\"\n\n{LINTS}"
        ));
        assert_findings(
            &violations,
            &[
                "[package] `version` is not inherited",
                "[package] `rust-version` is not inherited",
                "[package] `license` is not inherited",
            ],
        );
    }

    #[test]
    fn a_member_dependency_that_is_not_exactly_inherited_is_reported_in_every_table() {
        for table in [
            "[dependencies]\ntokio = \"=1.0.0\"",
            "[dependencies]\ntokio = { version = \"=1.0.0\" }",
            "[dependencies]\ntokio = { workspace = false }",
            "[dependencies]\nlablet-model = { path = \"../model\" }",
            "[dev-dependencies]\nrt = { workspace = true, package = \"tokio\" }",
            "[dev-dependencies]\ntokio = { workspace = true, version = \"=1.0.0\" }",
            "[build-dependencies]\nprost-build = { git = \"https://example.com/prost\" }",
            "[target.'cfg(unix)'.dependencies]\nnix = { version = \"=1.0.0\", registry = \"x\" }",
        ] {
            let violations = check(&format!("{PACKAGE}\n{LINTS}\n{table}\n"));
            let (header, entry) = table.split_once('\n').unwrap();
            let key = entry.split(' ').next().unwrap();
            let phrase = format!("m: {header} `{key}` is not exactly workspace-inherited");
            assert_findings(&violations, &[&phrase]);
        }
    }

    #[test]
    fn a_package_is_named_after_its_directory_with_three_exceptions() {
        for (path, name) in [
            ("crates/domain/model", "lablet-model"),
            (
                "crates/adapters/secondary/shared/http-util",
                "lablet-http-util",
            ),
            ("apps/lablet", "lablet"),
            ("tests/conformance", "lablet-conformance"),
            ("tests/mcp-server", "lablet-test-mcp-server"),
        ] {
            assert_eq!(expected_package_name(path), name);
        }
        let violations = check(&format!("{PACKAGE}\n{LINTS}").replace("lablet-x", "x_types"));
        assert_findings(
            &violations,
            &["`name` is \"x_types\", but the crate in `crates/domain/x` is package \"lablet-x\""],
        );
    }

    #[test]
    fn only_a_full_version_behind_an_equals_sign_is_an_exact_pin() {
        for pin in ["=1.2.3", "=0.1.0", "=1.0.0-rc.1", "=1.1.6+spec-1.1.0"] {
            assert!(is_exact_pin(pin), "{pin}");
        }
        let loose = "1.2.3 ^1.2.3 ~1.2 =1.2 =1 * =1.2.* >=1.2.3 =1.2.x";
        let spaced = ["=1.2.3, <2", "= 1.2.3", "", "=1.2.3-rc.1, <2", "=1.2.3-*"];
        for loose in loose.split(' ').chain(spaced) {
            assert!(!is_exact_pin(loose), "{loose}");
        }
    }

    #[test]
    fn a_workspace_entry_is_an_exact_crates_io_pin_or_a_listed_member() {
        let mut workspace = FixtureWorkspace::new(
            "# a\nasync-trait = \"=0.1.89\"\n\
             # b\nserde = { version = \"=1.0.0\", default-features = false, features = [\"derive\"] }\n\
             # c\notel = { package = \"opentelemetry\", version = \"=0.33.0\" }\n\
             # d\nlablet-model = { path = \"crates/domain/model\", version = \"0.1.0\" }\n\
             # e\nloose = { version = \"1.47\" }\n\
             # f\nunversioned = { features = [\"x\"] }\n\
             # g\nfrom-git = { git = \"https://example.com/x\", branch = \"main\", version = \"=1.0.0\" }\n\
             # h\nfrom-registry = { version = \"=1.0.0\", registry = \"corp\" }\n\
             # i\nvendored = { path = \"../vendor/thing\", version = \"0.1.0\" }\n\
             # j\nlablet-policy = { path = \"./crates/domain/model\", version = \"0.1.0\" }\n\
             # k\nlablet-run = { path = \"crates/domain/model\", version = \"0.2.0\" }\n\
             # l\nlablet-renamed = { path = \"crates/domain/model\", version = \"0.1.0\", package = \"x\" }\n",
        )
        .member("crates/domain/model", "lablet-model", "")
        .load();
        assert_findings(
            &check_workspace_dependencies("w", &workspace),
            &[
                "`from-git` carries `branch`; a third-party crate comes from crates.io",
                "`from-registry` carries `registry`",
                "`lablet-policy` has the path `./crates/domain/model`, which is not a directory listed",
                "`lablet-renamed` carries `package`; an internal entry is",
                "`lablet-run` does not carry the workspace version; write `version = \"0.1.0\"` \
                 beside the path",
                "`loose` has the requirement \"1.47\"",
                "`unversioned` has no version",
                "`vendored` has the path `../vendor/thing`, which is not a directory listed",
            ],
        );
        // Cargo refuses this first, so the line only has to be true.
        workspace.document = toml::Value::Table(toml::Table::new());
        let findings = check_workspace_dependencies("w", &workspace);
        let unchecked = "`lablet-model` cannot be checked: [workspace.package] sets no `version`";
        assert!(findings.iter().any(|finding| finding.contains(unchecked)));
    }

    #[test]
    fn a_dependency_needs_a_comment_on_the_line_directly_above_it() {
        // The fixture's [workspace.dependencies] header is line 11.
        let workspace = FixtureWorkspace::new(
            "# Group header\n\n\
             # Why a: a comment block\n# of two lines\na = \"=1.0.0\"\n\
             b = \"=1.0.0\"\n\
             # A comment with a blank line under it\n\n\
             c = \"=1.0.0\"\n\
             d = \"=1.0.0\" # a trailing comment\n\
             # Why e\ne = { version = \"=1.0.0\", features = [\n  # not a dependency\n  \"x\",\n] }\n\
             # Why f, in its dotted spelling\nf.version = \"=1.0.0\"\nf.features = [\"x\"]\n\
             # Why serde_json, whose line starts with the next key\n\
             serde_json = \"=1.0.0\"\nserde = \"=1.0.0\"\n",
        )
        .load();
        assert_findings(
            &check_workspace_dependencies("w", &workspace),
            &[
                "w:17: [workspace.dependencies] `b` has no comment; say why the dependency is \
                 there in a comment on the line directly above it. A trailing comment",
                "w:20: [workspace.dependencies] `c` has no comment",
                "w:21: [workspace.dependencies] `d` has no comment",
                "w:32: [workspace.dependencies] `serde` has no comment",
            ],
        );
    }

    #[test]
    fn a_dependency_whose_line_the_scan_cannot_find_is_reported_rather_than_passed() {
        // Each is valid TOML that names the dependency `a` some other way.
        for text in [
            "[dependencies.a]\nversion = \"=1.0.0\"\n",
            "dependencies.a = \"=1.0.0\"\n",
            "[dependencies]\n# Why\n\"a\" = \"=1.0.0\"\n",
            "[ dependencies ]\n# Why\na = \"=1.0.0\"\n",
            // The scan stops where the table does.
            "[dependencies]\n\"a\" = \"=1.0.0\"\n\n[dev-dependencies]\n# Why\na = \"=1.0.0\"\n",
        ] {
            let parsed: toml::Value = toml::from_str(text).unwrap();
            let keys = parsed["dependencies"].as_table().unwrap().keys();
            let violations = comment_findings("m", text, "[dependencies]", keys);
            assert_findings(
                &violations,
                &["m: [dependencies] `a`: could not find this entry's line"],
            );
        }
    }

    /// A workspace whose lint set is small enough to copy by hand.
    fn workspace_with_lints(dependencies: &str) -> Workspace {
        FixtureWorkspace::new(&format!(
            "{dependencies}\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n\n\
             [workspace.lints.clippy]\nall = {{ level = \"warn\", priority = -1 }}\n\
             print_stdout = \"warn\"\n"
        ))
        .load()
    }

    const XTASK_LINTS: &str = "[lints.rust]\nunsafe_code = \"forbid\"\n\n[lints.clippy]\n\
                               all = { level = \"warn\", priority = -1 }\nprint_stdout = \"allow\"\n\
                               print_stderr = \"allow\"\n";

    #[test]
    fn a_drifted_lint_copy_names_each_lint_that_differs() {
        let xtask = "[package]\nname = \"xtask\"\n\n[lints.rust]\nunsafe_code = \"warn\"\n\n\
                     [lints.clippy]\nall = { level = \"warn\", priority = -1 }\n\
                     print_stdout = \"allow\"\ndbg_macro = \"warn\"\n";
        let violations = check_xtask("x", xtask, &workspace_with_lints(""));
        assert_findings(&violations, &["[lints] must be [workspace.lints] with"]);
        for line in [
            r#"rust.unsafe_code: expected "forbid", found "warn""#,
            r#"clippy.print_stderr: expected "allow", found absent"#,
            r#"clippy.dbg_macro: expected absent, found "warn""#,
        ] {
            assert!(violations[0].contains(line), "{}", violations[0]);
        }
    }

    #[test]
    fn xtask_dependencies_follow_the_pin_and_comment_rules_and_agree_with_the_workspace() {
        let xtask = format!(
            "[package]\nname = \"xtask\"\n\n[dependencies]\n# Manifests\ntoml = \"1.1\"\n\n\
             serde = \"=1.0.229\"\n# Reports\nserde_json = \"=1.0.150\"\n\n\
             [dev-dependencies]\n# Fixtures\ntempfile = \"=3.0.0\"\n\
             # Vendored\ncc = {{ path = \"../vendor/cc\" }}\n\n{XTASK_LINTS}"
        );
        let workspace = workspace_with_lints(
            "# Reports\nserde_json = { version = \"=1.0.151\", features = [\"std\"] }\n",
        );
        assert_findings(
            &check_xtask("x", &xtask, &workspace),
            &[
                "`serde_json` is pinned to \"=1.0.150\", but [workspace.dependencies] pins it to \
                 \"=1.0.151\"",
                "[dependencies] `toml` has the requirement \"1.1\"",
                "x:8: [dependencies] `serde` has no comment",
                "[dev-dependencies] `cc` carries `path`",
            ],
        );
    }

    #[test]
    fn a_table_or_setting_that_swaps_a_pinned_crate_is_refused() {
        let workspace = workspace_with_lints("");
        // The table being there is enough, whatever it holds.
        for (key, table) in [
            ("patch", "[patch.crates-io]\n"),
            ("replace", "[replace]\n"),
            ("target", "[target.'cfg(unix)'.dependencies]\n"),
        ] {
            let xtask = format!("[package]\nname = \"xtask\"\n\n{XTASK_LINTS}\n{table}");
            let phrase = format!("x: `{key}` is not supported by lablet's lints");
            assert_findings(&check_xtask("x", &xtask, &workspace), &[&phrase]);
        }
        for (key, config) in [
            ("patch", "[patch.crates-io]\n"),
            ("paths", "paths = [\"../vendor/serde\"]\n"),
            ("source", "[source.crates-io]\n"),
        ] {
            let phrase = format!("c: `{key}` is not supported by lablet's lints");
            assert_findings(&check_cargo_config("c", config), &[&phrase]);
        }
        assert_findings(&check_cargo_config("c", "[alias"), &["c: could not parse"]);
    }

    #[test]
    fn the_sweep_runs_every_pass_and_names_the_file_of_each_finding() {
        // One fault per pass, so a pass the sweep drops is a finding missed.
        let fixture = FixtureWorkspace::new("# Errors\nthiserror = \"2.0\"\n").member(
            "crates/domain/model",
            "lablet-model",
            "[dependencies]\nthiserror = \"2\"\n",
        );
        fixture.write(".cargo/config.toml", "[source.crates-io]\n");
        let workspace = fixture.load();
        let repo = TempDir::new("repo");
        repo.write("xtask/Cargo.toml", "[package]\nname = \"xtask\"\n");
        repo.write(".cargo/config.toml", "paths = [\"vendor\"]\n");
        let root = workspace.root.file_name().unwrap().to_string_lossy();
        assert_findings(
            &lint(&workspace, repo.path()),
            &[
                &format!("{root}/Cargo.toml: [workspace.dependencies] `thiserror`"),
                &format!("{root}/crates/domain/model/Cargo.toml: [dependencies] `thiserror`"),
                "xtask/Cargo.toml: [lints] must be",
                ".cargo/config.toml: `paths` is not supported",
                &format!("{root}/.cargo/config.toml: `source` is not supported"),
            ],
        );
        std::fs::remove_file(repo.path().join("xtask/Cargo.toml")).unwrap();
        let findings = lint(&workspace, repo.path());
        assert!(findings[2].starts_with("xtask/Cargo.toml: could not read"));
    }

    #[test]
    fn the_real_manifests_have_no_violations() {
        let workspace = Workspace::load(&workspace_root()).unwrap();
        assert!(!workspace.dependencies.is_empty());
        assert_findings(&lint(&workspace, &repo_root()), &[]);
    }

    #[test]
    fn the_msrv_is_the_pinned_toolchain_minus_two_minor_versions() {
        let read = |path: &str| -> toml::Value {
            toml::from_str(&std::fs::read_to_string(repo_root().join(path)).unwrap()).unwrap()
        };
        let toolchain = read("rust-toolchain.toml");
        let channel = toolchain["toolchain"]["channel"].as_str().unwrap();
        let mut parts = channel.split('.').map(|part| part.parse::<u32>().unwrap());
        let (major, minor) = (parts.next().unwrap(), parts.next().unwrap());
        assert_eq!(
            read("lablet/Cargo.toml")["workspace"]["package"]["rust-version"].as_str(),
            Some(format!("{major}.{}", minor - 2).as_str()),
            "lablet/Cargo.toml: `rust-version` is the MSRV, the toolchain pinned in \
             rust-toolchain.toml ({channel}) minus two minor versions"
        );
    }
}
