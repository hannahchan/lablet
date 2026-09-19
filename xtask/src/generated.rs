//! `cargo xtask weaver generate`: the `lablet-telemetry-registry` sources and
//! the telemetry reference, rendered from the registry. Both are rendered into
//! a staging directory first, then installed or, with `--check`, compared with
//! the tree, so a failed render never leaves the tree half written.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::gates::{CheckResult, weaver_diagnostic_args};
use crate::process;
use crate::workspace::{Workspace, repo_root, workspace_root};

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
        let relative = format!("lablet/target/weaver-generate/{}", std::process::id());
        let stage = Self(repo_root().join(&relative));
        let _ = std::fs::remove_dir_all(&stage.0);
        for output in &OUTPUTS {
            let args = weaver_args(output, &format!("{relative}/{}", output.staged));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            process::capture("weaver", &args)?;
        }
        // Weaver's output is unformatted, and `cargo fmt` reaches only the
        // members of a workspace. rustfmt follows `mod` lines from the root.
        let edition = edition(&Workspace::load(&workspace_root())?)?;
        let lib = stage.0.join("src/lib.rs").display().to_string();
        process::capture("rustfmt", &["--edition", &edition, &lib])?;
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
    let root = repo_root();
    for output in &OUTPUTS {
        let tree = root.join(output.tree);
        let failed = |e: std::io::Error| format!("could not replace {}: {e}", tree.display());
        match std::fs::remove_dir_all(&tree) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(failed(e)),
            _ => {}
        }
        std::fs::rename(stage.0.join(output.staged), &tree).map_err(failed)?;
    }
    Ok(None)
}

/// Fails when the tree differs from what the registry renders to. It writes
/// only under `lablet/target`.
pub fn check() -> CheckResult {
    let stage = Stage::render()?;
    let root = repo_root();
    let mut found = Vec::new();
    for output in &OUTPUTS {
        let rendered = stage.0.join(output.staged);
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
