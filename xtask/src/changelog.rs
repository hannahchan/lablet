//! The changelog gate (spec §8). The config schema, the telemetry registry,
//! and the outcome JSON are lablet's public contract, so a change to any of
//! them must come with an entry under `## [Unreleased]` in `CHANGELOG.md`.
//! The comparison runs from a base commit to the working tree, so it judges
//! what is committed on the branch and what is about to be.

use std::fmt::Write as _;
use std::path::Path;

use crate::gates::CheckResult;
use crate::process;
use crate::workspace::repo_root;

/// The contract files, relative to the repository root. An entry ending in
/// `/` is a directory and covers everything under it.
const CONTRACT_PATHS: [&str; 3] = [
    "lablet/schema.json",
    "lablet/telemetry/registry/",
    "lablet/tests/fixtures/outcome.json",
];

const CHANGELOG: &str = "CHANGELOG.md";
const UNRELEASED_HEADING: &str = "## [Unreleased]";

/// Names the base commit outright, for CI events where the merge-base with
/// `origin/main` is the pushed commit itself.
const BASE_VARIABLE: &str = "LABLET_CHANGELOG_BASE";

/// What the gate concluded.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// No contract file differs from the base.
    NoContractChange,
    /// Contract files changed and `Unreleased` gained or changed an entry.
    Recorded(Vec<String>),
    /// Contract files changed and `Unreleased` is as it was at the base.
    Unchanged(Vec<String>),
    /// Contract files changed and `Unreleased` has no entry at all, or the
    /// changelog has no such section.
    Empty(Vec<String>),
}

fn is_contract_path(path: &str) -> bool {
    CONTRACT_PATHS.iter().any(|contract| {
        if contract.ends_with('/') {
            path.starts_with(contract)
        } else {
            path == *contract
        }
    })
}

/// The lines after the heading, up to the next second-level heading. `None`
/// when there is no such section.
fn unreleased_section(changelog: &str) -> Option<String> {
    let mut lines = changelog.lines();
    lines.find(|line| line.trim_end().eq_ignore_ascii_case(UNRELEASED_HEADING))?;
    let body: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    Some(body.join("\n").trim().to_owned())
}

fn has_entry(section: &str) -> bool {
    section
        .lines()
        .any(|line| line.trim_start().starts_with(['-', '*']))
}

/// `changed` is every path that differs between the base and the working
/// tree; a changelog is the whole file, `None` when it does not exist.
pub fn decide(
    changed: &[String],
    changelog_at_base: Option<&str>,
    changelog_now: Option<&str>,
) -> Verdict {
    let mut contract: Vec<String> = changed
        .iter()
        .filter(|path| is_contract_path(path))
        .cloned()
        .collect();
    contract.sort();
    contract.dedup();
    if contract.is_empty() {
        return Verdict::NoContractChange;
    }
    let before = changelog_at_base.and_then(unreleased_section);
    let now = changelog_now.and_then(unreleased_section);
    match now {
        Some(now) if !has_entry(&now) => Verdict::Empty(contract),
        None => Verdict::Empty(contract),
        Some(now) if before.as_deref() == Some(now.as_str()) => Verdict::Unchanged(contract),
        Some(_) => Verdict::Recorded(contract),
    }
}

/// Where the base commit came from, for the step's note.
#[derive(Debug, PartialEq, Eq)]
pub struct Base {
    /// The commit, as a revision git resolves.
    pub revision: String,
    /// How it was chosen.
    pub source: String,
}

/// `LABLET_CHANGELOG_BASE` when it names a commit, else the merge-base with
/// `origin/main`, else with `main`. An all-zero id, which GitHub sends for a
/// new branch, counts as unset. The error is the note for a run that can
/// compare nothing: spec §8 has an unresolvable base warn and pass.
pub fn resolve_base(
    from_environment: Option<&str>,
    resolves: impl Fn(&str) -> bool,
    merge_base: impl Fn(&str) -> Option<String>,
) -> Result<Base, String> {
    let named = from_environment
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.bytes().all(|b| b == b'0'));
    if let Some(name) = named.filter(|name| resolves(name)) {
        return Ok(Base {
            revision: name.to_owned(),
            source: BASE_VARIABLE.to_owned(),
        });
    }
    for branch in ["origin/main", "main"] {
        if let Some(revision) = merge_base(branch) {
            return Ok(Base {
                revision,
                source: format!("merge-base with {branch}"),
            });
        }
    }
    Err(format!(
        "warning: skipped, no base commit to compare with (no merge-base with origin/main or \
         main; a shallow clone?). CI must check out full history, or set {BASE_VARIABLE}"
    ))
}

