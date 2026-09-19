//! Where the repository is, and the parsed view of a Cargo workspace that the
//! lints and the floor checks read. Everything here takes the workspace root
//! as an argument, so the lints run over a fixture in a test as they do over
//! `lablet/`.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The repository root: the parent of xtask's manifest directory. `cargo run`,
/// which the `cargo xtask` alias is, sets `CARGO_MANIFEST_DIR` for the binary
/// it starts, so the root is the checkout the command was started in and does
/// not depend on the directory below it. Cargo does not rebuild xtask when a
/// checkout is copied or moved, or when two checkouts share a target
/// directory, so the value compiled in is only the fallback for a binary that
/// is run directly.
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

/// The `name = spec` entries of one dependency table.
pub type Dependencies = BTreeMap<String, DependencySpec>;

/// One dependency as a manifest declares it.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// A bare version requirement: `foo = "1"`.
    Version(String),
    /// A table, inline or dotted: `foo = { path = ".." }`, `foo.workspace = true`.
    Table(DependencyTable),
}

/// The keys of a dependency table that the lints read.
#[derive(Debug, Deserialize)]
pub struct DependencyTable {
    /// The version requirement, when one is given.
    pub version: Option<String>,
    /// A path dependency's location, relative to the declaring manifest.
    pub path: Option<String>,
    /// The real crate name behind a renamed dependency key.
    pub package: Option<String>,
    /// A git source, which is never an exact pin.
    pub git: Option<String>,
    /// A registry other than crates.io, by its configured name.
    pub registry: Option<String>,
    /// A registry other than crates.io, by its index URL.
    #[serde(rename = "registry-index")]
    pub registry_index: Option<String>,
    /// `workspace = true`: the entry is inherited from `[workspace.dependencies]`.
    #[serde(default)]
    pub workspace: bool,
}

impl DependencySpec {
    /// The version requirement, when one is given.
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Version(version) => Some(version),
            Self::Table(table) => table.version.as_deref(),
        }
    }

    /// A path dependency's location.
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Version(_) => None,
            Self::Table(table) => table.path.as_deref(),
        }
    }

    /// The real crate name behind a renamed dependency key, when declared.
    pub fn package(&self) -> Option<&str> {
        match self {
            Self::Version(_) => None,
            Self::Table(table) => table.package.as_deref(),
        }
    }

    /// Whether the entry comes from a git repository.
    pub fn is_git(&self) -> bool {
        matches!(self, Self::Table(table) if table.git.is_some())
    }

    /// Whether the entry names a registry other than crates.io.
    pub fn is_alternative_registry(&self) -> bool {
        matches!(self, Self::Table(table) if table.registry.is_some() || table.registry_index.is_some())
    }

    /// Whether the entry is inherited from `[workspace.dependencies]`.
    pub fn inherits_workspace(&self) -> bool {
        matches!(self, Self::Table(table) if table.workspace)
    }
}

/// Which of cargo's three dependency tables an entry sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyKind {
    /// `[dependencies]`: ships with the crate.
    Normal,
    /// `[build-dependencies]`: runs in the crate's build script.
    Build,
    /// `[dev-dependencies]`: tests, examples, and benches only.
    Dev,
}

/// One dependency table of a manifest, top-level or under `[target.*]`.
#[derive(Debug)]
pub struct DependencySection<'a> {
    /// The table header in cargo's usual spelling, for diagnostics.
    pub header: String,
    /// The table's key path: `["target", "cfg(unix)", "dependencies"]`.
    pub path: Vec<String>,
    /// Which of the three tables this is.
    pub kind: DependencyKind,
    /// The entries.
    pub entries: &'a Dependencies,
}

/// The dependency tables of one `[target.'cfg(..)']` section.
#[derive(Debug, Default, Deserialize)]
pub struct TargetTables {
    #[serde(default)]
    dependencies: Dependencies,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: Dependencies,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: Dependencies,
}

/// The `[package]` keys the lints read. The inheritable ones stay raw values:
/// the lint asks whether each is `{ workspace = true }`, not what it holds.
#[derive(Debug, Deserialize)]
pub struct Package {
    /// The package name, which is how crates are matched.
    pub name: String,
    /// `version`, raw.
    pub version: Option<toml::Value>,
    /// `edition`, raw.
    pub edition: Option<toml::Value>,
    /// `rust-version`, raw.
    #[serde(rename = "rust-version")]
    pub rust_version: Option<toml::Value>,
    /// `license`, raw.
    pub license: Option<toml::Value>,
}

