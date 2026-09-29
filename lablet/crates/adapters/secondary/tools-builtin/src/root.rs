//! The directory the tools stay under, and where a path leads from it.

use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::SettingsError;

/// The root, with every symbolic link on the way to it followed, so that
/// what a path resolves to can be compared with it.
#[derive(Debug, Clone)]
pub(crate) struct Root(PathBuf);

/// Why a path wasn't followed, in words for the model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refused {
    /// The path resolves to somewhere that isn't under the root.
    #[error("it leads outside the run's root directory")]
    Outside,
    /// Nothing is there.
    #[error("no such file or directory")]
    Missing,
    /// A symbolic link is there and its target isn't. Writing to it would
    /// make the target, wherever the link says that is.
    #[error("it's a symbolic link to something that doesn't exist")]
    Dangling,
    /// The part of the path that doesn't exist yet holds something other
    /// than names, which nothing can resolve.
    #[error("it goes through a directory that doesn't exist")]
    Through,
    /// The system refused to resolve it.
    #[error("{0}")]
    Unresolved(String),
}

impl From<std::io::Error> for Refused {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            ErrorKind::NotFound => Self::Missing,
            _ => Self::Unresolved(error.to_string()),
        }
    }
}

impl Root {
    /// The directory `root` names.
    pub(crate) fn open(root: &Path) -> Result<Self, SettingsError> {
        let shown = || root.display().to_string();
        let resolved = std::fs::canonicalize(root).map_err(|error| SettingsError::Root {
            root: shown(),
            reason: error.to_string(),
        })?;
        if resolved.is_dir() {
            Ok(Self(resolved))
        } else {
            Err(SettingsError::RootIsNoDirectory { root: shown() })
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// Where `path` leads once every symbolic link is followed, which is
    /// something that exists under the root. A path that isn't absolute
    /// starts at the root.
    pub(crate) async fn existing(&self, path: &str) -> Result<PathBuf, Refused> {
        let resolved = tokio::fs::canonicalize(self.0.join(path)).await?;
        self.holding(resolved)
    }

    /// Where `path` leads once every symbolic link is followed, which is
    /// under the root and needn't exist yet: the part of the path that
    /// exists is resolved, and the names after it are taken as they're
    /// written.
    pub(crate) async fn reachable(&self, path: &str) -> Result<PathBuf, Refused> {
        let mut exists = self.0.join(path);
        let mut missing = Vec::new();
        let resolved = loop {
            match tokio::fs::canonicalize(&exists).await {
                Ok(resolved) => break resolved,
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            if tokio::fs::symlink_metadata(&exists).await.is_ok() {
                return Err(Refused::Dangling);
            }
            let Some(Component::Normal(name)) = exists.components().next_back() else {
                return Err(Refused::Through);
            };
            missing.push(name.to_owned());
            exists.pop();
        };
        let mut reached = self.holding(resolved)?;
        reached.extend(missing.into_iter().rev());
        Ok(reached)
    }

    fn holding(&self, resolved: PathBuf) -> Result<PathBuf, Refused> {
        if resolved.starts_with(&self.0) {
            Ok(resolved)
        } else {
            Err(Refused::Outside)
        }
    }
}
