//! The changelog gate (spec §8). The config schema, the telemetry registry,
//! and the outcome JSON are lablet's public contract, so a change to any of
//! them must come with an entry under `## [Unreleased]` in `CHANGELOG.md`.
//!
//! The comparison runs from a base commit to the working tree, so it judges
//! what is committed on the branch and what is about to be. The decision is a
//! pure function of what git reports; only [`check`] talks to git.

use std::fmt::Write as _;

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

/// The changelog, relative to the repository root.
const CHANGELOG: &str = "CHANGELOG.md";

/// The heading of the section the gate reads.
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

/// Whether `path`, relative to the repository root, is part of the contract.
fn is_contract_path(path: &str) -> bool {
    CONTRACT_PATHS.iter().any(|contract| {
        if contract.ends_with('/') {
            path.starts_with(contract)
        } else {
            path == *contract
        }
    })
}

/// The body of the `## [Unreleased]` section: the lines after its heading, up
/// to the next second-level heading. `None` when there is no such section.
fn unreleased_section(changelog: &str) -> Option<String> {
    let mut lines = changelog.lines();
    lines.find(|line| line.trim_end().eq_ignore_ascii_case(UNRELEASED_HEADING))?;
    let body: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    Some(body.join("\n").trim().to_owned())
}

/// Whether a section body holds at least one list entry.
fn has_entry(section: &str) -> bool {
    section
        .lines()
        .any(|line| line.trim_start().starts_with(['-', '*']))
}

/// The decision, from what changed and the changelog at both ends.
/// `changed` is every path that differs between the base and the working
/// tree; the changelogs are whole files, `None` when the file does not exist.
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

/// Picks the base commit: `LABLET_CHANGELOG_BASE` when it is set and
/// resolves, else the merge-base with `origin/main`, else the merge-base with
/// `main`, else `main` itself. `None` when nothing resolves, as in a shallow
/// clone. An all-zero id, which GitHub sends for a new branch, counts as unset.
/// `warnings` collects what was skipped and why.
pub fn resolve_base(
    from_environment: Option<&str>,
    resolves: impl Fn(&str) -> bool,
    merge_base: impl Fn(&str) -> Option<String>,
    warnings: &mut Vec<String>,
) -> Option<Base> {
    let named = from_environment
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.bytes().all(|b| b == b'0'));
    if let Some(name) = named {
        if resolves(name) {
            return Some(Base {
                revision: name.to_owned(),
                source: BASE_VARIABLE.to_owned(),
            });
        }
        warnings.push(format!(
            "{BASE_VARIABLE}={name} does not name a commit in this clone; falling back to main"
        ));
    }
    for branch in ["origin/main", "main"] {
        if let Some(revision) = merge_base(branch) {
            return Some(Base {
                revision,
                source: format!("merge-base with {branch}"),
            });
        }
    }
    resolves("main").then(|| Base {
        revision: "main".to_owned(),
        source: "main".to_owned(),
    })
}

/// The gate: gathers the inputs from git and the working tree, decides, and
/// words the outcome.
pub fn check() -> CheckResult {
    let mut warnings = Vec::new();
    let from_environment = std::env::var(BASE_VARIABLE).ok();
    let base = resolve_base(
        from_environment.as_deref(),
        |revision| {
            let commit = format!("{revision}^{{commit}}");
            git(&["rev-parse", "--verify", "--quiet", &commit]).is_ok()
        },
        |branch| {
            git(&["merge-base", "HEAD", branch])
                .ok()
                .map(|out| out.trim().to_owned())
                .filter(|revision| !revision.is_empty())
        },
        &mut warnings,
    );
    for warning in &warnings {
        eprintln!("warning: changelog: {warning}");
    }
    let Some(base) = base else {
        eprintln!(
            "warning: changelog: no base commit could be resolved (no origin/main and no main; \
             a shallow clone?), so the gate cannot compare and passes. CI must check out full \
             history, or set {BASE_VARIABLE}."
        );
        return Ok(Some("skipped: no base commit to compare with".to_owned()));
    };

    let mut changed = lines(&git(&["diff", "--name-only", &base.revision, "--"])?);
    // Files git does not track yet differ from the base too.
    changed.extend(lines(&git(&[
        "ls-files",
        "--others",
        "--exclude-standard",
    ])?));
    let at_base = git(&["show", &format!("{}:{CHANGELOG}", base.revision)]).ok();
    let now = std::fs::read_to_string(repo_root().join(CHANGELOG)).ok();

    let since = format!("{} ({})", short(&base.revision), base.source);
    match decide(&changed, at_base.as_deref(), now.as_deref()) {
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

/// The failure diagnostic: what changed, what is missing, what to do.
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

/// Runs git in the repository root and returns its stdout.
fn git(args: &[&str]) -> Result<String, String> {
    process::capture("git", args)
}

fn lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
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

    // --- Contract paths ---

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

    // --- The Unreleased section ---

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

    // --- The decision ---

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

    // --- The base ---

    fn base(
        environment: Option<&str>,
        known: &[&str],
        merge_bases: &[(&str, &str)],
    ) -> (Option<Base>, Vec<String>) {
        let mut warnings = Vec::new();
        let base = resolve_base(
            environment,
            |revision| known.contains(&revision),
            |branch| {
                merge_bases
                    .iter()
                    .find(|(name, _)| *name == branch)
                    .map(|(_, revision)| (*revision).to_owned())
            },
            &mut warnings,
        );
        (base, warnings)
    }

    #[test]
    fn the_environment_names_the_base_when_it_resolves() {
        let (found, warnings) = base(Some("abc123"), &["abc123"], &[("origin/main", "def456")]);
        assert_eq!(found.unwrap().revision, "abc123");
        assert!(warnings.is_empty());
    }

    #[test]
    fn an_unresolvable_or_all_zero_environment_base_falls_back_to_main() {
        let (found, warnings) = base(Some("gone"), &[], &[("origin/main", "def456")]);
        assert_eq!(found.unwrap().revision, "def456");
        assert_eq!(warnings.len(), 1);

        let zeros = "0".repeat(40);
        let (found, warnings) = base(Some(&zeros), &[], &[("origin/main", "def456")]);
        assert_eq!(found.unwrap().source, "merge-base with origin/main");
        assert!(warnings.is_empty());
    }

    #[test]
    fn origin_main_is_preferred_to_the_local_main() {
        let (found, _) = base(None, &["main"], &[("main", "111"), ("origin/main", "222")]);
        assert_eq!(found.unwrap().revision, "222");
        let (found, _) = base(None, &["main"], &[("main", "111")]);
        assert_eq!(found.unwrap().revision, "111");
    }

    #[test]
    fn main_itself_is_the_last_resort_and_nothing_at_all_is_none() {
        let (found, _) = base(None, &["main"], &[]);
        assert_eq!(
            found,
            Some(Base {
                revision: "main".to_owned(),
                source: "main".to_owned()
            })
        );
        let (found, warnings) = base(Some(""), &[], &[]);
        assert_eq!(found, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_full_commit_id_is_shortened_and_a_ref_name_is_not() {
        assert_eq!(short(&"a1".repeat(20)), "a1a1a1a1a1a1");
        assert_eq!(short("origin/main"), "origin/main");
    }
}