/// The `[lints]` table of a member: only the inheritance flag matters.
#[derive(Debug, Deserialize)]
pub struct LintsTable {
    /// `workspace = true`.
    #[serde(default)]
    pub workspace: bool,
}

/// A package manifest, reduced to what the lints read.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// `[package]`.
    pub package: Option<Package>,
    /// `[lints]`.
    pub lints: Option<LintsTable>,
    #[serde(default)]
    dependencies: Dependencies,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: Dependencies,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: Dependencies,
    #[serde(default)]
    target: BTreeMap<String, TargetTables>,
    /// `[[test]]`: integration test targets declared by hand.
    #[serde(default, rename = "test")]
    pub test_targets: Vec<toml::Value>,
}

impl Manifest {
    /// Parses a manifest; the error is the parser's own message.
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Every non-empty dependency table, top-level first, then each
    /// `[target.*]` section's.
    pub fn dependency_sections(&self) -> Vec<DependencySection<'_>> {
        let mut sections = Vec::new();
        push_sections(
            &mut sections,
            &[],
            [
                &self.dependencies,
                &self.dev_dependencies,
                &self.build_dependencies,
            ],
        );
        for (target, tables) in &self.target {
            push_sections(
                &mut sections,
                &["target", target],
                [
                    &tables.dependencies,
                    &tables.dev_dependencies,
                    &tables.build_dependencies,
                ],
            );
        }
        sections
    }
}

/// Appends the non-empty tables of one scope, given as normal, dev, build.
fn push_sections<'a>(
    sections: &mut Vec<DependencySection<'a>>,
    scope: &[&str],
    tables: [&'a Dependencies; 3],
) {
    let prefix = match scope {
        [first, target] => format!("{first}.'{target}'."),
        _ => String::new(),
    };
    let kinds = [
        (DependencyKind::Normal, "dependencies"),
        (DependencyKind::Dev, "dev-dependencies"),
        (DependencyKind::Build, "build-dependencies"),
    ];
    for ((kind, name), entries) in kinds.into_iter().zip(tables) {
        if !entries.is_empty() {
            sections.push(DependencySection {
                header: format!("[{prefix}{name}]"),
                path: scope
                    .iter()
                    .chain([&name])
                    .map(|part| (*part).to_owned())
                    .collect(),
                kind,
                entries,
            });
        }
    }
}

/// The `[workspace]` table of the root manifest.
#[derive(Debug, Deserialize)]
pub struct WorkspaceTable {
    /// Member paths, each listed literally. `exclude` is not read: cargo lets
    /// a literally listed member win over it, so it never removes one.
    pub members: Vec<String>,
    /// `[workspace.dependencies]`, which members inherit with `workspace = true`.
    #[serde(default)]
    pub dependencies: Dependencies,
}

#[derive(Debug, Deserialize)]
struct RootManifest {
    workspace: WorkspaceTable,
}

/// One workspace member.
#[derive(Debug)]
pub struct Member {
    /// The member's directory, relative to the workspace root, with `/`.
    pub path: String,
    /// The package name.
    pub name: String,
    /// The manifest as written, for the checks that read comments.
    pub text: String,
    /// The manifest parsed.
    pub manifest: Manifest,
}

/// A Cargo workspace read from disk: the root manifest and every member's.
#[derive(Debug)]
pub struct Workspace {
    /// The directory holding the root `Cargo.toml`.
    pub root: PathBuf,
    /// The root manifest as written.
    pub text: String,
    /// The root manifest as a raw document, for the tables compared whole.
    pub document: toml::Value,
    /// The `[workspace]` table.
    pub table: WorkspaceTable,
    /// Every member, in path order.
    pub members: Vec<Member>,
}

