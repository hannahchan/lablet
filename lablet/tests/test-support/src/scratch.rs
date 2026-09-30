//! A directory of a test's own.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::must;

/// A directory of one test's own, removed with what it holds when it's
/// dropped. Its path is the one the system resolves it to, so a test can
/// compare it with a path the code under test resolved.
pub struct Scratch(PathBuf);

impl Scratch {
    /// A new directory under the system's temporary directory, named from
    /// `test`, the process, and a count of the directories the process has
    /// made, so no two tests share one, even under one name.
    ///
    /// # Panics
    ///
    /// When the directory can't be made.
    #[must_use]
    pub fn new(test: &str) -> Self {
        Self::under(&std::env::temp_dir(), test)
    }

    fn under(parent: &Path, test: &str) -> Self {
        static MADE: AtomicU32 = AtomicU32::new(0);
        let directory = parent.join(format!(
            "lablet-{test}-{}-{}",
            std::process::id(),
            MADE.fetch_add(1, Ordering::Relaxed)
        ));
        // What a killed process of the same id left there isn't this test's.
        let _ = std::fs::remove_dir_all(&directory);
        let making = format!("making {}", directory.display());
        must(std::fs::create_dir_all(&directory), &making);
        Self(must(directory.canonicalize(), &making))
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The path `relative` names in the directory, which may not exist.
    #[must_use]
    pub fn at(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    /// Writes `contents` to the file `relative` names, and the directories
    /// on the way to it.
    ///
    /// # Panics
    ///
    /// When the file can't be written.
    pub fn write(&self, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.at(relative);
        if let Some(parent) = path.parent() {
            let making = format!("making {}", parent.display());
            must(std::fs::create_dir_all(parent), &making);
        }
        let writing = format!("writing {}", path.display());
        must(std::fs::write(&path, contents), &writing);
        path
    }

    /// Makes the directory `relative` names, and the directories on the way
    /// to it.
    ///
    /// # Panics
    ///
    /// When the directory can't be made.
    pub fn create_dir(&self, relative: &str) {
        let path = self.at(relative);
        let making = format!("making {}", path.display());
        must(std::fs::create_dir_all(&path), &making);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A panic here would abort a test that's already failing, and lose
        // what it reports.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests;
