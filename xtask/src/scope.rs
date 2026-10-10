//! Whether a push needs a build: the `ci` gate runs every step, or, when
//! every path changed since the base is documentation, only the steps that
//! start no compiler. In CI, `coverage` and `weaver live-check` ask the same
//! question through `cargo xtask scope` and skip on the same answer.
//!
//! The base is the changelog gate's, so the two judge one range: on a branch
//! the merge-base with `origin/main`, the whole branch and never one push of
//! it, since a docs push on top of a code push would otherwise land with no
//! full run of its tip; on `main`, what `main` pointed at before the push.
//!
//! Deny by default: a path is documentation only when [`is_documentation`]
//! says so, so a file nobody has classified is a build input. And every
//! uncertainty, no base, a git that fails, an empty change, is the full gate:
//! a step that runs when it needn't costs a minute, one that quietly skips
//! itself costs a defect.

use std::fmt;
use std::path::Path;
use std::sync::LazyLock;

use crate::changelog::{self, BASE_VARIABLE};
use crate::error::chain;
use crate::report::Note;
use crate::workspace::repo_root;
use crate::{generated, process};

/// The steps a push needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every step of the gate, and the coverage and live-check jobs.
    Full,
    /// Only the steps that read documentation.
    Docs,
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "full",
            Self::Docs => "docs",
        })
    }
}

/// A scope and why.
#[derive(Debug, PartialEq, Eq)]
pub struct Decision {
    pub scope: Scope,
    pub reason: String,
}

/// Decided once for the process, so the steps a gate plans and the note its
/// `scope` step reports can't disagree.
static DECISION: LazyLock<Decision> = LazyLock::new(|| {
    let from_environment = std::env::var(BASE_VARIABLE).ok();
    decide_in(&repo_root(), from_environment.as_deref())
});

/// The scope of the repository this xtask belongs to.
pub fn decision() -> &'static Decision {
    &DECISION
}

/// The note of the `scope` step of `ci`, which says which steps follow. A
/// reduced run warns, so its closing line never reads as a full one.
pub fn note() -> Note {
    let Decision { scope, reason } = decision();
    match scope {
        Scope::Full => Note::Info(format!("full: {reason}")),
        Scope::Docs => Note::Warning(format!(
            "documentation only: {reason}; no step that builds ran"
        )),
    }
}

/// [`decision`], for the repository at `root`, with `from_environment` read
/// from [`BASE_VARIABLE`].
pub fn decide_in(root: &Path, from_environment: Option<&str>) -> Decision {
    let full = |reason: String| Decision {
        scope: Scope::Full,
        reason,
    };
    let base = changelog::resolve_base(
        from_environment,
        |revision| Ok(process::git_commit(root, revision)?.is_some()),
        |branch| process::git_merge_base(root, branch),
    );
    let base = match base {
        Ok(Some(base)) => base,
        Ok(None) => return full("there's no base commit to compare with".to_owned()),
        Err(error) => return full(format!("the base couldn't be found: {}", chain(&error))),
    };
    let since = format!("{} ({})", changelog::short(&base.revision), base.source);
    let paths = match changelog::changed_paths(root, &base.revision) {
        Ok(paths) => paths,
        Err(error) => {
            return full(format!(
                "what changed since {since} couldn't be listed: {}",
                chain(&error)
            ));
        }
    };
    // A document that is gone may be one a test reads by name, as the
    // prose lint's own test reads `product/spec.md`.
    if let Some(gone) = paths.iter().find(|path| !root.join(path).exists()) {
        return full(format!(
            "{gone} was removed since {since}, and something may read it"
        ));
    }
    match classify(&paths) {
        Change::Empty => full(format!("nothing changed since {since}")),
        Change::BuildInput(path) => full(format!(
            "{path} changed since {since}, and it isn't documentation"
        )),
        Change::Documentation => Decision {
            scope: Scope::Docs,
            reason: format!(
                "the {} path(s) changed since {since} are all documentation",
                paths.len()
            ),
        },
    }
}

/// What a changed set is, with the path that settled it.
#[derive(Debug, PartialEq, Eq)]
pub enum Change<'a> {
    /// No paths, which says nothing about the change.
    Empty,
    /// Every path is documentation.
    Documentation,
    /// At least this path is a build input.
    BuildInput(&'a str),
}

pub fn classify(paths: &[String]) -> Change<'_> {
    if paths.is_empty() {
        return Change::Empty;
    }
    paths
        .iter()
        .find(|path| !is_documentation(path))
        .map_or(Change::Documentation, |path| Change::BuildInput(path))
}

/// The trees whose Markdown is documentation.
const DOCUMENTATION_TREES: [&str; 3] = ["product/", "contributing/", "lablet/docs/"];

