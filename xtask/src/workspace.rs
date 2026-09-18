//! Where the repository is, and the parsed view of a Cargo workspace that the
//! lints and the floor checks read. Everything here takes the workspace root
//! as an argument, so the lints run over a fixture in a test as they do over
//! `lablet/`.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The repository root. xtask lives at `<repo>/xtask`, and cargo records that
/// directory when it compiles this crate, so the answer does not depend on the
/// directory `cargo xtask` was started from. A moved checkout is a new package
/// path to cargo, which rebuilds xtask and records the new location.
pub fn repo_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map_or_else(|| manifest_dir.to_path_buf(), Path::to_path_buf)
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
    /// The table header as it is written, for diagnostics.
    pub header: String,
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
            "",
            [
                &self.dependencies,
                &self.dev_dependencies,
                &self.build_dependencies,
            ],
        );
        for (target, tables) in &self.target {
            push_sections(
                &mut sections,
                &format!("target.'{target}'."),
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
    prefix: &str,
    tables: [&'a Dependencies; 3],
) {
    let kinds = [
        (DependencyKind::Normal, "dependencies"),
        (DependencyKind::Dev, "dev-dependencies"),
        (DependencyKind::Build, "build-dependencies"),
    ];
    for ((kind, name), entries) in kinds.into_iter().zip(tables) {
        if !entries.is_empty() {
            sections.push(DependencySection {
                header: format!("[{prefix}{name}]"),
                kind,
                entries,
            });
        }
    }
}

/// The `[workspace]` table of the root manifest.
#[derive(Debug, Deserialize)]
pub struct WorkspaceTable {
    /// Member paths, which may hold `*` and `?` wildcards.
    pub members: Vec<String>,
    /// Paths taken back out of what `members` matched.
    #[serde(default)]
    pub exclude: Vec<String>,
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
    /// skipped such a member would pass it unread.
    pub fn load(root: &Path) -> Result<Self, LoadError> {
        let manifest_path = root.join("Cargo.toml");
        let text = read(&manifest_path)?;
        let document: toml::Value = parse(&manifest_path, &text)?;
        let table = parse::<RootManifest>(&manifest_path, &text)?.workspace;
        let mut members = Vec::new();
        for path in expand_members(root, &table.members, &table.exclude)? {
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

/// The member directories `patterns` select, sorted and without duplicates.
/// A literal path must hold a `Cargo.toml`; a wildcard pattern selects the
/// matching directories that hold one, as cargo does.
fn expand_members(
    root: &Path,
    patterns: &[String],
    exclude: &[String],
) -> Result<Vec<String>, LoadError> {
    let mut members = Vec::new();
    for pattern in patterns {
        let pattern = pattern.trim_end_matches('/');
        if pattern.contains(['*', '?']) {
            let segments: Vec<&str> = pattern.split('/').collect();
            expand_pattern(root, String::new(), &segments, &mut members);
        } else if root.join(pattern).join("Cargo.toml").is_file() {
            members.push(pattern.to_owned());
        } else {
            return Err(LoadError(format!(
                "workspace member `{pattern}` has no Cargo.toml under {}",
                root.display()
            )));
        }
    }
    members.retain(|member| {
        !exclude.iter().any(|excluded| {
            let excluded = excluded.trim_end_matches('/');
            member == excluded
                || member
                    .strip_prefix(excluded)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    });
    members.sort();
    members.dedup();
    Ok(members)
}

/// Walks one wildcard pattern segment by segment, collecting the directories
/// that match all of it and hold a `Cargo.toml`.
fn expand_pattern(root: &Path, prefix: String, segments: &[&str], found: &mut Vec<String>) {
    let Some((segment, rest)) = segments.split_first() else {
        if root.join(&prefix).join("Cargo.toml").is_file() {
            found.push(prefix);
        }
        return;
    };
    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        }
    };
    if !segment.contains(['*', '?']) {
        expand_pattern(root, join(segment), rest, found);
        return;
    }
    let Ok(entries) = std::fs::read_dir(root.join(&prefix)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && wildcard_matches(segment, &name) {
            expand_pattern(root, join(&name), rest, found);
        }
    }
}

/// Whether `name` matches `pattern`, where `*` is any run of characters and
/// `?` is any one.
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    // Iterative matching with one backtrack point: the last `*` seen.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((star_p, star_n)) = star {
            p = star_p + 1;
            n = star_n + 1;
            star = Some((star_p, star_n + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
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
    fn the_repository_root_holds_xtask_and_is_found_without_the_environment() {
        assert!(xtask_manifest().is_file());
        assert_eq!(workspace_root(), repo_root().join("lablet"));
    }

    #[test]
    fn a_wildcard_matches_runs_and_single_characters() {
        assert!(wildcard_matches("*", "model"));
        assert!(wildcard_matches("provider-*", "provider-fake"));
        assert!(wildcard_matches("*-fake", "provider-fake"));
        assert!(wildcard_matches("p*r-f?ke", "provider-fake"));
        assert!(!wildcard_matches("provider-*", "tools-mcp"));
        assert!(!wildcard_matches("?", "ab"));
        assert!(wildcard_matches("**", ""));
    }

    #[test]
    fn a_wildcard_member_selects_only_directories_holding_a_manifest() {
        let dir = TempDir::new("glob");
        dir.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/domain/*\", \"apps/lablet\"]\nexclude = [\"crates/domain/old\"]\n",
        );
        for member in ["crates/domain/model", "crates/domain/old", "apps/lablet"] {
            dir.write(&format!("{member}/Cargo.toml"), "[package]\nname = \"x\"\n");
        }
        dir.write("crates/domain/notes/README.md", "not a crate");
        let workspace = Workspace::load(dir.path()).unwrap();
        let paths: Vec<&str> = workspace.members.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, ["apps/lablet", "crates/domain/model"]);
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
