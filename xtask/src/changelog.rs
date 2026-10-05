//! The changelog gate (spec §8). The config schema, the telemetry registry,
//! and the outcome JSON are lablet's public contract, the templates that
//! render the registry into each crate's module decide what every signal
//! records, the transcript JSON is what a grader parses, and the golden
//! telemetry fixtures are what a run emits, so a change to any of them must
//! come with an entry under
//! `## [Unreleased]` in `CHANGELOG.md`. The comparison runs from
//! a base commit to the working tree, so it judges what is committed on the
//! branch and what is about to be.

use std::fmt::Write as _;
use std::path::Path;

use crate::error::{Error, Verb};
use crate::gates::{CheckResult, Failure};
use crate::process;
use crate::report::Note;
use crate::workspace::repo_root;

/// The contract files, relative to the repository root. An entry ending in
/// `/` is a directory and covers everything under it.
const CONTRACT_PATHS: [&str; 6] = [
    "lablet/schema.json",
    "lablet/telemetry/registry/",
    "lablet/telemetry/templates/registry/rust-crate/",
    "lablet/tests/fixtures/outcome.json",
    "lablet/tests/fixtures/transcript.json",
    "lablet/tests/fixtures/golden/",
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
/// new branch, counts as unset. `None` is git answering that there is no
/// base, a run that can compare nothing, which spec §8 has warn and pass:
/// see [`skipped`]. Git failing to answer is an error, never that pass.
pub fn resolve_base(
    from_environment: Option<&str>,
    resolves: impl Fn(&str) -> Result<bool, Error>,
    merge_base: impl Fn(&str) -> Result<Option<String>, Error>,
) -> Result<Option<Base>, Error> {
    let named = from_environment
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.bytes().all(|b| b == b'0'));
    if let Some(name) = named
        && resolves(name)?
    {
        return Ok(Some(Base {
            revision: name.to_owned(),
            source: BASE_VARIABLE.to_owned(),
        }));
    }
    for branch in ["origin/main", "main"] {
        if let Some(revision) = merge_base(branch)? {
            return Ok(Some(Base {
                revision,
                source: format!("merge-base with {branch}"),
            }));
        }
    }
    Ok(None)
}

/// The note of a run that found no base to compare with.
fn skipped() -> Note {
    Note::Warning(format!(
        "skipped, no base commit to compare with (no merge-base with origin/main or main; a \
         shallow clone?). CI must check out full history, or set {BASE_VARIABLE}"
    ))
}

/// Every path whose content differs between `base` and the working tree,
/// untracked files included. Rename detection is off: with it git names only
/// the new path, and a contract file moved out of the contract is exactly the
/// change the gate is for. `-z` keeps git from quoting an unusual name, which
/// would no longer start with a contract path.
fn changed_paths(directory: &Path, base: &str) -> Result<Vec<String>, Error> {
    let git = |args: &[&str]| process::capture_in(directory, "git", args);
    // A diff against a commit refreshes the index's stat data unless told
    // not to, which writes to the repository a gate only reads. Told not to,
    // `--name-only` lists a file whose timestamp moved and whose content
    // didn't, where `--numstat` compares the content: each record is
    // `added\tdeleted\tpath`, and the path is kept whole whatever it holds.
    let numstat = git(&[
        "-c",
        "diff.autoRefreshIndex=false",
        "diff",
        "--numstat",
        "--no-renames",
        "-z",
        base,
        "--",
    ])?;
    let mut changed: Vec<String> = nul_separated(&numstat)
        .iter()
        .filter_map(|record| record.splitn(3, '\t').nth(2))
        .map(str::to_owned)
        .collect();
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
    let from_environment = std::env::var(BASE_VARIABLE).ok();
    check_in(&repo_root(), from_environment.as_deref())
}

/// [`check`], for the repository at `root`, with `from_environment` read
/// from [`BASE_VARIABLE`].
fn check_in(root: &Path, from_environment: Option<&str>) -> CheckResult {
    let commit = |revision: &str| process::git_commit(root, revision);
    let base = resolve_base(
        from_environment,
        |revision| Ok(commit(revision)?.is_some()),
        |branch| process::git_merge_base(root, branch),
    )?;
    let Some(base) = base else {
        return Ok(Some(skipped()));
    };

    let changed = changed_paths(root, &base.revision)?;
    let at_base = process::git_file_at(root, &base.revision, CHANGELOG)?;
    let path = root.join(CHANGELOG);
    let now = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::file(Verb::Read, &path)(e).into()),
    };
    let verdict = decide(&changed, at_base.as_deref(), now.as_deref());
    conclude(verdict, &base, commit(&base.revision)? == commit("HEAD")?)
}

