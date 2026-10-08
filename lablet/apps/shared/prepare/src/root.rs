//! Whether the root of the built-in tools holds a file of lablet's own.
//!
//! The model reads and writes under the root, so a root that held the
//! config, a prompt, the transcript or the telemetry would let a run read
//! what it's measured with and write over its own record.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// A file of lablet's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnFile {
    /// The file the config was read from.
    Config,
    /// The file `prompt.system_file` names.
    SystemPrompt,
    /// The file a run's task prompt is read from, as `lablet run
    /// --prompt-file` names it: [`lablet_config::Config::with_prompt_file`].
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

impl OwnFile {
    /// Where lablet's reading or writing of the file at `path` lands.
    fn lands(self, path: &Path) -> PathBuf {
        match self {
            // The transcript is written in place of whatever its path names,
            // so a link there is replaced and what it leads to is left alone.
            Self::Transcript => replaced(path),
            // The others are read, or appended to, through a link.
            Self::Config | Self::SystemPrompt | Self::TaskPrompt | Self::Telemetry => {
                resolved(path)
            }
        }
    }
}

/// Where `path` leads once every symbolic link on the part of it that
/// exists is followed. The file itself needn't exist, and a transcript or
/// a telemetry file doesn't before its run: a name past the part that
/// exists is taken as it's written, since it can only be made as a
/// directory or a file and never as a link, and a `..` after it leads back
/// to the directory the name was to be made in.
pub(crate) fn resolved(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            // What's resolved so far has no link left on it, so its parent
            // is the directory `..` names.
            Component::ParentDir => {
                resolved.pop();
            }
            component => {
                resolved.push(component);
                // A name that exists may be a link.
                if let Ok(real) = std::fs::canonicalize(&resolved) {
                    resolved = real;
                }
            }
        }
    }
    resolved
}

/// Where a file that's put in place of whatever `path` names lands: under
/// its own name, in the directory the rest of `path` leads to.
fn replaced(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    match (absolute.parent(), absolute.file_name()) {
        (Some(directory), Some(name)) => resolved(directory).join(name),
        // A path that names no file has nothing put in its place.
        _ => resolved(&absolute),
    }
}

/// The first of `files` that `root` holds. `root` is resolved already.
pub fn held<'a>(
    root: &Path,
    files: impl IntoIterator<Item = (OwnFile, &'a Path)>,
) -> Option<(OwnFile, &'a Path)> {
    files
        .into_iter()
        .find(|(file, path)| file.lands(path).starts_with(root))
}

#[cfg(test)]
mod tests;