/// Every path that differs between `base` and the working tree, untracked
/// files included. Rename detection is off: with it git names only the new
/// path, and a contract file moved out of the contract is exactly the change
/// the gate is for. `-z` keeps git from quoting an unusual name, which would
/// no longer start with a contract path.
fn changed_paths(directory: &Path, base: &str) -> Result<Vec<String>, String> {
    let git = |args: &[&str]| process::capture_in(directory, "git", args);
    let mut changed = nul_separated(&git(&[
        "diff",
        "--name-only",
        "--no-renames",
        "-z",
        base,
        "--",
    ])?);
    changed.extend(nul_separated(&git(&[
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
    ])?));
    Ok(changed)
}

/// The changelog gate as a step.
pub fn check() -> CheckResult {
    let commit = |revision: &str| {
        let commit = format!("{revision}^{{commit}}");
        git(&["rev-parse", "--verify", "--quiet", &commit])
            .ok()
            .map(|out| out.trim().to_owned())
    };
    let from_environment = std::env::var(BASE_VARIABLE).ok();
    let base = resolve_base(
        from_environment.as_deref(),
        |revision| commit(revision).is_some(),
        |branch| {
            git(&["merge-base", "HEAD", branch])
                .ok()
                .map(|out| out.trim().to_owned())
                .filter(|revision| !revision.is_empty())
        },
    );
    let base = match base {
        Ok(base) => base,
        Err(skipped) => return Ok(Some(skipped)),
    };

    let changed = changed_paths(&repo_root(), &base.revision)?;
    let at_base = git(&["show", &format!("{}:{CHANGELOG}", base.revision)]).ok();
    let now = std::fs::read_to_string(repo_root().join(CHANGELOG)).ok();

    let since = format!("{} ({})", short(&base.revision), base.source);
    match decide(&changed, at_base.as_deref(), now.as_deref()) {
        // On `main` without LABLET_CHANGELOG_BASE the base is the commit being
        // judged: only the working tree was compared, and the note says so.
        Verdict::NoContractChange if commit(&base.revision) == commit("HEAD") => Ok(Some(format!(
            "no contract file changed in the working tree; the base {since} is HEAD, so no \
             commit was compared"
        ))),
        Verdict::NoContractChange => Ok(Some(format!("no contract file changed since {since}"))),
        Verdict::Recorded(paths) => Ok(Some(format!(
            "{} contract file(s) changed since {since}, with an Unreleased entry",
            paths.len()
        ))),
        Verdict::Unchanged(paths) => Err(failure(
            &paths,
            &since,
            &format!("the `{UNRELEASED_HEADING}` section of {CHANGELOG} has not changed"),
        )),
        Verdict::Empty(paths) => Err(failure(
            &paths,
            &since,
            &format!("{CHANGELOG} has no entry under `{UNRELEASED_HEADING}`"),
        )),
    }
}

fn failure(paths: &[String], since: &str, missing: &str) -> String {
    let mut message = format!("Contract files changed since {since}, but {missing}:\n\n");
    for path in paths {
        let _ = writeln!(message, "  {path}");
    }
    let _ = write!(
        message,
        "\nThe config schema, the telemetry registry, and the outcome JSON are lablet's public \
         contract (spec §8).\nAdd an entry under `{UNRELEASED_HEADING}` in {CHANGELOG} that says \
         what changed for users."
    );
    message
}

fn git(args: &[&str]) -> Result<String, String> {
    process::capture("git", args)
}