/// Single documents outside those trees.
const DOCUMENTS: [&str; 5] = [
    "README.md",
    "CLAUDE.md",
    "CHANGELOG.md",
    "lablet/README.md",
    "lablet/examples/README.md",
];

/// Whether a repository-relative path can be ruled out as a build input. Only
/// Markdown, and only where nothing reads what's there; a test that reads a
/// document by name, as the prose lint's does, is answered by counting a
/// removed path as a build input. Not documentation: the golden fixtures and an
/// example's work directory hold Markdown that runs read, and the telemetry
/// reference is rendered by weaver, so a hand edit there is caught only by
/// `weaver generate --check`.
fn is_documentation(path: &str) -> bool {
    let is_markdown = Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    let generated = generated::trees().any(|tree| {
        path.strip_prefix(tree)
            .is_some_and(|rest| rest.starts_with('/'))
    });
    is_markdown
        && !generated
        && (DOCUMENTS.contains(&path)
            || DOCUMENTATION_TREES
                .iter()
                .any(|tree| path.starts_with(tree)))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::workspace::fixture::{TempDir, defy_git_defaults, scratch_git};

    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn a_change_confined_to_documentation_is_documentation() {
        let changed = paths(&[
            "README.md",
            "CLAUDE.md",
            "CHANGELOG.md",
            "product/build-plan.md",
            "product/research/design-review/first-pass.md",
            "contributing/reviews.md",
            "lablet/README.md",
            "lablet/docs/telemetry.md",
            "lablet/docs/README.md",
            "lablet/examples/README.md",
        ]);
        assert_eq!(classify(&changed), Change::Documentation);
    }

    #[test]
    fn one_build_input_puts_the_whole_change_on_the_full_gate() {
        let changed = paths(&["product/build-plan.md", "xtask/src/scope.rs"]);
        assert_eq!(classify(&changed), Change::BuildInput("xtask/src/scope.rs"));
    }

    #[test]
    fn the_markdown_something_reads_and_the_build_inputs_beside_documentation_are_not_documentation()
     {
        for path in [
            // Rendered by weaver from the registry.
            "lablet/docs/telemetry/README.md",
            "lablet/docs/telemetry/lablet/spans.md",
            // A contract fixture the golden tests read.
            "lablet/tests/fixtures/golden/cancelled/work/notes.md",
            // An example's work directory, which its runs read.
            "lablet/examples/two-turns/work/notes.md",
            // Vendored, and weaver reads it.
            "lablet/telemetry/deps/weaver-packages/templates/docs/markdown/readme.md.j2",
            "lablet/telemetry/deps/semantic-conventions/model/README.md",
            "lablet/crates/domain/model/README.md",
            "product/research/data.csv",
            "lablet/examples/two-turns/lablet.yaml",
            "lablet/Cargo.toml",
            "lablet/Cargo.lock",
            "rust-toolchain.toml",
            "mise.toml",
            ".vale.ini",
            "dprint.json",
            ".github/workflows/ci.yml",
            ".claude/settings.json",
            "xtask/src/scope.rs",
            "AGENTS.md",
        ] {
            assert_eq!(
                classify(&paths(&[path])),
                Change::BuildInput(path),
                "{path} classified as documentation"
            );
        }
    }

    #[test]
    fn every_rendered_tree_is_a_build_input_whatever_it_holds() {
        for tree in generated::trees() {
            let page = format!("{tree}/README.md");
            assert!(!is_documentation(&page), "{page}");
        }
        // A sibling that only shares the tree's name is not inside it.
        assert!(is_documentation("lablet/docs/telemetry.md"));
    }

    #[test]
    fn an_empty_change_is_the_full_gate() {
        assert_eq!(classify(&[]), Change::Empty);
    }

    #[test]
    fn a_reduced_run_warns_and_a_full_one_informs() {
        let note = note();
        match decision().scope {
            Scope::Full => assert!(matches!(note, Note::Info(_)), "{note}"),
            Scope::Docs => assert!(matches!(note, Note::Warning(_)), "{note}"),
        }
        assert_eq!(Scope::Full.to_string(), "full");
        assert_eq!(Scope::Docs.to_string(), "docs");
    }

    /// A scratch repository on `main` with a source file and a document.
    fn repository(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        dir.write("README.md", "# Lablet\n");
        dir.write("lablet/src/lib.rs", "\n");
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

    #[test]
    fn a_branch_is_judged_whole_from_its_merge_base_not_by_its_last_commit() {
        let dir = repository("scope-branch");
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        git(&["switch", "--quiet", "--create", "topic"]);
        dir.write("contributing/README.md", "# Contributing\n");
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=docs"]);
        let docs = decide_in(dir.path(), None);
        assert_eq!(docs.scope, Scope::Docs, "{}", docs.reason);
        assert!(
            docs.reason.contains("(merge-base with main)"),
            "{}",
            docs.reason
        );

        // Code, then documentation on top: the tip is a docs commit, and the
        // branch still needs a build.
        dir.write("lablet/src/lib.rs", "pub fn f() {}\n");
        git(&["commit", "--quiet", "--all", "--message=code"]);
        dir.write("README.md", "# Lablet, again\n");
        git(&["commit", "--quiet", "--all", "--message=more docs"]);
        let full = decide_in(dir.path(), None);
        assert_eq!(full.scope, Scope::Full);
        assert!(
            full.reason.starts_with("lablet/src/lib.rs changed since "),
            "{}",
            full.reason
        );

        // `main` moving on after the branch point doesn't count against it.
        let docs_only = git(&["rev-parse", "HEAD~2"]);
        git(&["switch", "--quiet", "--create", "docs-only", &docs_only]);
        git(&["switch", "--quiet", "main"]);
        dir.write("lablet/src/other.rs", "\n");
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=main moves on"]);
        git(&["switch", "--quiet", "docs-only"]);
        assert_eq!(decide_in(dir.path(), None).scope, Scope::Docs);
    }

    #[test]
    fn on_main_the_named_base_is_the_range_and_without_one_the_change_is_empty() {
        let dir = repository("scope-main");
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        let before = git(&["rev-parse", "HEAD"]);
        dir.write("product/brief.md", "# Brief\n");
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message=docs"]);

        let empty = decide_in(dir.path(), None);
        assert_eq!(empty.scope, Scope::Full);
        assert!(
            empty.reason.starts_with("nothing changed since "),
            "{}",
            empty.reason
        );

        let named = decide_in(dir.path(), Some(&before));
        assert_eq!(named.scope, Scope::Docs, "{}", named.reason);
        assert!(
            named.reason.contains("(LABLET_CHANGELOG_BASE)"),
            "{}",
            named.reason
        );
    }

    #[test]
    fn a_removed_or_renamed_document_is_the_full_gate() {
        let dir = repository("scope-removed");
        let git = |args: &[&str]| scratch_git(dir.path(), args).trim().to_owned();
        git(&["switch", "--quiet", "--create", "topic"]);
        std::fs::create_dir(dir.path().join("contributing")).unwrap();
        git(&["mv", "README.md", "contributing/README.md"]);
        git(&["commit", "--quiet", "--message=move"]);
        let moved = decide_in(dir.path(), None);
        assert_eq!(moved.scope, Scope::Full);
        assert!(
            moved.reason.starts_with("README.md was removed since "),
            "{}",
            moved.reason
        );
    }

    #[test]
    fn a_named_base_that_does_not_resolve_on_main_is_the_full_gate() {
        let dir = repository("scope-unresolved");
        let missing = "0123456789abcdef0123456789abcdef01234567";
        let decision = decide_in(dir.path(), Some(missing));
        assert_eq!(decision.scope, Scope::Full);
        assert!(
            decision.reason.starts_with("nothing changed since "),
            "{}",
            decision.reason
        );
    }

    #[test]
    fn uncommitted_and_untracked_files_count() {
        let dir = repository("scope-worktree");
        let git = |args: &[&str]| scratch_git(dir.path(), args);
        git(&["switch", "--quiet", "--create", "topic"]);
        dir.write("lablet/src/new.rs", "\n");
        assert_eq!(
            decide_in(dir.path(), None).reason,
            format!(
                "lablet/src/new.rs changed since {} (merge-base with main), and it isn't \
                 documentation",
                changelog::short(git(&["rev-parse", "HEAD"]).trim())
            )
        );
    }

    #[test]
    fn no_base_and_a_failing_git_are_the_full_gate() {
        let dir = repository("scope-no-base");
        scratch_git(dir.path(), &["branch", "--move", "main", "topic"]);
        assert_eq!(
            decide_in(dir.path(), None),
            Decision {
                scope: Scope::Full,
                reason: "there's no base commit to compare with".to_owned(),
            }
        );

        let gone = dir.path().join("gone");
        let failed = decide_in(&gone, None);
        assert_eq!(failed.scope, Scope::Full);
        assert!(
            failed.reason.starts_with("the base couldn't be found: "),
            "{}",
            failed.reason
        );
    }

    // The invariant the allowlist rests on: no Rust source reads a document
    // at compile time, or editing it would be a build input after all. A test
    // that reads one at run time reads it by name, and only removing it can
    // break that test, which the full gate catches.

    /// The placeholder target for an include whose argument isn't one this
    /// scan resolves.
    const UNREADABLE: &str = "<argument this scan can't resolve>";

    const MANIFEST_DIR: &str = "env!(\"CARGO_MANIFEST_DIR\")";

    /// Each `include_str!`/`include_bytes!` argument in `source`: a string
    /// literal relative to the file, `concat!(env!("CARGO_MANIFEST_DIR"),
    /// "...")` relative to the package, or `None` for anything else.
    fn include_arguments(source: &str) -> Vec<Option<Include>> {
        let mut arguments = Vec::new();
        for opening in ["include_str!(", "include_bytes!("] {
            for (index, _) in source.match_indices(opening) {
                let rest = source[index + opening.len()..].trim_start();
                let argument = if let Some(body) = rest.strip_prefix('"') {
                    body.split('"')
                        .next()
                        .map(|path| Include::File(path.to_owned()))
                } else {
                    rest.strip_prefix("concat!(")
                        .map(str::trim_start)
                        .and_then(|rest| rest.strip_prefix(MANIFEST_DIR))
                        .map(str::trim_start)
                        .and_then(|rest| rest.strip_prefix(','))
                        .map(str::trim_start)
                        .and_then(|rest| rest.strip_prefix('"'))
                        .and_then(|body| body.split('"').next())
                        .map(|path| Include::Package(path.to_owned()))
                };
                arguments.push(argument);
            }
        }
        arguments
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Include {
        /// Relative to the including file's directory.
        File(String),
        /// Appended to the package's directory.
        Package(String),
    }

    /// Every compile-time read in the tracked Rust sources under `root`, as
    /// `(source file, target)` pairs relative to `root`.
    fn compile_time_reads(root: &Path) -> Vec<(String, PathBuf)> {
        let listed = process::capture_in(root, "git", &["ls-files", "-z", "--", "*.rs"]).unwrap();
        let mut reads = Vec::new();
        for file in listed.split('\0').filter(|file| !file.is_empty()) {
            let source = std::fs::read_to_string(root.join(file)).unwrap();
            let directory = Path::new(file).parent().unwrap_or(Path::new(""));
            for argument in include_arguments(&source) {
                let target = match argument {
                    Some(Include::File(path)) => normalise(&directory.join(path)),
                    Some(Include::Package(path)) => {
                        let package = directory
                            .ancestors()
                            .find(|dir| root.join(dir).join("Cargo.toml").is_file())
                            .unwrap_or(Path::new(""));
                        normalise(&package.join(path.trim_start_matches('/')))
                    }
                    None => PathBuf::from(UNREADABLE),
                };
                reads.push((file.to_owned(), target));
            }
        }
        reads
    }

    /// Resolves `.` and `..` lexically, since a target that doesn't exist is
    /// still one the invariant wants to see.
    fn normalise(path: &Path) -> PathBuf {
        let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    parts.pop();
                }
                std::path::Component::CurDir => {}
                other => parts.push(other.as_os_str()),
            }
        }
        parts.iter().collect()
    }

    #[test]
    fn the_scan_reads_every_argument_shape_rustfmt_leaves() {
        // Assembled at run time, so this file doesn't hold what it scans for.
        let source = format!(
            r#"
            const A: &str = {str}"a.yaml");
            const B: &str = {str}concat!(
                {MANIFEST_DIR},
                "/../../schema.json"
            ));
            const C: &[u8] = {bytes}concat!("x", "/y.md"));
            "#,
            str = "include_str!(",
            bytes = "include_bytes!(",
        );
        assert_eq!(
            include_arguments(&source),
            [
                Some(Include::File("a.yaml".to_owned())),
                Some(Include::Package("/../../schema.json".to_owned())),
                None,
            ]
        );
    }

    #[test]
    fn the_scan_finds_the_reads_that_exist() {
        // Guards the guard: a scan that found nothing would pass the next
        // test for the wrong reason.
        let reads = compile_time_reads(&repo_root());
        for expected in [
            "lablet/schema.json",
            "lablet/tests/fixtures/outcome.json",
            "lablet/examples/two-turns/lablet.yaml",
        ] {
            assert!(
                reads
                    .iter()
                    .any(|(_, target)| target == Path::new(expected)),
                "the scan missed {expected}: {reads:#?}"
            );
        }
    }

    #[test]
    fn no_rust_source_reads_a_document_at_compile_time() {
        // The fix is never to widen this test: read a data file the crate
        // owns, and hold it to the document some other way.
        let offenders: Vec<String> = compile_time_reads(&repo_root())
            .into_iter()
            .filter(|(_, target)| {
                target == Path::new(UNREADABLE) || target.to_str().is_none_or(is_documentation)
            })
            .map(|(source, target)| format!("{source} reads {}", target.display()))
            .collect();
        assert!(
            offenders.is_empty(),
            "a document read at compile time makes editing it a build input:\n{}",
            offenders.join("\n")
        );
    }
}
