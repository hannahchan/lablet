//! `cargo xtask weaver generate`: the `lablet-telemetry-registry` sources and
//! the telemetry reference, rendered from the registry. Both are rendered into
//! a staging directory first, then installed or, with `--check`, compared with
//! the tree, so a failed render never leaves the tree half written.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::gates::{CheckResult, weaver_diagnostic_args};
use crate::process;
use crate::workspace::{Workspace, repo_root};

const REGISTRY: &str = "lablet/telemetry/registry";
const FIX: &str = "fix with: cargo xtask weaver generate";

/// One generated directory. Every file in it is generated.
struct Output {
    target: &'static str,
    /// Weaver looks under it for `registry/<target>/weaver.yaml`, then for
    /// `<target>/weaver.yaml`, which is how upstream lays out the pages.
    templates: &'static str,
    params: &'static [&'static str],
    /// Its directory in the staging area.
    staged: &'static str,
    /// The directory it replaces, relative to the repository root.
    tree: &'static str,
}

const OUTPUTS: [Output; 2] = [
    Output {
        target: "rust",
        templates: "lablet/telemetry/templates",
        params: &[],
        staged: "src",
        tree: "lablet/crates/adapters/secondary/shared/telemetry-registry/src",
    },
    Output {
        target: "markdown",
        templates: "lablet/telemetry/deps/weaver-packages/templates/docs",
        // The pages link to one another from the repository root, as GitHub
        // resolves a link that starts with a slash.
        params: &["--param", "registry_base_url=/lablet/docs/telemetry"],
        staged: "docs",
        tree: "lablet/docs/telemetry",
    },
];

/// Paths are relative to the repository root, where weaver runs.
fn weaver_args(output: &Output, directory: &str) -> Vec<String> {
    let mut args = vec!["registry", "generate", "--v2", "--quiet"];
    args.extend(["--registry", REGISTRY, "--templates", output.templates]);
    args.extend(output.params);
    args.extend(weaver_diagnostic_args());
    args.extend([output.target, directory]);
    args.into_iter().map(str::to_owned).collect()
}

/// A rendering under `lablet/target`, which git ignores and which shares a
/// filesystem with the tree, so installing is a rename. Removed on drop.
struct Stage(PathBuf);

impl Stage {
    fn render() -> Result<Self, String> {
        let root = repo_root();
        Self::render_with(&root, |program, args| {
            process::capture_in(&root, program, args).map(drop)
        })
    }

