//! Where the repository is, and the parsed view of a Cargo workspace that the
//! lints and the floor checks read. Everything here takes the workspace root
//! as an argument, so the lints run over a fixture in a test as they do over
//! `lablet/`.
//!
//! The lints model only the Cargo features lablet uses. A workspace shaped any
//! other way (member globs, `exclude`, a root package, `[patch]`) is refused
//! with the [`unsupported`] diagnostic instead of being handled, so every
//! reader of a [`Workspace`] fails safe on it.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The repository root: the parent of xtask's manifest directory. `cargo run`,
/// which the `cargo xtask` alias is, names that directory at run time, so a
/// binary cargo reuses from another checkout (a copied checkout, a shared
/// target directory) still gates the one it was started in. The value compiled
/// in is only the fallback for a binary that is run directly.
pub fn repo_root() -> PathBuf {
    root_from(std::env::var_os("CARGO_MANIFEST_DIR"))
}

/// The root given the manifest directory cargo named at run time, if it did.
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

/// The diagnostic for a Cargo feature lablet does not use, which the lints
/// refuse rather than model.
pub fn unsupported(feature: &str) -> String {
    format!(
        "{feature} is not supported by lablet's lints, which model only the Cargo features \
         lablet uses; remove it, or extend xtask deliberately to support it"
    )
}

/// Whether a raw manifest value is a table holding `workspace = true`: a
/// dependency, a `[package]` key, or the `[lints]` table, inherited.
pub fn inherits_workspace(value: &toml::Value) -> bool {
    value.get("workspace").and_then(toml::Value::as_bool) == Some(true)
}

/// A crate name with `-` folded to `_`, the form rustc sees, so the two
/// spellings of one name compare equal.
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
    #[serde(flatten)]
    tables: DependencyTables,
    #[serde(default)]
    target: BTreeMap<String, DependencyTables>,
}

impl Manifest {
    /// Every non-empty dependency table, top-level first, then each
    /// `[target.*]` section's.
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
    /// Reads the workspace rooted at `root`. The error names the file: a
    /// manifest that is missing or does not parse, a member without a
    /// `[package]`, or a shape the lints do not support.
    pub fn load(root: &Path) -> Result<Self, String> {
        let manifest_path = root.join("Cargo.toml");
        let file = manifest_path.display().to_string();
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
        if let Some(feature) = refused {
            return Err(format!("{file}: {}", unsupported(&feature)));
        }
        let table: WorkspaceTable = table
            .cloned()
            .ok_or_else(|| format!("{file} has no [workspace] table"))?
            .try_into()
            .map_err(|e| format!("could not parse {file}: {e}"))?;

        let mut members = Vec::new();
        for path in table.members {
            // A crate's ring is read from this text, so it must be the path.
            let literal = !path.contains(['*', '?', '[', ']', '{', '}'])
                && path
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..");
            if !literal {
                let feature = format!(
                    "workspace member `{path}` (a glob, a `.` or `..` component, an absolute \
                     path, or a trailing `/`, where a literal relative directory is wanted)"
                );
                return Err(format!("{file}: {}", unsupported(&feature)));
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

    /// The member listed at exactly `path`, which is how an internal entry of
    /// `[workspace.dependencies]` names its crate.
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

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))
}

fn parse<T: serde::de::DeserializeOwned>(path: &Path, text: &str) -> Result<T, String> {
    toml::from_str(text).map_err(|e| format!("could not parse {}: {e}", path.display()))
}

#[cfg(test)]
pub mod fixture {
    //! A throwaway workspace on disk, for the lints' tests.

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::Workspace;

    /// Asserts a lint found one finding per phrase, in order, each holding
    /// its phrase.
    #[track_caller]
    pub fn assert_findings(found: &[String], phrases: &[&str]) {
        assert_eq!(found.len(), phrases.len(), "{found:#?}");
        for (finding, phrase) in found.iter().zip(phrases) {
            assert!(finding.contains(phrase), "`{phrase}` is not in: {finding}");
        }
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

    /// A workspace under construction: a `[workspace.dependencies]` body and
    /// the members added so far.
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

    #[test]
    fn the_repository_root_holds_xtask_and_the_workspace() {
        assert!(xtask_manifest().is_file());
        assert!(workspace_root().join("Cargo.toml").is_file());
    }

    #[test]
    fn the_manifest_directory_cargo_names_at_run_time_wins_over_the_compiled_one() {
        // A binary built in one checkout and reused in another must gate the
        // one it runs in.
        assert_eq!(
            root_from(Some("/elsewhere/checkout/xtask".into())),
            PathBuf::from("/elsewhere/checkout")
        );
        assert_eq!(
            root_from(None),
            Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
        );
    }

    /// Loads a workspace whose root manifest is `root`, with a package `x` in
    /// each of `on_disk`.
    fn load(root: &str, on_disk: &[&str]) -> Result<Workspace, String> {
        let dir = TempDir::new("load");
        dir.write("Cargo.toml", root);
        for member in on_disk {
            dir.write(&format!("{member}/Cargo.toml"), "[package]\nname = \"x\"\n");
        }
        Workspace::load(dir.path())
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
            let error = load(&root, &["crates/domain/x"]).unwrap_err();
            assert!(
                error.contains(&format!("workspace member `{path}`")),
                "{error}"
            );
            assert!(
                error.contains("is not supported by lablet's lints"),
                "{error}"
            );
        }
        assert!(
            load(
                "[workspace]\nmembers = [\"crates/domain/x\"]\n",
                &["crates/domain/x"]
            )
            .is_ok()
        );
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
            let error = load(root, &[]).unwrap_err();
            assert!(error.contains(feature), "{error}");
            assert!(error.contains("extend xtask deliberately"), "{error}");
        }
    }

    #[test]
    fn a_member_with_no_manifest_or_no_package_is_an_error_naming_the_file() {
        let error = load("[workspace]\nmembers = [\"crates/gone\"]\n", &[]).unwrap_err();
        assert!(error.contains("crates/gone/Cargo.toml"), "{error}");

        let dir = TempDir::new("virtual");
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n");
        dir.write("a/Cargo.toml", "[dependencies]\n");
        let error = Workspace::load(dir.path()).unwrap_err();
        assert!(error.contains("a/Cargo.toml"), "{error}");
        assert!(error.contains("package"), "{error}");
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
            "[package]\nname = \"lablet-telemetry-otel\"\n",
        );
        let workspace = Workspace::load(dir.path()).unwrap();
        assert!(workspace.member_named("lablet_telemetry_otel").is_some());
        assert!(workspace.member_named("lablet-telemetry").is_none());
        assert!(workspace.member_at("a").is_some());
        assert!(workspace.member_at("./a").is_none());
    }
}