/// The step's result for a verdict against `base`. `base_is_head` is whether
/// the base is the commit being judged.
fn conclude(verdict: Verdict, base: &Base, base_is_head: bool) -> CheckResult {
    let since = format!("{} ({})", short(&base.revision), base.source);
    match verdict {
        // On `main` without LABLET_CHANGELOG_BASE the base is the commit being
        // judged: only the working tree was compared, and the note says so.
        Verdict::NoContractChange if base_is_head => Ok(Some(Note::Info(format!(
            "no contract file changed in the working tree; the base {since} is HEAD, so no \
             commit was compared"
        )))),
        Verdict::NoContractChange => Ok(Some(Note::Info(format!(
            "no contract file changed since {since}"
        )))),
        Verdict::Recorded(paths) => Ok(Some(Note::Info(format!(
            "{} contract file(s) changed since {since}, with an Unreleased entry",
            paths.len()
        )))),
        Verdict::Unchanged(paths) => Err(Failure::Verdict(failure(
            &paths,
            &since,
            &format!("the `{UNRELEASED_HEADING}` section of {CHANGELOG} has not changed"),
        ))),
        Verdict::Empty(paths) => Err(Failure::Verdict(failure(
            &paths,
            &since,
            &format!("{CHANGELOG} has no entry under `{UNRELEASED_HEADING}`"),
        ))),
    }
}

