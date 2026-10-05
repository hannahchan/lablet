//! `cargo xtask weaver generate`: the telemetry module of each crate that
//! emits a signal, the `lablet-telemetry-registry` sources, and the telemetry
//! reference, rendered from the registry. Everything is rendered into a
//! staging directory first, then installed or, with `--check`, compared with
//! the tree, so a failed render never leaves the tree half written.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::error::{Error, Malformed, Verb};
use crate::gates::{CheckResult, Failure, weaver_diagnostic_args};
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
    /// The file rustfmt formats the rendering from, following its `mod`
    /// lines; `None` for a rendering that isn't Rust.
    formatted: Option<&'static str>,
}

/// Where the templates of lablet's own Rust renderings are.
const RUST_TEMPLATES: &str = "lablet/telemetry/templates";

/// The typed module of one crate: the signals its registry folder declares,
/// using lablet's hand-written telemetry where `params` say.
const fn module(
    params: &'static [&'static str],
    staged: &'static str,
    tree: &'static str,
) -> Output {
    Output {
        target: "rust-crate",
        templates: RUST_TEMPLATES,
        params,
        staged,
        tree,
        formatted: Some("mod.rs"),
    }
}

const OUTPUTS: [Output; 4] = [
    // The loop's signals, and the `Join` struct every module uses.
    module(
        &[
            "--param",
            "crate_dir=application/run",
            "--param",
            "telemetry_path=crate::telemetry",
            "--param",
            "join=true",
        ],
        "run",
        "lablet/crates/application/run/src/telemetry/generated",
    ),
    // The composition root's signals, the root span and the wide event.
    module(
        &[
            "--param",
            "crate_dir=apps/lablet",
            "--param",
            "telemetry_path=lablet_run::telemetry",
            "--param",
            "join=false",
        ],
        "root",
        "lablet/apps/lablet/src/telemetry/generated",
    ),
    // The constants crate, until phase 6a's last landing removes it.
    Output {
        target: "rust",
        templates: RUST_TEMPLATES,
        params: &[],
        staged: "src",
        tree: "lablet/crates/adapters/secondary/shared/telemetry-registry/src",
        formatted: Some("lib.rs"),
    },
    Output {
        target: "markdown",
        templates: "lablet/telemetry/deps/weaver-packages/templates/docs",
        // The pages link to one another from the repository root, as GitHub
        // resolves a link that starts with a slash.
        params: &["--param", "registry_base_url=/lablet/docs/telemetry"],
        staged: "docs",
        tree: "lablet/docs/telemetry",
        formatted: None,
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
    fn render() -> Result<Self, Error> {
        let root = repo_root();
        Self::render_with(&root, |program, args| {
            process::capture_in(&root, program, args).map(drop)
        })
    }

    /// [`Stage::render`] for the repository at `root`, with `run` in place
    /// of running a program there.
    fn render_with(
        root: &Path,
        mut run: impl FnMut(&str, &[&str]) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        let relative = format!("lablet/target/weaver-generate/{}", std::process::id());
        let stage = Self(root.join(&relative));
        let _ = std::fs::remove_dir_all(&stage.0);
        for output in &OUTPUTS {
            let args = weaver_args(output, &format!("{relative}/{}", output.staged));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            run("weaver", &args)?;
        }
        // Weaver's output is unformatted, and `cargo fmt` reaches only the
        // members of a workspace. rustfmt follows `mod` lines from each
        // rendering's root file, and takes them all at once.
        let edition = edition(&Workspace::load(&root.join("lablet"))?)?;
        let roots: Vec<String> = OUTPUTS
            .iter()
            .filter_map(|output| {
                let formatted = output.formatted?;
                Some(
                    stage
                        .0
                        .join(output.staged)
                        .join(formatted)
                        .display()
                        .to_string(),
                )
            })
            .collect();
        let mut args = vec!["--edition", &edition];
        args.extend(roots.iter().map(String::as_str));
        run("rustfmt", &args)?;
        Ok(stage)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn edition(workspace: &Workspace) -> Result<String, Error> {
    workspace
        .document
        .get("workspace")
        .and_then(|table| table.get("package"))
        .and_then(|table| table.get("edition"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or(Malformed::Lacks("`workspace.package.edition`"))
        .map_err(Error::parse(&workspace.root.join("Cargo.toml")))
}

/// Replaces the generated directories of the tree.
pub fn write() -> CheckResult {
    let stage = Stage::render()?;
    install(&stage.0, &repo_root())?;
    Ok(None)
}

/// Moves each rendering in `stage` over the directory it replaces under
/// `root`, which need not exist yet, nor its parent: a crate's module is
/// made where the crate has no `telemetry` directory yet.
fn install(stage: &Path, root: &Path) -> Result<(), Error> {
    for output in &OUTPUTS {
        let tree = root.join(output.tree);
        match std::fs::remove_dir_all(&tree) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(Error::file(Verb::Replace, &tree)(e));
            }
            _ => {}
        }
        if let Some(parent) = tree.parent() {
            std::fs::create_dir_all(parent).map_err(Error::file(Verb::Replace, &tree))?;
        }
        std::fs::rename(stage.join(output.staged), &tree)
            .map_err(Error::file(Verb::Replace, &tree))?;
    }
    Ok(())
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
    Err(Failure::Verdict(message))
}

/// One line for each file that differs between `rendered` and `tree`, naming
/// it under `label`.
fn differences(rendered: &Path, tree: &Path, label: &str) -> Result<Vec<String>, Error> {
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
fn files(base: &Path, directory: &Path) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(Error::file(Verb::Read, directory)(e)),
    };
    let mut found = BTreeMap::new();
    for entry in entries {
        let path = entry.map_err(Error::file(Verb::Read, directory))?.path();
        if path.is_dir() {
            found.extend(files(base, &path)?);
        } else {
            let relative = path.strip_prefix(base).unwrap_or(&path);
            let contents = std::fs::read(&path).map_err(Error::file(Verb::Read, &path))?;
            found.insert(relative.display().to_string(), contents);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::chain;
    use crate::process::Invocation;
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
    const RUN_MODULE: &str = "lablet/crates/application/run/src/telemetry/generated";
    const ROOT_MODULE: &str = "lablet/apps/lablet/src/telemetry/generated";
    const REGISTRY_SOURCES: &str = "lablet/crates/adapters/secondary/shared/telemetry-registry/src";
    const REFERENCE: &str = "lablet/docs/telemetry";

    /// The file each output's rendering holds in the stage and, when it's up
    /// to date, in the tree.
    const RENDERED: [(&str, &str, &str); 4] = [
        ("run", "mod.rs", "pub mod key;\n"),
        ("root", "mod.rs", "pub mod key;\n"),
        ("src", "lib.rs", "pub mod attributes;\n"),
        ("docs", "README.md", "# Telemetry\n"),
    ];

    /// A stage holding every rendering: the two modules, the crate's sources
    /// and the pages.
    fn staged() -> TempDir {
        let stage = TempDir::new("stage");
        for (directory, file, text) in RENDERED {
            stage.write(&format!("{directory}/{file}"), text);
        }
        stage
    }

    /// A tree in which every output is up to date.
    fn up_to_date() -> TempDir {
        let root = TempDir::new("root");
        for (output, (_, file, text)) in OUTPUTS.iter().zip(RENDERED) {
            root.write(&format!("{}/{file}", output.tree), text);
        }
        root
    }

    #[test]
    fn the_outputs_are_the_two_crate_modules_the_constants_crate_and_the_pages() {
        let trees: Vec<&str> = OUTPUTS.iter().map(|output| output.tree).collect();
        assert_eq!(
            trees,
            [RUN_MODULE, ROOT_MODULE, REGISTRY_SOURCES, REFERENCE]
        );
        let staged: Vec<&str> = OUTPUTS.iter().map(|output| output.staged).collect();
        assert_eq!(staged, RENDERED.map(|(directory, ..)| directory));
        let formatted: Vec<Option<&str>> = OUTPUTS.iter().map(|output| output.formatted).collect();
        assert_eq!(
            formatted,
            [Some("mod.rs"), Some("mod.rs"), Some("lib.rs"), None]
        );
    }

    #[test]
    fn a_check_fails_when_any_output_differs_from_its_rendering_and_names_each_file() {
        let (stage, root) = (staged(), up_to_date());
        assert_eq!(compare(stage.path(), root.path()).unwrap(), None);

        root.write(&format!("{RUN_MODULE}/mod.rs"), "pub mod old;\n");
        root.write(&format!("{ROOT_MODULE}/spans.rs"), "struct Gone;\n");
        root.write(&format!("{REGISTRY_SOURCES}/lib.rs"), "pub mod old;\n");
        root.write(&format!("{REFERENCE}/spans.md"), "spans\n");
        assert_eq!(
            compare(stage.path(), root.path())
                .unwrap_err()
                .into_verdict(),
            format!(
                "the generated files differ from what the registry renders to:\n  \
                 {RUN_MODULE}/mod.rs is out of date\n  {ROOT_MODULE}/spans.rs is no longer \
                 generated\n  {REGISTRY_SOURCES}/lib.rs is out of date\n  {REFERENCE}/spans.md is \
                 no longer generated\nfix with: cargo xtask weaver generate"
            )
        );
    }

    #[test]
    fn installing_replaces_each_generated_directory_whole_and_makes_one_that_is_missing() {
        let (stage, root) = (staged(), TempDir::new("root"));
        root.write(&format!("{REGISTRY_SOURCES}/lib.rs"), "pub mod old;\n");
        root.write(&format!("{REGISTRY_SOURCES}/old.rs"), "\n");
        root.write(&format!("{RUN_MODULE}/mod.rs"), "pub mod old;\n");
        // The pages have never been rendered here, and neither has the
        // composition root's module, whose crate has no `telemetry`
        // directory at all.
        std::fs::create_dir_all(root.path().join("lablet/docs")).unwrap();
        install(stage.path(), root.path()).unwrap();

        let tree = |relative: &str| {
            let directory = root.path().join(relative);
            files(&directory, &directory).unwrap()
        };
        let file = |path: &str, text: &str| (path.to_owned(), text.as_bytes().to_vec());
        for (output, (_, name, text)) in OUTPUTS.iter().zip(RENDERED) {
            assert_eq!(
                tree(output.tree),
                BTreeMap::from([file(name, text)]),
                "{}",
                output.tree
            );
        }
    }

    #[test]
    fn a_file_where_a_generated_directory_belongs_is_an_error_not_an_empty_directory() {
        let (rendered, tree) = (TempDir::new("rendered"), TempDir::new("tree"));
        rendered.write("lib.rs", "");
        tree.write("src", "not a directory\n");
        let in_the_way = tree.path().join("src");
        let error = differences(rendered.path(), &in_the_way, "src").unwrap_err();
        let Error::File { verb, path, source } = &error else {
            panic!("{error:?}");
        };
        assert_eq!((*verb, path), (Verb::Read, &in_the_way));
        let expected = format!("could not read {}: {source}", in_the_way.display());
        assert_eq!(chain(&error), expected);
    }

    /// A link to nothing is listed as an entry and can't be read.
    #[test]
    fn a_file_that_cannot_be_read_is_named_itself_not_its_directory() {
        let (rendered, tree) = (TempDir::new("rendered"), TempDir::new("tree"));
        rendered.write("lib.rs", "");
        tree.write("lib.rs", "");
        let dangling = tree.path().join("attributes.rs");
        std::os::unix::fs::symlink(tree.path().join("gone.rs"), &dangling).unwrap();
        let error = differences(rendered.path(), tree.path(), "src").unwrap_err();
        let Error::File { verb, path, source } = &error else {
            panic!("{error:?}");
        };
        assert_eq!((*verb, path), (Verb::Read, &dangling));
        let expected = format!("could not read {}: {source}", dangling.display());
        assert_eq!(chain(&error), expected);
    }

    #[test]
    fn a_generated_directory_that_cannot_be_removed_is_left_as_it_was_and_named() {
        let (stage, root) = (staged(), TempDir::new("root"));
        // A file where the first output's directory belongs: removing it
        // fails for its not being a directory, where a rename alone would
        // have replaced it.
        root.write(RUN_MODULE, "in the way\n");
        let tree = root.path().join(RUN_MODULE);
        let error = install(stage.path(), root.path()).unwrap_err();
        let Error::File { verb, path, source } = &error else {
            panic!("{error:?}");
        };
        assert_eq!((*verb, path), (Verb::Replace, &tree));
        let expected = format!("could not replace {}: {source}", tree.display());
        assert_eq!(chain(&error), expected);
        assert_eq!(std::fs::read_to_string(&tree).unwrap(), "in the way\n");
        for output in &OUTPUTS[1..] {
            assert!(!root.path().join(output.tree).exists(), "{}", output.tree);
        }
    }

    /// Stands in for weaver and rustfmt under `root`: weaver writes a file
    /// where it is told to render, and the call numbered `failing`, counting
    /// from 1, fails. Returns the rendering and the programs run.
    fn render(root: &Path, failing: usize) -> (Result<Stage, Error>, Vec<Invocation>) {
        let mut ran = Vec::new();
        let rendered = Stage::render_with(root, |program, args| {
            ran.push(Invocation::new(root, program, args));
            if program == "weaver" {
                let directory = root.join(args[args.len() - 1]);
                std::fs::create_dir_all(&directory).unwrap();
                std::fs::write(directory.join("rendered.rs"), "").unwrap();
            }
            if ran.len() == failing {
                Err(Error::Failed {
                    command: Invocation::new(root, program, args),
                    stderr: format!("{program} failed"),
                })
            } else {
                Ok(())
            }
        });
        (rendered, ran)
    }

    /// The program each invocation ran.
    fn programs(ran: &[Invocation]) -> Vec<String> {
        ran.iter()
            .map(|run| {
                run.to_string()
                    .split(' ')
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
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
        let weavers = OUTPUTS.len();
        for failing in 1..=weavers + 1 {
            let program = if failing > weavers {
                "rustfmt"
            } else {
                "weaver"
            };
            let (rendered, ran) = render(root.path(), failing);
            // The failed command's own error, naming the command it was.
            let error = rendered.err().unwrap();
            let expected = format!("{}\n{program} failed", ran[failing - 1].failed());
            assert_eq!(chain(&error), expected);
            assert_eq!(ran.len(), failing);
            assert!(!staged.exists(), "left by a failed {program}");
        }

        let (rendered, ran) = render(root.path(), 0);
        assert_eq!(
            programs(&ran),
            ["weaver", "weaver", "weaver", "weaver", "rustfmt"]
        );
        let rendered = rendered.unwrap();
        assert_eq!(rendered.0, staged);
        for output in &OUTPUTS {
            assert!(staged.join(output.staged).join("rendered.rs").is_file());
        }
        // One rustfmt over every Rust rendering's root file, in the stage.
        let rustfmt = ran[weavers].to_string();
        for output in &OUTPUTS {
            let root_file = output
                .formatted
                .map(|file| staged.join(output.staged).join(file).display().to_string());
            match root_file {
                Some(file) => assert!(rustfmt.contains(&file), "{rustfmt} lacks {file}"),
                None => assert!(!rustfmt.contains(output.staged), "{rustfmt}"),
            }
        }
        assert!(rustfmt.contains("--edition 2024"), "{rustfmt}");
        drop(rendered);
        assert!(!staged.exists(), "left once the rendering was done with");

        // Without the workspace's edition, rustfmt never runs.
        let manifest = root.path().join("lablet/Cargo.toml");
        std::fs::remove_file(&manifest).unwrap();
        let (rendered, ran) = render(root.path(), 0);
        let error = rendered.err().unwrap();
        assert!(
            matches!(&error, Error::File { verb: Verb::Read, path, .. } if *path == manifest),
            "{error:?}"
        );
        assert_eq!(programs(&ran), ["weaver", "weaver", "weaver", "weaver"]);
        assert!(!staged.exists(), "left by a failed read of the edition");
    }

    #[test]
    fn a_workspace_that_sets_no_edition_is_an_error_naming_its_manifest() {
        let root = TempDir::new("no-edition");
        root.write("Cargo.toml", "[workspace]\nmembers = []\n");
        let workspace = Workspace::load(root.path()).unwrap();
        let error = edition(&workspace).unwrap_err();
        assert_eq!(
            chain(&error),
            format!(
                "could not parse {}: it has no `workspace.package.edition`",
                root.path().join("Cargo.toml").display()
            )
        );
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
        for output in &OUTPUTS[..3] {
            assert!(!weaver_args(output, "stage").contains(&links));
        }
        assert!(weaver_args(&OUTPUTS[3], "stage").contains(&links));
    }

    /// Each module's signals are the folder's, it finds lablet's telemetry
    /// where its crate does, and the `Join` struct is generated once, into
    /// lablet-run, which the composition root's module uses from there.
    #[test]
    fn each_crate_module_is_rendered_from_its_registry_folder_for_its_crate() {
        let params = |output: &Output| -> Vec<String> {
            weaver_args(output, "stage")
                .windows(2)
                .filter(|pair| pair[0] == "--param")
                .map(|pair| pair[1].clone())
                .collect()
        };
        assert_eq!(
            params(&OUTPUTS[0]),
            [
                "crate_dir=application/run",
                "telemetry_path=crate::telemetry",
                "join=true",
            ]
        );
        assert_eq!(
            params(&OUTPUTS[1]),
            [
                "crate_dir=apps/lablet",
                "telemetry_path=lablet_run::telemetry",
                "join=false",
            ]
        );
        for output in &OUTPUTS[..2] {
            let folder = params(output)[0]
                .trim_start_matches("crate_dir=")
                .to_owned();
            assert!(
                repo_root().join(REGISTRY).join(&folder).is_dir(),
                "{folder} isn't a registry folder"
            );
            assert!(
                output.tree.ends_with("/src/telemetry/generated"),
                "{}",
                output.tree
            );
        }
    }

    #[test]
    fn rustfmt_is_given_the_edition_of_the_workspace() {
        let workspace = Workspace::load(&workspace_root()).unwrap();
        let edition = edition(&workspace).unwrap();
        let declared = std::fs::read_to_string(workspace_root().join("Cargo.toml")).unwrap();
        assert!(declared.contains(&format!("edition = \"{edition}\"")));
    }
}
