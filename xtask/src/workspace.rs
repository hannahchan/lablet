//! Where the repository is, and the parsed view of a Cargo workspace that the
//! lints and the floor checks read. The lints model only the Cargo features
//! lablet uses; a workspace shaped any other way (member globs, `exclude`, a
//! root package, `[patch]`) is refused, so every reader fails safe on it.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Malformed, Verb};

/// The repository root: the parent of xtask's manifest directory. `cargo run`
/// names that directory at run time, so a binary cargo reuses from another
/// checkout (a shared target directory) still gates the one it was started
/// in. The compiled-in value is the fallback for a binary run directly.
pub fn repo_root() -> PathBuf {
    root_from(std::env::var_os("CARGO_MANIFEST_DIR"))
}

fn root_from(runtime_manifest_dir: Option<std::ffi::OsString>) -> PathBuf {
    let manifest_dir = runtime_manifest_dir
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    manifest_dir
        .parent()
        .map_or_else(|| manifest_dir.clone(), Path::to_path_buf)
}

/// The Cargo workspace, one level below the repository root.
pub fn workspace_root() -> PathBuf {
    repo_root().join("lablet")
}

/// xtask's own manifest, for the passes that cover this crate.
pub fn xtask_manifest() -> PathBuf {
    repo_root().join("xtask").join("Cargo.toml")
}

/// The diagnostic for a Cargo feature the lints refuse rather than model.
pub fn unsupported(feature: &str) -> String {
    format!(
        "{feature} is not supported by lablet's lints, which model only the Cargo features \
         lablet uses; remove it, or extend xtask deliberately to support it"
    )
}

/// Whether a raw manifest value is a table holding `workspace = true`.
pub fn inherits_workspace(value: &toml::Value) -> bool {
    value.get("workspace").and_then(toml::Value::as_bool) == Some(true)
}

/// `-` folded to `_`, so the two spellings of one crate name compare equal.
pub fn normalise(name: &str) -> String {
    name.replace('-', "_")
}

/// The `name = spec` entries of one dependency table, specs left raw.
pub type Dependencies = BTreeMap<String, toml::Value>;

/// Cargo's three dependency tables, at the top level or under `[target.*]`.
#[derive(Debug, Default, Deserialize)]
struct DependencyTables {
    #[serde(default)]
    dependencies: Dependencies,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: Dependencies,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: Dependencies,
}

/// One non-empty dependency table of a manifest.
#[derive(Debug)]
pub struct DependencySection<'a> {
    /// The table header in cargo's usual spelling, for diagnostics.
    pub header: String,
    /// Whether this is a `dev-dependencies` table: tests, examples, benches.
    pub dev: bool,
    /// The entries.
    pub entries: &'a Dependencies,
}

/// `[package]`: the name, and every other key left raw.
#[derive(Debug, Deserialize)]
pub struct Package {
    /// The package name, which is how crates are matched.
    pub name: String,
    /// The other keys; the lint asks whether each inherits, not what it holds.
    #[serde(flatten)]
    pub keys: BTreeMap<String, toml::Value>,
}

/// A member's manifest, reduced to what the lints read.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// `[package]`. A member without one does not parse.
    pub package: Package,
    /// `[lints]`, raw.
    pub lints: Option<toml::Value>,
    /// `[[bin]]`, raw: each one is a binary target.
    #[serde(default)]
    pub bin: Vec<toml::Value>,
    #[serde(flatten)]
    tables: DependencyTables,
    #[serde(default)]
    target: BTreeMap<String, DependencyTables>,
}

impl Manifest {
    /// Every non-empty dependency table, `[target.*]` sections included.
    pub fn dependency_sections(&self) -> Vec<DependencySection<'_>> {
        let scopes = std::iter::once((String::new(), &self.tables)).chain(
            self.target
                .iter()
                .map(|(target, tables)| (format!("target.'{target}'."), tables)),
        );
        let mut sections = Vec::new();
        for (scope, tables) in scopes {
            for (name, dev, entries) in [
                ("dependencies", false, &tables.dependencies),
                ("dev-dependencies", true, &tables.dev_dependencies),
                ("build-dependencies", false, &tables.build_dependencies),
            ] {
                if !entries.is_empty() {
                    sections.push(DependencySection {
                        header: format!("[{scope}{name}]"),
                        dev,
                        entries,
                    });
                }
            }
        }
        sections
    }
}

