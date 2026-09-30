//! Whether the root of the built-in tools holds a file of lablet's own.
//!
//! The model reads and writes under the root, so a root that held the
//! config, a prompt, the transcript or the telemetry would let a run read
//! what it's measured with and write over its own record.

use std::fmt;
use std::path::{Path, PathBuf};

/// A file of lablet's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnFile {
    /// The file the config was read from.
    Config,
    /// The file `prompt.system_file` names.
    SystemPrompt,
    /// The file a run's task prompt is read from, as `lablet run
    /// --prompt-file` names it: [`crate::Config::with_prompt_file`].
    TaskPrompt,
    /// The file `run.transcript_path` names.
    Transcript,
    /// The file `telemetry.file.path` names, or the one a run has in the
    /// working directory when it names none.
    Telemetry,
}

impl fmt::Display for OwnFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Config => "the config",
            Self::SystemPrompt => "the system prompt that `prompt.system_file` names",
            Self::TaskPrompt => "the task prompt's file",
            Self::Transcript => "the transcript that `run.transcript_path` names",
            Self::Telemetry => "the telemetry file that `telemetry.file.path` names",
        })
    }
}

/// Where `path` leads once every symbolic link on the part of it that
/// exists is followed. The file itself needn't exist, and a transcript or
/// a telemetry file doesn't before its run: the names after the part that
/// exists are taken as they're written.
pub(crate) fn resolved(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut exists = absolute.clone();
    let mut missing = Vec::new();
    loop {
        if let Ok(mut resolved) = std::fs::canonicalize(&exists) {
            resolved.extend(missing.iter().rev());
            return resolved;
        }
        let Some(name) = exists.file_name().map(ToOwned::to_owned) else {
            return absolute;
        };
        missing.push(name);
        exists.pop();
    }
}

/// The first of `files` that `root` holds. `root` is resolved already.
pub(crate) fn held<'a>(
    root: &Path,
    files: impl IntoIterator<Item = (OwnFile, &'a Path)>,
) -> Option<(OwnFile, &'a Path)> {
    files
        .into_iter()
        .find(|(_, file)| resolved(file).starts_with(root))
}

#[cfg(test)]
mod tests;