    /// [`Stage::render`] for the repository at `root`, with `run` in place
    /// of running a program there.
    fn render_with(
        root: &Path,
        mut run: impl FnMut(&str, &[&str]) -> Result<(), String>,
    ) -> Result<Self, String> {
        let relative = format!("lablet/target/weaver-generate/{}", std::process::id());
        let stage = Self(root.join(&relative));
        let _ = std::fs::remove_dir_all(&stage.0);
        for output in &OUTPUTS {
            let args = weaver_args(output, &format!("{relative}/{}", output.staged));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            run("weaver", &args)?;
        }
        // Weaver's output is unformatted, and `cargo fmt` reaches only the
        // members of a workspace. rustfmt follows `mod` lines from the root.
        let edition = edition(&Workspace::load(&root.join("lablet"))?)?;
        let lib = stage.0.join("src/lib.rs").display().to_string();
        run("rustfmt", &["--edition", &edition, &lib])?;
        Ok(stage)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn edition(workspace: &Workspace) -> Result<String, String> {
    workspace
        .document
        .get("workspace")
        .and_then(|table| table.get("package"))
        .and_then(|table| table.get("edition"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "lablet/Cargo.toml sets no `workspace.package.edition`".to_owned())
}

/// Replaces the generated directories of the tree.
pub fn write() -> CheckResult {
    let stage = Stage::render()?;
    install(&stage.0, &repo_root())
}

/// Moves each rendering in `stage` over the directory it replaces under
/// `root`, which need not exist yet.
fn install(stage: &Path, root: &Path) -> CheckResult {
    for output in &OUTPUTS {
        let tree = root.join(output.tree);
        let failed = |e: std::io::Error| format!("could not replace {}: {e}", tree.display());
        match std::fs::remove_dir_all(&tree) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(failed(e)),
            _ => {}
        }
        std::fs::rename(stage.join(output.staged), &tree).map_err(failed)?;
    }
    Ok(None)
}

/// Fails when the tree differs from what the registry renders to. It writes
/// only under `lablet/target`.
pub fn check() -> CheckResult {
    let stage = Stage::render()?;
    compare(&stage.0, &repo_root())
}

/// Fails when a directory under `root` differs from its rendering in `stage`.
fn compare(stage: &Path, root: &Path) -> CheckResult {
    let mut found = Vec::new();
    for output in &OUTPUTS {
        let rendered = stage.join(output.staged);
        found.extend(differences(
            &rendered,
            &root.join(output.tree),
            output.tree,
        )?);
    }
    if found.is_empty() {
        return Ok(None);
    }
    let mut message = "the generated files differ from what the registry renders to:\n".to_owned();
    for line in &found {
        let _ = writeln!(message, "  {line}");
    }
    let _ = write!(message, "{FIX}");
    Err(message)
}

/// One line for each file that differs between `rendered` and `tree`, naming
/// it under `label`.
fn differences(rendered: &Path, tree: &Path, label: &str) -> Result<Vec<String>, String> {
    let rendered = files(rendered, rendered)?;
    let mut in_tree = files(tree, tree)?;
    let mut lines = Vec::new();
    for (path, contents) in &rendered {
        match in_tree.remove(path) {
            Some(existing) if existing == *contents => {}
            Some(_) => lines.push(format!("{label}/{path} is out of date")),
            None => lines.push(format!("{label}/{path} is missing")),
        }
    }
    for path in in_tree.keys() {
        lines.push(format!("{label}/{path} is no longer generated"));
    }
    Ok(lines)
}

/// Every file under `directory`, keyed by its path relative to `base`. A
/// directory that doesn't exist holds no files.
fn files(base: &Path, directory: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let failed = |e: std::io::Error| format!("could not read {}: {e}", directory.display());
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(failed(e)),
    };
    let mut found = BTreeMap::new();
    for entry in entries {
        let path = entry.map_err(failed)?.path();
        if path.is_dir() {
            found.extend(files(base, &path)?);
        } else {
            let relative = path.strip_prefix(base).unwrap_or(&path);
            let contents = std::fs::read(&path).map_err(failed)?;
            found.insert(relative.display().to_string(), contents);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::TempDir;
    use crate::workspace::workspace_root;

    #[test]
    fn a_tree_that_matches_the_rendering_has_no_differences() {
        let rendered = TempDir::new("rendered");
        let tree = TempDir::new("tree");
        for dir in [&rendered, &tree] {
            dir.write("README.md", "# Telemetry\n");
            dir.write("lablet/spans.md", "spans\n");
        }
        let found = differences(rendered.path(), tree.path(), "docs").unwrap();
        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn a_changed_a_missing_and_a_leftover_file_are_each_named() {
        let rendered = TempDir::new("rendered");
        let tree = TempDir::new("tree");
        rendered.write("README.md", "new\n");
        tree.write("README.md", "old\n");
        rendered.write("lablet/events.md", "events\n");
        tree.write("gone/spans.md", "spans\n");
        let found = differences(rendered.path(), tree.path(), "lablet/docs/telemetry").unwrap();
        assert_eq!(
            found,
            [
                "lablet/docs/telemetry/README.md is out of date",
                "lablet/docs/telemetry/lablet/events.md is missing",
                "lablet/docs/telemetry/gone/spans.md is no longer generated",
            ]
        );
    }

    #[test]
    fn a_tree_without_the_directory_is_missing_every_file() {
        let rendered = TempDir::new("rendered");
        rendered.write("lib.rs", "");
        let nowhere = rendered.path().join("absent");
        let found = differences(rendered.path(), &nowhere, "src").unwrap();
        assert_eq!(found, ["src/lib.rs is missing"]);
    }

    /// Where each output's directory is in the tree, spelt out, so that a
    /// rendering compared with or moved to the wrong one fails.
    const REGISTRY_SOURCES: &str = "lablet/crates/adapters/secondary/shared/telemetry-registry/src";
    const REFERENCE: &str = "lablet/docs/telemetry";

    /// A stage holding both renderings: the crate's sources and the pages.
    fn staged() -> TempDir {
        let stage = TempDir::new("stage");
        stage.write("src/lib.rs", "pub mod attributes;\n");
        stage.write("docs/README.md", "# Telemetry\n");
        stage
    }

    #[test]
    fn a_check_fails_when_either_output_differs_from_its_rendering_and_says_how_to_fix_it() {
        let (stage, root) = (staged(), TempDir::new("root"));
        root.write(
            &format!("{REGISTRY_SOURCES}/lib.rs"),
            "pub mod attributes;\n",
        );
        root.write(&format!("{REFERENCE}/README.md"), "# Telemetry\n");
        assert_eq!(compare(stage.path(), root.path()), Ok(None));

        root.write(&format!("{REGISTRY_SOURCES}/lib.rs"), "pub mod old;\n");
        root.write(&format!("{REFERENCE}/spans.md"), "spans\n");
        assert_eq!(
            compare(stage.path(), root.path()),
            Err(format!(
                "the generated files differ from what the registry renders to:\n  \
                 {REGISTRY_SOURCES}/lib.rs is out of date\n  {REFERENCE}/spans.md is no longer \
                 generated\nfix with: cargo xtask weaver generate"
            ))
        );
    }

    #[test]
    fn installing_replaces_each_generated_directory_whole_and_makes_one_that_is_missing() {
        let (stage, root) = (staged(), TempDir::new("root"));
        root.write(&format!("{REGISTRY_SOURCES}/lib.rs"), "pub mod old;\n");
        root.write(&format!("{REGISTRY_SOURCES}/old.rs"), "\n");
        // The pages have never been rendered here.
        std::fs::create_dir_all(root.path().join("lablet/docs")).unwrap();
        assert_eq!(install(stage.path(), root.path()), Ok(None));

        let tree = |relative: &str| {
            let directory = root.path().join(relative);
            files(&directory, &directory).unwrap()
        };
        let file = |path: &str, text: &str| (path.to_owned(), text.as_bytes().to_vec());
        assert_eq!(
            tree(REGISTRY_SOURCES),
            BTreeMap::from([file("lib.rs", "pub mod attributes;\n")])
        );
        assert_eq!(
            tree(REFERENCE),
            BTreeMap::from([file("README.md", "# Telemetry\n")])
        );
    }

    #[test]
    fn a_file_where_a_generated_directory_belongs_is_an_error_not_an_empty_directory() {
        let (rendered, tree) = (TempDir::new("rendered"), TempDir::new("tree"));
        rendered.write("lib.rs", "");
        tree.write("src", "not a directory\n");
        let in_the_way = tree.path().join("src");
        let error = differences(rendered.path(), &in_the_way, "src").unwrap_err();
        let expected = format!("could not read {}: ", in_the_way.display());
        assert!(error.starts_with(&expected), "{error}");
    }

    #[test]
    fn a_generated_directory_that_cannot_be_removed_is_left_as_it_was_and_named() {
        let (stage, root) = (TempDir::new("stage"), TempDir::new("root"));
        // A file at both ends: removing the tree's fails for its not being a
        // directory, where a rename alone would have replaced it.
        stage.write("src", "rendered\n");
        stage.write("docs/README.md", "# Telemetry\n");
        root.write(REGISTRY_SOURCES, "in the way\n");
        let tree = root.path().join(REGISTRY_SOURCES);
        let error = install(stage.path(), root.path()).unwrap_err();
        let expected = format!("could not replace {}: ", tree.display());
        assert!(error.starts_with(&expected), "{error}");
        assert_eq!(std::fs::read_to_string(&tree).unwrap(), "in the way\n");
        assert!(!root.path().join(REFERENCE).exists());
    }

    /// Stands in for weaver and rustfmt under `root`: weaver writes a file
    /// where it is told to render, and the call numbered `failing`, counting
    /// from 1, fails. Returns the rendering and the programs run.
    fn render(root: &Path, failing: usize) -> (Result<Stage, String>, Vec<String>) {
        let mut ran = Vec::new();
        let rendered = Stage::render_with(root, |program, args| {
            ran.push(program.to_owned());
            if program == "weaver" {
                let directory = root.join(args[args.len() - 1]);
                std::fs::create_dir_all(&directory).unwrap();
                std::fs::write(directory.join("lib.rs"), "").unwrap();
            }
            if ran.len() == failing {
                Err(format!("{program} failed"))
            } else {
                Ok(())
            }
        });
        (rendered, ran)
    }

    #[test]
    fn the_stage_is_removed_whether_the_rendering_fails_or_is_done_with() {
        let root = TempDir::new("render");
        root.write(
            "lablet/Cargo.toml",
            "[workspace]\nmembers = []\n\n[workspace.package]\nedition = \"2024\"\n",
        );
        let staged = root.path().join(format!(
            "lablet/target/weaver-generate/{}",
            std::process::id()
        ));
        for (failing, program) in [(1, "weaver"), (2, "weaver"), (3, "rustfmt")] {
            let (rendered, ran) = render(root.path(), failing);
            assert_eq!(rendered.err(), Some(format!("{program} failed")));
            assert_eq!(ran.len(), failing);
            assert!(!staged.exists(), "left by a failed {program}");
        }

        let (rendered, ran) = render(root.path(), 0);
        assert_eq!(ran, ["weaver", "weaver", "rustfmt"]);
        let rendered = rendered.unwrap();
        assert_eq!(rendered.0, staged);
        for output in &OUTPUTS {
            assert!(staged.join(output.staged).join("lib.rs").is_file());
        }
        drop(rendered);
        assert!(!staged.exists(), "left once the rendering was done with");

        // Without the workspace's edition, rustfmt never runs.
        std::fs::remove_file(root.path().join("lablet/Cargo.toml")).unwrap();
        let (rendered, ran) = render(root.path(), 0);
        let error = rendered.err().unwrap_or_default();
        assert!(error.starts_with("could not read "), "{error}");
        assert_eq!(ran, ["weaver", "weaver"]);
        assert!(!staged.exists(), "left by a failed read of the edition");
    }

    #[test]
    fn weaver_renders_each_output_from_the_working_tree_into_the_stage() {
        let root = repo_root();
        assert!(root.join(REGISTRY).join("manifest.yaml").is_file());
        for output in &OUTPUTS {
            let args = weaver_args(output, "stage");
            assert_eq!(args[..4], ["registry", "generate", "--v2", "--quiet"]);
            assert_eq!(args[args.len() - 2..], [output.target, "stage"]);
            // A git URL in place of the templates would be cloned on every run.
            let found = [
                format!("registry/{}", output.target),
                output.target.to_owned(),
            ]
            .iter()
            .any(|at| {
                root.join(output.templates)
                    .join(at)
                    .join("weaver.yaml")
                    .is_file()
            });
            assert!(
                found,
                "{} has no `{}` templates",
                output.templates, output.target
            );
            assert!(root.join(output.tree).is_dir(), "{}", output.tree);
        }
        let links = "registry_base_url=/lablet/docs/telemetry".to_owned();
        assert!(!weaver_args(&OUTPUTS[0], "stage").contains(&links));
        assert!(weaver_args(&OUTPUTS[1], "stage").contains(&links));
    }

    #[test]
    fn rustfmt_is_given_the_edition_of_the_workspace() {
        let workspace = Workspace::load(&workspace_root()).unwrap();
        let edition = edition(&workspace).unwrap();
        let declared = std::fs::read_to_string(workspace_root().join("Cargo.toml")).unwrap();
        assert!(declared.contains(&format!("edition = \"{edition}\"")));
    }
}