/// One workspace member.
#[derive(Debug)]
pub struct Member {
    /// The member's directory as `[workspace].members` lists it.
    pub path: String,
    /// The package name.
    pub name: String,
    /// The manifest parsed.
    pub manifest: Manifest,
}

/// The `[workspace]` keys that are read.
#[derive(Debug, Deserialize)]
struct WorkspaceTable {
    members: Vec<String>,
    #[serde(default)]
    dependencies: Dependencies,
}

/// A Cargo workspace read from disk: the root manifest and every member's.
#[derive(Debug)]
pub struct Workspace {
    /// The directory holding the root `Cargo.toml`.
    pub root: PathBuf,
    /// The root manifest as written, for the check that reads comments.
    pub text: String,
    /// The root manifest as a raw document, for the tables compared whole.
    pub document: toml::Value,
    /// `[workspace.dependencies]`, which members inherit with `workspace = true`.
    pub dependencies: Dependencies,
    /// Every member, in the order listed.
    pub members: Vec<Member>,
}

impl Workspace {
    /// Reads the workspace rooted at `root`. The error names the file.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let manifest_path = root.join("Cargo.toml");
        let text = read(&manifest_path)?;
        let document: toml::Value = parse(&manifest_path, &text)?;
        let table = document.get("workspace");
        let refused = ["package", "patch", "replace"]
            .into_iter()
            .filter(|key| document.get(key).is_some())
            .map(|key| format!("a `[{key}]` table in the workspace root"))
            .chain(
                ["exclude", "default-members"]
                    .into_iter()
                    .filter(|key| table.is_some_and(|table| table.get(key).is_some()))
                    .map(|key| format!("`[workspace] {key}`")),
            )
            .next();
        let refuse = |feature| Error::Unsupported {
            path: manifest_path.clone(),
            feature,
        };
        if let Some(feature) = refused {
            return Err(refuse(feature));
        }
        let table: WorkspaceTable = table
            .cloned()
            .ok_or(Malformed::Lacks("`[workspace]` table"))
            .map_err(Error::parse(&manifest_path))?
            .try_into()
            .map_err(Error::parse(&manifest_path))?;

        let mut members = Vec::new();
        for path in table.members {
            // A crate's ring is read from this text, so it must be the path.
            let literal = !path.contains(['*', '?', '[', ']', '{', '}'])
                && path
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..");
            if !literal {
                return Err(refuse(format!(
                    "workspace member `{path}` (a glob, a `.` or `..` component, an absolute \
                     path, or a trailing `/`, where a literal relative directory is wanted)"
                )));
            }
            let member_manifest = root.join(&path).join("Cargo.toml");
            let manifest: Manifest = parse(&member_manifest, &read(&member_manifest)?)?;
            members.push(Member {
                path,
                name: manifest.package.name.clone(),
                manifest,
            });
        }
        Ok(Self {
            root: root.to_path_buf(),
            text,
            document,
            dependencies: table.dependencies,
            members,
        })
    }

    /// The member listed at exactly `path`.
    pub fn member_at(&self, path: &str) -> Option<&Member> {
        self.members.iter().find(|member| member.path == path)
    }

    /// The member whose package name is `name`, treating `-` and `_` alike.
    pub fn member_named(&self, name: &str) -> Option<&Member> {
        let wanted = normalise(name);
        self.members
            .iter()
            .find(|member| normalise(&member.name) == wanted)
    }
}

/// The text of the file at `path`. The error names the file.
pub fn read(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(Error::file(Verb::Read, path))
}

fn parse<T: DeserializeOwned>(path: &Path, text: &str) -> Result<T, Error> {
    toml::from_str(text).map_err(Error::parse(path))
}