/// Why a workspace could not be read. The message names the file.
#[derive(Debug, PartialEq, Eq)]
pub struct LoadError(pub String);

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Workspace {
    /// Reads the workspace rooted at `root`. Any manifest that is missing,
    /// unparsable, or without a `[package]` name is an error: a lint that
    /// skipped such a member would pass it unread. A root manifest with a
    /// `[package]` table is a member too, as it is to cargo, at the empty path.
    pub fn load(root: &Path) -> Result<Self, LoadError> {
        let manifest_path = root.join("Cargo.toml");
        let text = read(&manifest_path)?;
        let document: toml::Value = parse(&manifest_path, &text)?;
        let table = parse::<RootManifest>(&manifest_path, &text)?.workspace;
        let mut members = Vec::new();
        let root_manifest: Manifest = parse(&manifest_path, &text)?;
        if let Some(package) = &root_manifest.package {
            members.push(Member {
                path: String::new(),
                name: package.name.clone(),
                text: text.clone(),
                manifest: root_manifest,
            });
        }
        for path in expand_members(root, &table.members)? {
            let member_manifest = root.join(&path).join("Cargo.toml");
            let member_text = read(&member_manifest)?;
            let manifest: Manifest = parse(&member_manifest, &member_text)?;
            let Some(package) = &manifest.package else {
                return Err(LoadError(format!(
                    "{} has no [package] table",
                    member_manifest.display()
                )));
            };
            members.push(Member {
                path,
                name: package.name.clone(),
                text: member_text,
                manifest,
            });
        }
        Ok(Self {
            root: root.to_path_buf(),
            text,
            document,
            table,
            members,
        })
    }

    /// The member a path dependency points at. `path` is read as cargo reads
    /// it, relative to the directory `from` of the manifest declaring it
    /// (itself relative to the workspace root; empty for the root manifest),
    /// with `.` and `..` folded by name. `None` when the path is absolute,
    /// leaves the workspace, or is no member's directory, whatever the
    /// package there is called.
    pub fn member_at(&self, from: &str, path: &str) -> Option<&Member> {
        let resolved = resolve_path(from, path)?;
        self.members.iter().find(|member| member.path == resolved)
    }

    /// The member whose package name is `name`, treating `-` and `_` alike.
    pub fn member_named(&self, name: &str) -> Option<&Member> {
        let wanted = normalise(name);
        self.members
            .iter()
            .find(|member| normalise(&member.name) == wanted)
    }
}

/// A crate name with `-` folded to `_`, the form rustc sees, so the two
/// spellings of one name compare equal.
pub fn normalise(name: &str) -> String {
    name.replace('-', "_")
}

fn read(path: &Path) -> Result<String, LoadError> {
    std::fs::read_to_string(path)
        .map_err(|e| LoadError(format!("could not read {}: {e}", path.display())))
}

fn parse<T: serde::de::DeserializeOwned>(path: &Path, text: &str) -> Result<T, LoadError> {
    toml::from_str(text).map_err(|e| LoadError(format!("could not parse {}: {e}", path.display())))
}

/// `path` as a manifest in directory `from` declares it, relative to the
/// workspace root, or `None` when it is absolute or climbs out of the root.
fn resolve_path(from: &str, path: &str) -> Option<String> {
    if Path::new(path).is_absolute() {
        return None;
    }
    let mut segments: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            name => segments.push(name),
        }
    }
    Some(segments.join("/"))
}

/// The member directories `[workspace].members` lists, sorted and without
/// duplicates. A crate's ring is read from this path, so each entry must be
/// the plain relative path of a real directory holding a `Cargo.toml`: a
/// wildcard, a `.` or `..` segment, or a symlink on the way would let the
/// path name one ring while cargo builds a crate that lives in another.
fn expand_members(root: &Path, listed: &[String]) -> Result<Vec<String>, LoadError> {
    let mut members = Vec::new();
    for entry in listed {
        let member = entry.trim_end_matches('/');
        if member.contains(['*', '?', '[', ']']) {
            return Err(LoadError(format!(
                "workspace member `{entry}` is a wildcard pattern; list workspace members \
                 literally, since xtask does not expand globs and would lint none of the crates \
                 cargo finds"
            )));
        }
        let plain = !Path::new(member).is_absolute()
            && !member.contains('\\')
            && member
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if !plain {
            return Err(LoadError(format!(
                "workspace member `{entry}` is not a plain relative path; a crate's ring is \
                 read from this path, so write it without `.`, `..`, or empty segments"
            )));
        }
        if !root.join(member).join("Cargo.toml").is_file() {
            return Err(LoadError(format!(
                "workspace member `{member}` has no Cargo.toml under {}",
                root.display()
            )));
        }
        members.push(member.to_owned());
    }
    members.sort();
    members.dedup();

    let real_root = root
        .canonicalize()
        .map_err(|e| LoadError(format!("could not resolve {}: {e}", root.display())))?;
    for member in &members {
        let directory = root.join(member);
        let real = directory
            .canonicalize()
            .map_err(|e| LoadError(format!("could not resolve {}: {e}", directory.display())))?;
        if real != real_root.join(member) {
            return Err(LoadError(format!(
                "workspace member `{member}` resolves to {}, not to its own path; a crate's \
                 ring is read from its path, so a member may not sit behind a symlink",
                real.display()
            )));
        }
    }
    Ok(members)
}