fn failure(paths: &[String], since: &str, missing: &str) -> String {
    let mut message = format!("Contract files changed since {since}, but {missing}:\n\n");
    for path in paths {
        let _ = writeln!(message, "  {path}");
    }
    let _ = write!(
        message,
        "\nThe config schema, the telemetry registry, and the outcome and transcript JSON are \
         what lablet's users parse, and the golden telemetry is what a run emits (spec §8).\nAdd \
         an entry under `{UNRELEASED_HEADING}` in {CHANGELOG} that says what changed for users."
    );
    message
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
    use crate::workspace::fixture::{TempDir, defy_git_defaults, scratch_git};

    const BEFORE: &str = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- The scaffold.\n\n## [0.1.0] - 2026-01-01\n\n- First.\n";

    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn the_contract_is_the_schema_the_registry_and_crate_template_trees_the_two_document_fixtures_and_the_golden_tree()
     {
        for path in [
            "lablet/schema.json",
            "lablet/telemetry/registry/manifest.yaml",
            "lablet/telemetry/registry/spans/chat.yaml",
            "lablet/telemetry/templates/registry/rust-crate/weaver.yaml",
            "lablet/telemetry/templates/registry/rust-crate/spans.rs.j2",
            "lablet/tests/fixtures/outcome.json",
            "lablet/tests/fixtures/transcript.json",
            "lablet/tests/fixtures/golden/starter/expected.json",
            "lablet/tests/fixtures/golden/cancelled/work/notes.md",
        ] {
            assert!(is_contract_path(path), "{path}");
        }
        for path in [
            "lablet/schema.json.bak",
            "lablet/telemetry/registry.yaml",
            "lablet/telemetry/deps/semconv/model/http.yaml",
            "lablet/telemetry/templates/registry/rust/weaver.yaml",
            "lablet/tests/fixtures/outcome.json/nested",
            "lablet/tests/fixtures/transcript.json.bak",
            "lablet/tests/fixtures/other.json",
            "lablet/tests/fixtures/golden.json",
            "lablet/tests/fixtures/golden-notes.md",
            "lablet/crates/adapters/secondary/transcript-json/src/lib.rs",
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

    /// A base the merge-base chose, as a full commit id.
    fn merge_base() -> Base {
        Base {
            revision: "a1".repeat(20),
            source: "merge-base with origin/main".to_owned(),
        }
    }

    #[test]
    fn a_contract_change_the_changelog_does_not_record_fails_the_step() {
        let schema = || paths(&["lablet/schema.json"]);
        let why = "\n\n  lablet/schema.json\n\nThe config schema, the telemetry registry, and the \
                   outcome and transcript JSON are what lablet's users parse, and the golden \
                   telemetry is what a run emits (spec §8).\nAdd an entry under `## [Unreleased]` \
                   in CHANGELOG.md that says what changed for users.";
        // Whether the base is HEAD changes only the note of a pass.
        for base_is_head in [false, true] {
            assert_eq!(
                conclude(Verdict::Unchanged(schema()), &merge_base(), base_is_head)
                    .unwrap_err()
                    .into_verdict(),
                format!(
                    "Contract files changed since a1a1a1a1a1a1 (merge-base with origin/main), but \
                     the `## [Unreleased]` section of CHANGELOG.md has not changed:{why}"
                )
            );
            assert_eq!(
                conclude(Verdict::Empty(schema()), &merge_base(), base_is_head)
                    .unwrap_err()
                    .into_verdict(),
                format!(
                    "Contract files changed since a1a1a1a1a1a1 (merge-base with origin/main), but \
                     CHANGELOG.md has no entry under `## [Unreleased]`:{why}"
                )
            );
        }
    }

    #[test]
    fn a_recorded_change_or_none_passes_with_a_note_naming_the_base() {
        let recorded = paths(&["lablet/schema.json", "lablet/tests/fixtures/outcome.json"]);
        assert_eq!(
            conclude(Verdict::Recorded(recorded), &merge_base(), false).unwrap(),
            Some(Note::Info(
                "2 contract file(s) changed since a1a1a1a1a1a1 (merge-base with origin/main), \
                 with an Unreleased entry"
                    .to_owned()
            ))
        );
        assert_eq!(
            conclude(Verdict::NoContractChange, &merge_base(), false).unwrap(),
            Some(Note::Info(
                "no contract file changed since a1a1a1a1a1a1 (merge-base with origin/main)"
                    .to_owned()
            ))
        );
        // On `main`, whose merge-base with origin/main is the commit itself.
        assert_eq!(
            conclude(Verdict::NoContractChange, &merge_base(), true).unwrap(),
            Some(Note::Info(
                "no contract file changed in the working tree; the base a1a1a1a1a1a1 (merge-base \
                 with origin/main) is HEAD, so no commit was compared"
                    .to_owned()
            ))
        );
    }

    fn base(
        environment: Option<&str>,
        known: &[&str],
        merge_bases: &[(&str, &str)],
    ) -> Option<Base> {
        resolve_base(
            environment,
            |revision| Ok(known.contains(&revision)),
            |branch| {
                Ok(merge_bases
                    .iter()
                    .find(|(name, _)| *name == branch)
                    .map(|(_, revision)| (*revision).to_owned()))
            },
        )
        .unwrap()
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
        // The all-zero id is unset by what it is, not by failing to resolve.
        let found = base(Some(&zeros), &[&zeros], &[("origin/main", "def456")]).unwrap();
        assert_eq!(found.revision, "def456");
    }

    #[test]
    fn origin_main_is_preferred_to_the_local_main_and_no_merge_base_is_none() {
        let found = base(None, &[], &[("main", "111"), ("origin/main", "222")]).unwrap();
        assert_eq!(found.revision, "222");
        let found = base(None, &[], &[("main", "111")]).unwrap();
        assert_eq!(found.revision, "111");
        assert_eq!(base(None, &["main"], &[]), None);
        assert_eq!(base(Some("gone"), &["main"], &[]), None);
    }

    fn git_did_not_answer() -> Error {
        Error::Missing {
            what: "git did not answer".to_owned(),
            remedy: "Install git".to_owned(),
        }
    }

    #[test]
    fn git_failing_to_answer_is_an_error_and_not_a_missing_base() {
        let unanswered = resolve_base(Some("abc123"), |_| Err(git_did_not_answer()), |_| Ok(None));
        assert!(
            matches!(unanswered, Err(Error::Missing { .. })),
            "{unanswered:?}"
        );
        let unanswered = resolve_base(None, |_| Ok(true), |_| Err(git_did_not_answer()));
        assert!(
            matches!(unanswered, Err(Error::Missing { .. })),
            "{unanswered:?}"
        );
    }

    #[test]
    fn a_directory_that_is_not_a_repository_fails_the_step_rather_than_skipping_it() {
        let dir = TempDir::new("changelog-not-a-repository");
        // A `.git` file naming no repository stops git's search upward, so
        // what lies above the temporary directory doesn't matter.
        dir.write(
            ".git",
            &format!("gitdir: {}\n", dir.path().join("none").display()),
        );
        for from_environment in [None, Some("abc123")] {
            let error = check_in(dir.path(), from_environment)
                .unwrap_err()
                .into_error();
            let Error::Failed { command, stderr } = &error else {
                panic!("{error:?}");
            };
            assert!(
                command.to_string().starts_with("git rev-parse"),
                "{command}"
            );
            assert!(stderr.contains("not a git repository"), "{stderr}");
        }

        let gone = dir.path().join("gone");
        let error = check_in(&gone, None).unwrap_err().into_error();
        assert!(matches!(error, Error::Start { .. }), "{error:?}");
    }

    /// Spec §8: a base that can't be found warns, and the gate passes.
    #[test]
    fn a_repository_with_no_base_to_compare_with_passes_with_a_warning() {
        let dir = TempDir::new("changelog-no-base");
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        dir.write("lablet/schema.json", "{}\n");
        git(&["init", "--quiet", "--initial-branch=topic"]);
        assert_eq!(
            std::fs::canonicalize(git(&["rev-parse", "--show-toplevel"]).trim()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=only"]);
        let note = check_in(dir.path(), None).unwrap();
        assert_eq!(
            note,
            Some(Note::Warning(
                "skipped, no base commit to compare with (no merge-base with origin/main or main; \
                 a shallow clone?). CI must check out full history, or set LABLET_CHANGELOG_BASE"
                    .to_owned()
            ))
        );
        assert!(
            note.is_some_and(|note| note.to_string().starts_with("warning: skipped, ")),
            "the warning is told as one"
        );
    }

    #[test]
    fn a_changelog_that_is_there_but_cannot_be_read_is_an_error_naming_it() {
        let dir = TempDir::new("changelog-unreadable");
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        dir.write("lablet/schema.json", "{}\n");
        git(&["init", "--quiet", "--initial-branch=main"]);
        assert_eq!(
            std::fs::canonicalize(git(&["rev-parse", "--show-toplevel"]).trim()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=base"]);
        git(&["switch", "--quiet", "--create", "topic"]);
        dir.write("lablet/schema.json", "{\"type\": \"object\"}\n");
        // A directory where the file goes: it exists, and reads as none.
        std::fs::create_dir(dir.path().join(CHANGELOG)).unwrap();
        let error = check_in(dir.path(), None).unwrap_err().into_error();
        let path = dir.path().join(CHANGELOG);
        assert!(
            matches!(&error, Error::File { verb: Verb::Read, path: named, .. } if *named == path),
            "{error:?}"
        );
    }

    #[test]
    fn a_contract_file_moved_out_of_the_contract_or_oddly_named_is_still_reported() {
        let dir = crate::workspace::fixture::TempDir::new("changelog-git");
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        let registry = "lablet/telemetry/registry/spans";
        let chat = format!("{registry}/chat.yaml");
        dir.write(&chat, "groups: []\n");
        // A tab in a name, which git's record separates its fields with.
        let tabbed = format!("{registry}/a\tb.yaml");
        dir.write(&tabbed, "groups: []\n");
        git(&["init", "--quiet", "--initial-branch=main"]);
        defy_git_defaults(dir.path(), "diff.noprefix");
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
        dir.write(&tabbed, "groups: [one]\n");
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
                "lablet/telemetry/registry/spans/a\tb.yaml",
                "lablet/telemetry/registry/spans/chat.yaml",
                "lablet/telemetry/registry/spans/a\"b.yaml"
            ]
        );
    }

    #[test]
    fn a_full_commit_id_is_shortened_and_a_ref_name_is_not() {
        assert_eq!(short(&"a1".repeat(20)), "a1a1a1a1a1a1");
        assert_eq!(short("origin/main"), "origin/main");
        // Only the two together are a full id: its length, and hex digits.
        let long_name = format!("topic/{}", "x".repeat(34));
        assert_eq!(short(&long_name), long_name);
        assert_eq!(short("abc123"), "abc123");
    }

    #[test]
    fn the_gate_judges_the_working_tree_against_the_base_it_resolved() {
        let dir = TempDir::new("changelog-gate");
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        dir.write(CHANGELOG, BEFORE);
        dir.write("lablet/schema.json", "{}\n");
        git(&["init", "--quiet", "--initial-branch=main"]);
        defy_git_defaults(dir.path(), "diff.noprefix");
        assert_eq!(
            std::fs::canonicalize(git(&["rev-parse", "--show-toplevel"])).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=base"]);
        let base = git(&["rev-parse", "--short=12", "HEAD"]);

        // On `main` the merge-base is the commit being judged.
        assert_eq!(
            check_in(dir.path(), None).unwrap(),
            Some(Note::Info(format!(
                "no contract file changed in the working tree; the base {base} (merge-base with \
                 main) is HEAD, so no commit was compared"
            )))
        );

        git(&["switch", "--quiet", "--create", "topic"]);
        dir.write("lablet/schema.json", "{\"type\": \"object\"}\n");
        git(&["commit", "--quiet", "--all", "--message=schema"]);
        let error = check_in(dir.path(), None).unwrap_err().into_verdict();
        let unchanged = format!(
            "Contract files changed since {base} (merge-base with main), but the `## \
             [Unreleased]` section of CHANGELOG.md has not changed:\n\n  lablet/schema.json\n"
        );
        assert!(error.starts_with(&unchanged), "{error}");

        let recorded = BEFORE.replace("- The scaffold.", "- The scaffold.\n- A schema type.");
        dir.write(CHANGELOG, &recorded);
        assert_eq!(
            check_in(dir.path(), None).unwrap(),
            Some(Note::Info(format!(
                "1 contract file(s) changed since {base} (merge-base with main), with an \
                 Unreleased entry"
            )))
        );

        // The variable names a base that isn't HEAD.
        let named = git(&["rev-parse", "HEAD"]);
        git(&["commit", "--quiet", "--all", "--message=entry"]);
        assert_eq!(
            check_in(dir.path(), Some(&named)).unwrap(),
            Some(Note::Info(format!(
                "no contract file changed since {} (LABLET_CHANGELOG_BASE)",
                &named[..12]
            )))
        );
    }

    /// A scratch repository on `main` holding `files`, committed.
    fn committed_on_main(tag: &str, files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new(tag);
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        for (path, text) in files {
            dir.write(path, text);
        }
        git(&["init", "--quiet", "--initial-branch=main"]);
        defy_git_defaults(dir.path(), "diff.noprefix");
        assert_eq!(
            std::fs::canonicalize(git(&["rev-parse", "--show-toplevel"]).trim()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap(),
            "git resolved outside the scratch repository"
        );
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=base"]);
        dir
    }

    /// An editor saving an unchanged buffer, or an edit put back, leaves a
    /// newer timestamp and the same bytes. The gate only reads, so the index,
    /// whose stat data is then stale, is left as it was.
    #[test]
    fn a_contract_file_whose_timestamp_moved_and_whose_content_did_not_is_unchanged() {
        let files = [
            (CHANGELOG, BEFORE),
            ("lablet/schema.json", "{}\n"),
            ("README.md", "Lablet.\n"),
        ];
        let dir = committed_on_main("changelog-touched", &files);
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        let base = git(&["rev-parse", "--short=12", "HEAD"]);
        git(&["switch", "--quiet", "--create", "topic"]);
        dir.write("README.md", "Lablet, a loop.\n");
        git(&["commit", "--quiet", "--all", "--message=readme"]);
        let schema = std::fs::File::options()
            .write(true)
            .open(dir.path().join("lablet/schema.json"))
            .unwrap();
        let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        schema.set_modified(long_ago).unwrap();

        let index = || std::fs::read(dir.path().join(".git/index")).unwrap();
        let before = index();
        assert_eq!(
            check_in(dir.path(), None).unwrap(),
            Some(Note::Info(format!(
                "no contract file changed since {base} (merge-base with main)"
            )))
        );
        assert!(index() == before, "the changelog gate rewrote the index");
    }

    #[test]
    fn a_base_without_a_changelog_takes_a_first_entry_as_recorded() {
        let dir = committed_on_main("changelog-none-at-base", &[("lablet/schema.json", "{}\n")]);
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        let base = git(&["rev-parse", "--short=12", "HEAD"]);
        git(&["switch", "--quiet", "--create", "topic"]);
        dir.write("lablet/schema.json", "{\"type\": \"object\"}\n");
        dir.write(CHANGELOG, BEFORE);
        assert_eq!(
            check_in(dir.path(), None).unwrap(),
            Some(Note::Info(format!(
                "1 contract file(s) changed since {base} (merge-base with main), with an \
                 Unreleased entry"
            )))
        );
    }

    /// Read as no changelog at the base, an unchanged `Unreleased` section
    /// would pass as a first entry. A blob gone from the object store, as a
    /// partial clone offline may lack it, is git failing to read it.
    #[test]
    fn a_changelog_at_the_base_that_git_cannot_read_is_an_error_and_not_an_absent_one() {
        let files = [(CHANGELOG, BEFORE), ("lablet/schema.json", "{}\n")];
        let dir = committed_on_main("changelog-unread-at-base", &files);
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        let blob = git(&["rev-parse", &format!("HEAD:{CHANGELOG}")]);
        git(&["switch", "--quiet", "--create", "topic"]);
        dir.write("lablet/schema.json", "{\"type\": \"object\"}\n");
        git(&["commit", "--quiet", "--all", "--message=schema"]);
        let object = dir
            .path()
            .join(".git/objects")
            .join(&blob[..2])
            .join(&blob[2..]);
        std::fs::remove_file(&object).unwrap();

        let error = check_in(dir.path(), None).unwrap_err().into_error();
        let Error::Failed { command, .. } = &error else {
            panic!("{error:?}");
        };
        assert_eq!(command.to_string(), format!("git cat-file blob {blob}"));
    }
}