/// The JSON file at `path`, read into a `T`. The error names the file.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, Error> {
    serde_json::from_str(&read(path)?).map_err(Error::parse(path))
}

#[cfg(test)]
pub mod fixture {
    //! A throwaway workspace or repository on disk, for the tests.

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::Workspace;

    /// Asserts one finding per phrase, in order, each holding its phrase.
    #[track_caller]
    pub fn assert_findings(found: &[String], phrases: &[&str]) {
        assert_eq!(found.len(), phrases.len(), "{found:#?}");
        for (finding, phrase) in found.iter().zip(phrases) {
            assert!(finding.contains(phrase), "`{phrase}` is not in: {finding}");
        }
    }

    /// Git pinned to a scratch repository and cut off from the developer's own
    /// configuration. Clearing the inherited variables isn't enough on its own:
    /// under a hook, a command that missed that step would reach the real
    /// repository, so the repository is named outright.
    pub fn scratch_git(root: &Path, args: &[&str]) -> String {
        scratch_git_with(root, args, &[])
    }

    /// [`scratch_git`], with `env` set too for git and what it runs, such as
    /// a hook.
    pub fn scratch_git_with(root: &Path, args: &[&str], env: &[(&str, &Path)]) -> String {
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
        for variable in crate::process::GIT_REPOSITORY_ENV {
            command.env_remove(variable);
        }
        let output = command
            .env("GIT_DIR", root.join(".git"))
            .env("GIT_WORK_TREE", root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Writes settings into a scratch repository's own config that change
    /// what git writes by default, so that each command there has to say what
    /// it relies on: copy and rename detection, colour, and an external diff
    /// tool and a text conversion that both fail. `prefixes` is
    /// `diff.noprefix` or `diff.mnemonicPrefix`, one at a time because the
    /// first overrides the second.
    pub fn defy_git_defaults(root: &Path, prefixes: &str) {
        for setting in [
            ["diff.renames", "copies"],
            ["color.diff", "always"],
            ["diff.external", "false"],
            ["diff.defiant.textconv", "false"],
            [prefixes, "true"],
        ] {
            scratch_git(root, &[&["config"], &setting[..]].concat());
        }
        // The text conversion applies to a file its attributes name.
        let info = root.join(".git/info");
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("attributes"), "* diff=defiant\n").unwrap();
    }

    /// A directory under the system temporary directory, removed on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        /// Creates a directory no other test or process shares.
        pub fn new(tag: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let unique = format!(
                "lablet-xtask-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// The directory.
        pub fn path(&self) -> &Path {
            &self.0
        }

        /// Writes `text` at `relative`, creating the directories above it.
        pub fn write(&self, relative: &str, text: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A workspace under construction.
    pub struct FixtureWorkspace {
        dir: TempDir,
        workspace_dependencies: String,
        members: Vec<String>,
    }

    impl FixtureWorkspace {
        /// A workspace whose `[workspace.dependencies]` table holds `body`.
        pub fn new(workspace_dependencies: &str) -> Self {
            Self {
                dir: TempDir::new("workspace"),
                workspace_dependencies: workspace_dependencies.to_owned(),
                members: Vec::new(),
            }
        }

        /// Adds a member at `path` named `name`; `tables` is the manifest
        /// text after the `[package]` and `[lints]` tables.
        pub fn member(mut self, path: &str, name: &str, tables: &str) -> Self {
            let manifest = format!(
                "[package]\nname = \"{name}\"\nversion.workspace = true\nedition.workspace = true\n\
                 rust-version.workspace = true\nlicense.workspace = true\n\n\
                 [lints]\nworkspace = true\n\n{tables}"
            );
            self.dir.write(&format!("{path}/Cargo.toml"), &manifest);
            self.members.push(format!("\"{path}\""));
            self
        }

        /// Writes a file of the workspace, such as a member's source.
        pub fn write(&self, relative: &str, text: &str) {
            self.dir.write(relative, text);
        }

        /// Writes the root manifest and reads the whole workspace back.
        pub fn load(&self) -> Workspace {
            let root = format!(
                "[workspace]\nresolver = \"3\"\nmembers = [{}]\n\n\
                 [workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                 rust-version = \"1.96\"\nlicense = \"MIT OR Apache-2.0\"\n\n\
                 [workspace.dependencies]\n{}\n",
                self.members.join(", "),
                self.workspace_dependencies
            );
            self.dir.write("Cargo.toml", &root);
            Workspace::load(self.dir.path()).unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::TempDir;
    use super::*;
    use crate::error::chain;

    #[test]
    fn the_repository_root_holds_xtask_and_the_workspace() {
        assert!(xtask_manifest().is_file());
        assert!(workspace_root().join("Cargo.toml").is_file());
    }

    #[test]
    fn the_manifest_directory_cargo_names_at_run_time_wins_over_the_compiled_one() {
        assert_eq!(
            root_from(Some("/elsewhere/checkout/xtask".into())),
            PathBuf::from("/elsewhere/checkout")
        );
        assert_eq!(
            root_from(None),
            Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
        );
    }

    /// Why the workspace whose root manifest is `root`, with a package `x`
    /// in each of `on_disk`, is refused, and the manifest it named.
    fn refusal(root: &str, on_disk: &[&str]) -> (Error, PathBuf) {
        let dir = TempDir::new("load");
        dir.write("Cargo.toml", root);
        for member in on_disk {
            dir.write(&format!("{member}/Cargo.toml"), "[package]\nname = \"x\"\n");
        }
        let error = Workspace::load(dir.path()).unwrap_err();
        (error, dir.path().join("Cargo.toml"))
    }

    #[test]
    fn a_member_that_is_not_a_literal_relative_directory_is_refused() {
        // Each reaches a real crate on disk while its text names another
        // ring, or none, or more than one crate.
        for path in [
            "crates/domain/*",
            "crates/domain/mode?",
            "crates/domain/[mx]",
            "crates/{domain,application}/x",
            "apps/../crates/domain/x",
            "./crates/domain/x",
            "crates//domain/x",
            "crates/domain/x/",
            "/crates/domain/x",
        ] {
            let root = format!("[workspace]\nmembers = [\"{path}\"]\n");
            let (error, manifest) = refusal(&root, &["crates/domain/x"]);
            assert!(matches!(error, Error::Unsupported { .. }), "{error:?}");
            let error = chain(&error);
            let named = format!("{}: workspace member `{path}`", manifest.display());
            assert!(error.starts_with(&named), "{error}");
            assert!(
                error.contains("is not supported by lablet's lints"),
                "{error}"
            );
        }
        let dir = TempDir::new("load");
        dir.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/domain/x\"]\n",
        );
        dir.write("crates/domain/x/Cargo.toml", "[package]\nname = \"x\"\n");
        Workspace::load(dir.path()).unwrap();
    }

    #[test]
    fn a_workspace_shape_lablet_does_not_use_is_refused() {
        for (feature, root) in [
            (
                "a `[package]` table",
                "[package]\nname = \"root\"\n\n[workspace]\nmembers = []\n",
            ),
            (
                "a `[patch]` table",
                "[workspace]\nmembers = []\n\n[patch.crates-io]\nserde = { path = \"vendor/serde\" }\n",
            ),
            (
                "a `[replace]` table",
                "[workspace]\nmembers = []\n\n[replace]\n\"serde:1.0.0\" = { path = \"vendor/serde\" }\n",
            ),
            (
                "`[workspace] exclude`",
                "[workspace]\nmembers = []\nexclude = [\"crates/old\"]\n",
            ),
            (
                "`[workspace] default-members`",
                "[workspace]\nmembers = []\ndefault-members = []\n",
            ),
        ] {
            let (error, manifest) = refusal(root, &[]);
            assert!(matches!(error, Error::Unsupported { .. }), "{error:?}");
            let error = chain(&error);
            let named = format!("{}: {feature}", manifest.display());
            assert!(error.starts_with(&named), "{error}");
            assert!(error.contains("extend xtask deliberately"), "{error}");
        }
    }

    #[test]
    fn a_root_manifest_without_a_workspace_table_is_an_error_naming_it() {
        let (error, manifest) = refusal("[lints]\n", &[]);
        assert!(matches!(error, Error::Parse { .. }), "{error:?}");
        assert_eq!(
            chain(&error),
            format!(
                "could not parse {}: it has no `[workspace]` table",
                manifest.display()
            )
        );
    }

    #[test]
    fn a_member_with_no_manifest_or_no_package_is_an_error_naming_the_file() {
        let (error, root) = refusal("[workspace]\nmembers = [\"crates/gone\"]\n", &[]);
        let gone = root.with_file_name("crates/gone/Cargo.toml");
        let Error::File { verb, path, source } = &error else {
            panic!("{error:?}");
        };
        assert_eq!((*verb, path), (Verb::Read, &gone));
        assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        let expected = format!("could not read {}: {source}", gone.display());
        assert_eq!(chain(&error), expected);

        let dir = TempDir::new("virtual");
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n");
        dir.write("a/Cargo.toml", "[dependencies]\n");
        let error = Workspace::load(dir.path()).unwrap_err();
        let Error::Parse { path, .. } = &error else {
            panic!("{error:?}");
        };
        assert_eq!(*path, dir.path().join("a/Cargo.toml"));
        let error = chain(&error);
        assert!(error.contains("missing field `package`"), "{error}");
    }

    #[test]
    fn a_json_file_is_read_whole_and_a_bad_one_is_named() {
        let dir = TempDir::new("json");
        dir.write("good.json", "[1, 2]");
        dir.write("bad.json", "[1,");
        let good: Vec<u8> = read_json(&dir.path().join("good.json")).unwrap();
        assert_eq!(good, [1, 2]);
        let bad = dir.path().join("bad.json");
        let error = read_json::<Vec<u8>>(&bad).unwrap_err();
        assert!(
            matches!(&error, Error::Parse { path, .. } if *path == bad),
            "{error:?}"
        );
        let prefix = format!("could not parse {}: ", bad.display());
        assert!(chain(&error).starts_with(&prefix), "{}", chain(&error));
        let absent = dir.path().join("absent.json");
        let error = read_json::<Vec<u8>>(&absent).unwrap_err();
        assert!(
            matches!(&error, Error::File { verb: Verb::Read, path, .. } if *path == absent),
            "{error:?}"
        );
    }

    #[test]
    fn dependency_sections_cover_target_tables_and_name_their_headers() {
        let manifest: Manifest = toml::from_str(
            "[package]\nname = \"x\"\n\n[dependencies]\na.workspace = true\n\n\
             [dev-dependencies]\nb = \"1\"\n\n\
             [target.'cfg(unix)'.build-dependencies]\nc = { version = \"1\" }\n",
        )
        .unwrap();
        let sections = manifest.dependency_sections();
        let headers: Vec<(&str, bool)> = sections
            .iter()
            .map(|section| (section.header.as_str(), section.dev))
            .collect();
        assert_eq!(
            headers,
            [
                ("[dependencies]", false),
                ("[dev-dependencies]", true),
                ("[target.'cfg(unix)'.build-dependencies]", false),
            ]
        );
        // The dotted and the inline spelling both read as inherited.
        assert!(inherits_workspace(&sections[0].entries["a"]));
        assert!(!inherits_workspace(&sections[1].entries["b"]));
    }

    #[test]
    fn members_are_found_by_listed_path_and_by_name_in_either_spelling() {
        let dir = TempDir::new("names");
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n");
        dir.write(
            "a/Cargo.toml",
            "[package]\nname = \"lablet-sample-adapter\"\n",
        );
        let workspace = Workspace::load(dir.path()).unwrap();
        assert!(workspace.member_named("lablet_sample_adapter").is_some());
        assert!(workspace.member_named("lablet-sample").is_none());
        assert!(workspace.member_at("a").is_some());
        assert!(workspace.member_at("./a").is_none());
    }
}