#[cfg(test)]
pub mod fixture {
    //! A throwaway workspace on disk, for the lints' tests.

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::Workspace;

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
            self.members.push(path.to_owned());
            self
        }

        /// Writes the root manifest and reads the whole workspace back.
        pub fn load(&self) -> Workspace {
            let members: Vec<String> = self
                .members
                .iter()
                .map(|member| format!("  \"{member}\",\n"))
                .collect();
            let root = format!(
                "[workspace]\nresolver = \"3\"\nmembers = [\n{}]\n\n\
                 [workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n\
                 rust-version = \"1.96\"\nlicense = \"MIT OR Apache-2.0\"\n\n\
                 [workspace.dependencies]\n{}\n",
                members.concat(),
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
        assert_eq!(workspace_root(), repo_root().join("lablet"));
    }

    #[test]
    fn the_manifest_directory_cargo_names_at_run_time_wins_over_the_compiled_one() {
        // A binary built in one checkout and reused in another (a copied
        // checkout, a shared target directory) must gate the one it runs in.
        assert_eq!(
            root_from(Some("/elsewhere/checkout/xtask".into())),
            PathBuf::from("/elsewhere/checkout")
        );
        assert_eq!(
            root_from(None),
            Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
        );
    }

    fn load_with_members(
        tag: &str,
        members: &str,
        on_disk: &[&str],
    ) -> Result<Workspace, LoadError> {
        let dir = TempDir::new(tag);
        dir.write(
            "Cargo.toml",
            &format!("[workspace]\nmembers = [{members}]\n"),
        );
        for member in on_disk {
            dir.write(&format!("{member}/Cargo.toml"), "[package]\nname = \"x\"\n");
        }
        Workspace::load(dir.path())
    }

    #[test]
    fn a_wildcard_member_is_an_error_naming_the_entry() {
        for pattern in [
            "crates/domain/*",
            "crates/domain/mode?",
            "crates/domain/[mp]*",
            "crates/**",
        ] {
            let error =
                load_with_members("glob", &format!("\"{pattern}\""), &["crates/domain/model"])
                    .unwrap_err();
            assert!(
                error
                    .0
                    .contains(&format!("`{pattern}` is a wildcard pattern")),
                "{error}"
            );
        }
    }

    #[test]
    fn a_member_path_with_dot_segments_is_an_error() {
        // Each of these reaches crates/domain/x or crates/application/run on
        // disk while its leading text names another ring, or none.
        for path in [
            "apps/../crates/domain/x",
            "crates/adapters/secondary/shared/../../../application/run",
            "./crates/domain/x",
            "crates//domain/x",
        ] {
            let error = load_with_members(
                "dots",
                &format!("\"{path}\""),
                &["crates/domain/x", "crates/application/run", "apps/lablet"],
            )
            .unwrap_err();
            assert!(
                error.0.contains("is not a plain relative path"),
                "{path}: {error}"
            );
        }
    }

    #[test]
    fn an_absolute_member_path_is_an_error() {
        let dir = TempDir::new("absolute");
        dir.write("crates/domain/x/Cargo.toml", "[package]\nname = \"x\"\n");
        let absolute = dir.path().join("crates/domain/x");
        dir.write(
            "Cargo.toml",
            &format!("[workspace]\nmembers = [\"{}\"]\n", absolute.display()),
        );
        let error = Workspace::load(dir.path()).unwrap_err();
        assert!(error.0.contains("is not a plain relative path"), "{error}");
    }

    #[test]
    fn a_member_behind_a_symlink_is_an_error() {
        let dir = TempDir::new("symlink");
        dir.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/adapters/secondary/shared/run\"]\n",
        );
        dir.write(
            "crates/application/run/Cargo.toml",
            "[package]\nname = \"x\"\n",
        );
        let shared = dir.path().join("crates/adapters/secondary/shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::os::unix::fs::symlink("../../../application/run", shared.join("run")).unwrap();
        let error = Workspace::load(dir.path()).unwrap_err();
        assert!(error.0.contains("may not sit behind a symlink"), "{error}");
    }

    #[test]
    fn a_literal_member_stays_a_member_whatever_exclude_says() {
        // Cargo lets a literally listed member win over `exclude`.
        let dir = TempDir::new("exclude");
        dir.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/domain/model\", \"apps/lablet/\"]\nexclude = [\"crates/domain\"]\n",
        );
        for member in ["crates/domain/model", "apps/lablet"] {
            dir.write(&format!("{member}/Cargo.toml"), "[package]\nname = \"x\"\n");
        }
        let workspace = Workspace::load(dir.path()).unwrap();
        let paths: Vec<&str> = workspace.members.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, ["apps/lablet", "crates/domain/model"]);
    }

    #[test]
    fn a_root_manifest_with_a_package_table_is_a_member_at_the_empty_path() {
        let dir = TempDir::new("root-package");
        dir.write(
            "Cargo.toml",
            "[package]\nname = \"lablet-root\"\n\n[workspace]\nmembers = [\"a\"]\n",
        );
        dir.write("a/Cargo.toml", "[package]\nname = \"a\"\n");
        let workspace = Workspace::load(dir.path()).unwrap();
        let members: Vec<(&str, &str)> = workspace
            .members
            .iter()
            .map(|m| (m.path.as_str(), m.name.as_str()))
            .collect();
        assert_eq!(members, [("", "lablet-root"), ("a", "a")]);
    }

    #[test]
    fn a_path_dependency_resolves_to_the_member_whose_directory_it_names() {
        let workspace = load_with_members(
            "member-at",
            "\"crates/domain/model\", \"apps/lablet\"",
            &["crates/domain/model", "apps/lablet"],
        )
        .unwrap();
        let at = |from: &str, path: &str| workspace.member_at(from, path).map(|m| m.path.as_str());
        assert_eq!(
            at("apps/lablet", "../../crates/domain/model"),
            Some("crates/domain/model")
        );
        assert_eq!(at("", "crates/domain/model"), Some("crates/domain/model"));
        assert_eq!(
            at("", "./crates/domain/../domain/model/"),
            Some("crates/domain/model")
        );
        assert_eq!(at("apps/lablet", "../../crates/domain"), None);
        // A path that climbs out of the workspace root is no member's.
        assert_eq!(at("apps/lablet", "../../../outside/policy"), None);
        assert_eq!(at("", "/crates/domain/model"), None);
    }

    #[test]
    fn a_literal_member_without_a_manifest_is_an_error_naming_it() {
        let dir = TempDir::new("missing");
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"crates/gone\"]\n");
        let error = Workspace::load(dir.path()).unwrap_err();
        assert!(error.0.contains("crates/gone"), "{error}");
    }

    #[test]
    fn a_member_without_a_package_table_is_an_error() {
        let dir = TempDir::new("virtual");
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n");
        dir.write("a/Cargo.toml", "[dependencies]\n");
        let error = Workspace::load(dir.path()).unwrap_err();
        assert!(error.0.contains("no [package] table"), "{error}");
    }

    #[test]
    fn dependency_sections_cover_target_tables_and_name_their_headers() {
        let manifest = Manifest::parse(
            r#"
[package]
name = "x"

[dependencies]
a = "1"

[dev-dependencies]
b = "1"

[target.'cfg(unix)'.build-dependencies]
c = { version = "1" }
"#,
        )
        .unwrap();
        let headers: Vec<(String, DependencyKind)> = manifest
            .dependency_sections()
            .into_iter()
            .map(|section| (section.header, section.kind))
            .collect();
        assert_eq!(
            headers,
            [
                ("[dependencies]".to_owned(), DependencyKind::Normal),
                ("[dev-dependencies]".to_owned(), DependencyKind::Dev),
                (
                    "[target.'cfg(unix)'.build-dependencies]".to_owned(),
                    DependencyKind::Build
                ),
            ]
        );
    }

    #[test]
    fn a_dotted_workspace_key_reads_as_inherited() {
        let manifest =
            Manifest::parse("[package]\nname = \"x\"\n[dependencies]\nserde.workspace = true\n")
                .unwrap();
        let sections = manifest.dependency_sections();
        assert!(sections[0].entries["serde"].inherits_workspace());
    }

    #[test]
    fn members_are_found_by_name_in_either_spelling() {
        let dir = TempDir::new("names");
        dir.write("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n");
        dir.write(
            "a/Cargo.toml",
            "[package]\nname = \"lablet-telemetry-otel\"\n",
        );
        let workspace = Workspace::load(dir.path()).unwrap();
        assert!(workspace.member_named("lablet_telemetry_otel").is_some());
        assert!(workspace.member_named("lablet-telemetry").is_none());
    }
}
