//! Secondary adapter: writes a run's transcript document to a JSON file.
//!
//! It implements no port. One format is nothing to generalise from, so the
//! composition root calls it directly, after the run and where effects
//! belong.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};

use lablet_documents::TranscriptDocument;
use lablet_model::RunId;

/// What a configured path holds where the run id belongs.
const RUN_ID: &[u8] = b"{run_id}";

/// The file one run's transcript is written to.
///
/// It's made before the run, because a run's record names where its
/// transcript goes before anything is written there, and the path is
/// private, so the path a record names is the path [`TranscriptFile::write`]
/// writes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptFile {
    path: PathBuf,
}

/// Why a run's transcript has no file, or wasn't written to it.
///
/// Neither is a reason to fail a run: a transcript is a record of one, and
/// the run measured what it measured.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranscriptWriteError {
    /// The run id can't take the place of `{run_id}`, because a path with it
    /// there would name another place than the one the config names.
    #[error(
        "the run id {run_id:?} can't take the place of `{{run_id}}` in the transcript's path: \
         {reason}"
    )]
    RunIdNotOneComponent {
        /// The refused run id.
        run_id: String,
        /// Which rule of a path component it breaks.
        reason: &'static str,
    },
    /// The file couldn't be made or written whole.
    #[error("the transcript couldn't be written to {}: {reason}", path.display())]
    Unwritable {
        /// The file that was to be written.
        path: PathBuf,
        /// What the operating system said.
        reason: String,
    },
}

impl TranscriptFile {
    /// The file of the run `run_id`, at the path the config names, with the
    /// run id wherever that path holds `{run_id}`.
    ///
    /// A path that holds none is every run's file, so a second run writes
    /// over the first, and the run id is never read.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptWriteError::RunIdNotOneComponent`] when the path
    /// holds `{run_id}` and the run id isn't one component of a path. A
    /// caller names a run id, and the config names where transcripts go:
    /// an id that held a separator, or that named the directory above,
    /// would move the file out of there.
    pub fn for_run(configured: &Path, run_id: &RunId) -> Result<Self, TranscriptWriteError> {
        let configured = configured.as_os_str().as_bytes();
        let mut path = Vec::with_capacity(configured.len());
        let mut rest = configured;
        while let Some(at) = rest
            .windows(RUN_ID.len())
            .position(|window| window == RUN_ID)
        {
            let component = one_component(run_id)?;
            path.extend_from_slice(&rest[..at]);
            path.extend_from_slice(component);
            rest = &rest[at + RUN_ID.len()..];
        }
        path.extend_from_slice(rest);
        Ok(Self {
            path: OsString::from_vec(path).into(),
        })
    }

    /// Where the transcript is written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `document` as compact JSON and a newline, in place of
    /// whatever the file held. The directory isn't made: a path into one
    /// that doesn't exist is a path that can't be written.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptWriteError::Unwritable`] when the file can't be
    /// made or written whole.
    pub fn write(&self, document: &TranscriptDocument) -> Result<(), TranscriptWriteError> {
        File::create(&self.path)
            .and_then(|file| render(document, BufWriter::new(file)))
            .map_err(|error| TranscriptWriteError::Unwritable {
                path: self.path.clone(),
                reason: error.to_string(),
            })
    }
}

/// The run id as the bytes of one path component, which is a name that
/// holds no separator and no NUL and isn't one of the two every directory
/// has.
fn one_component(run_id: &RunId) -> Result<&[u8], TranscriptWriteError> {
    let reason = match run_id.as_str() {
        "." => "it names the directory itself",
        ".." => "it names the directory above",
        id if id.contains('/') => "it holds a `/`",
        id if id.contains('\0') => "it holds a NUL",
        id => return Ok(id.as_bytes()),
    };
    Err(TranscriptWriteError::RunIdNotOneComponent {
        run_id: run_id.as_str().to_owned(),
        reason,
    })
}

/// Writes `document` to `to` and sees it through, so an error a buffer
/// would otherwise keep until it's dropped is one the caller gets.
fn render(document: &TranscriptDocument, mut to: impl Write) -> io::Result<()> {
    serde_json::to_writer(&mut to, document)?;
    to.write_all(b"\n")?;
    to.flush()
}

#[cfg(test)]
mod tests;