fn nul_separated(output: &str) -> Vec<String> {
    output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A full commit id cut to what a person reads; a ref name is left alone.
fn short(revision: &str) -> &str {
    if revision.len() == 40 && revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        &revision[..12]
    } else {
        revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- The scaffold.\n\n## [0.1.0] - 2026-01-01\n\n- First.\n";

    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn the_contract_is_the_schema_the_registry_tree_and_the_outcome_fixture() {
        for path in [
            "lablet/schema.json",
            "lablet/telemetry/registry/manifest.yaml",
            "lablet/telemetry/registry/spans/chat.yaml",
            "lablet/tests/fixtures/outcome.json",
        ] {
            assert!(is_contract_path(path), "{path}");
        }
        for path in [
            "lablet/schema.json.bak",
            "lablet/telemetry/registry.yaml",
            "lablet/telemetry/deps/semconv/model/http.yaml",
            "lablet/telemetry/templates/registry/rust/weaver.yaml",
            "lablet/tests/fixtures/outcome.json/nested",
            "lablet/tests/fixtures/other.json",
            "schema.json",
            "CHANGELOG.md",
        ] {
            assert!(!is_contract_path(path), "{path}");
        }
    }

    #[test]
    fn the_unreleased_section_ends_at_the_next_release_heading() {
        assert_eq!(
            unreleased_section(BEFORE).as_deref(),
            Some("### Added\n\n- The scaffold.")
        );
    }

    #[test]
    fn a_changelog_without_the_heading_has_no_unreleased_section() {
        assert_eq!(unreleased_section("# Changelog\n\n## [0.1.0]\n- x\n"), None);
    }

    #[test]
    fn an_unreleased_section_at_the_end_of_the_file_is_read_whole() {
        assert_eq!(
            unreleased_section("## [Unreleased]\n- a\n- b\n").as_deref(),
            Some("- a\n- b")
        );
    }

    #[test]
    fn no_contract_change_passes_whatever_the_changelog_says() {
        let changed = paths(&["lablet/crates/domain/model/src/lib.rs", "README.md"]);
        assert_eq!(decide(&changed, None, None), Verdict::NoContractChange);
        assert_eq!(
            decide(&[], Some(BEFORE), Some(BEFORE)),
            Verdict::NoContractChange
        );
    }

    #[test]
    fn a_contract_change_with_an_untouched_unreleased_section_fails() {
        let changed = paths(&["lablet/schema.json", "lablet/src/main.rs"]);
        assert_eq!(
            decide(&changed, Some(BEFORE), Some(BEFORE)),
            Verdict::Unchanged(paths(&["lablet/schema.json"]))
        );
    }

    #[test]
    fn a_change_elsewhere_in_the_changelog_does_not_count() {
        let changed = paths(&["lablet/telemetry/registry/spans.yaml"]);
        let now = BEFORE.replace("- First.", "- First, reworded.");
        assert_eq!(
            decide(&changed, Some(BEFORE), Some(&now)),
            Verdict::Unchanged(paths(&["lablet/telemetry/registry/spans.yaml"]))
        );
    }

    #[test]
    fn a_contract_change_with_a_new_unreleased_entry_passes() {
        let changed = paths(&[
            "lablet/tests/fixtures/outcome.json",
            "lablet/schema.json",
            "lablet/schema.json",
        ]);
        let now = BEFORE.replace(
            "- The scaffold.",
            "- The scaffold.\n- `stop_reason` gained `cancelled`.",
        );
        assert_eq!(
            decide(&changed, Some(BEFORE), Some(&now)),
            Verdict::Recorded(paths(&[
                "lablet/schema.json",
                "lablet/tests/fixtures/outcome.json"
            ]))
        );
    }

    #[test]
    fn a_first_changelog_with_an_entry_passes() {
        let changed = paths(&["lablet/schema.json"]);
        assert_eq!(
            decide(&changed, None, Some(BEFORE)),
            Verdict::Recorded(paths(&["lablet/schema.json"]))
        );
    }

    #[test]
    fn a_contract_change_with_an_empty_unreleased_section_fails() {
        let changed = paths(&["lablet/schema.json"]);
        let emptied = "# Changelog\n\n## [Unreleased]\n\n## [0.2.0]\n\n- The scaffold.\n";
        assert_eq!(
            decide(&changed, Some(BEFORE), Some(emptied)),
            Verdict::Empty(paths(&["lablet/schema.json"]))
        );
    }

    #[test]
    fn a_contract_change_with_no_changelog_at_all_fails() {
        let changed = paths(&["lablet/schema.json"]);
        assert_eq!(
            decide(&changed, None, None),
            Verdict::Empty(paths(&["lablet/schema.json"]))
        );
        assert_eq!(
            decide(&changed, None, Some("# Changelog\n")),
            Verdict::Empty(paths(&["lablet/schema.json"]))
        );
    }

    fn base(
        environment: Option<&str>,
        known: &[&str],
        merge_bases: &[(&str, &str)],
    ) -> Result<Base, String> {
        resolve_base(
            environment,
            |revision| known.contains(&revision),
            |branch| {
                merge_bases
                    .iter()
                    .find(|(name, _)| *name == branch)
                    .map(|(_, revision)| (*revision).to_owned())
            },
        )
    }

    #[test]
    fn the_environment_names_the_base_when_it_names_a_commit() {
        let found = base(Some("abc123"), &["abc123"], &[("origin/main", "def456")]).unwrap();
        assert_eq!(found.revision, "abc123");
        assert_eq!(found.source, "LABLET_CHANGELOG_BASE");
        let zeros = "0".repeat(40);
        for unusable in [None, Some(""), Some(zeros.as_str()), Some("gone")] {
            let found = base(unusable, &["abc123"], &[("origin/main", "def456")]).unwrap();
            assert_eq!(found.revision, "def456");
            assert_eq!(found.source, "merge-base with origin/main");
        }
    }

    #[test]
    fn origin_main_is_preferred_to_the_local_main_and_no_merge_base_is_a_skip_note() {
        let found = base(None, &[], &[("main", "111"), ("origin/main", "222")]).unwrap();
        assert_eq!(found.revision, "222");
        let found = base(None, &[], &[("main", "111")]).unwrap();
        assert_eq!(found.revision, "111");
        let skipped = base(None, &["main"], &[]).unwrap_err();
        assert!(
            skipped.starts_with("warning: skipped, no base commit to compare with"),
            "{skipped}"
        );
    }

    /// Git pinned to a scratch repository and cut off from the developer's own
    /// configuration. Clearing the inherited variables isn't enough on its own:
    /// under a hook, a command that missed that step would reach the real
    /// repository, so the repository is named outright.
    fn scratch_git(root: &std::path::Path, args: &[&str]) -> String {
        let mut command = std::process::Command::new("git");
        command
            .args([
                "-c",
                "user.name=xtask",
                "-c",
                "user.email=x@example.invalid",
            ])
            .args(args)
            .current_dir(root);
        for variable in process::GIT_REPOSITORY_ENV {
            command.env_remove(variable);
        }
        let output = command
            .env("GIT_DIR", root.join(".git"))
            .env("GIT_WORK_TREE", root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn a_contract_file_moved_out_of_the_contract_or_oddly_named_is_still_reported() {
        let dir = crate::workspace::fixture::TempDir::new("changelog-git");
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        let registry = "lablet/telemetry/registry/spans";
        let chat = format!("{registry}/chat.yaml");
        dir.write(&chat, "groups: []\n");
        git(&["init", "--quiet", "--initial-branch=main"]);
        let resolved = git(&["rev-parse", "--show-toplevel"]);
        assert_eq!(
            std::fs::canonicalize(resolved.trim()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=base"]);
        // A span retired from the registry: git sees a rename and, with
        // rename detection on, would name only the new path.
        git(&["mv", &chat, "lablet/telemetry/retired-chat.yaml"]);
        git(&["commit", "--quiet", "--message=retire"]);
        // An untracked name git would quote without `-z`.
        dir.write(&format!("{registry}/a\"b.yaml"), "groups: []\n");

        let changed = changed_paths(dir.path(), "HEAD~1").unwrap();
        let contract: Vec<&str> = changed
            .iter()
            .map(String::as_str)
            .filter(|path| is_contract_path(path))
            .collect();
        assert_eq!(
            contract,
            [
                "lablet/telemetry/registry/spans/chat.yaml",
                "lablet/telemetry/registry/spans/a\"b.yaml"
            ]
        );
    }

    #[test]
    fn a_full_commit_id_is_shortened_and_a_ref_name_is_not() {
        assert_eq!(short(&"a1".repeat(20)), "a1a1a1a1a1a1");
        assert_eq!(short("origin/main"), "origin/main");
    }
}
