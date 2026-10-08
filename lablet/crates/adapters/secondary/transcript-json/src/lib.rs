//! Secondary adapter: writes a run's transcript document to a JSON file,
//! as `lablet-run`'s [`TranscriptWriter`].

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lablet_documents::TranscriptDocument;
use lablet_model::RunId;
use lablet_run::{RunTranscript, TranscriptError, TranscriptWriter};

/// What a configured path holds where the run id belongs.
const RUN_ID: &[u8] = b"{run_id}";

/// The most bytes a run id may hold where it takes the place of
/// `{run_id}`: what lablet holds a run id to before its run, so a file's
/// name keeps room for what a path writes beside it.
const RUN_ID_MAX_BYTES: usize = 128;

/// How many temporary names this process has tried. With the process's id
/// it keeps two writes of this process apart. It can't keep this process
/// apart from another, which in another PID namespace can have the same
/// id, so a temporary file is made new and a name that's taken is passed
/// over.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// How many temporary names a write tries before it fails.
const TEMPORARY_NAMES: u32 = 100;

/// The mode a new file is made with when no file is there, less the umask:
/// what `File::create` gives.
const NEW_FILE_MODE: u32 = 0o666;

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
    /// The file couldn't be made or written whole, or a directory on the
    /// way to it couldn't be made.
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
    /// holds `{run_id}` and the run id isn't one component of a path, or
    /// is longer than 128 bytes. A caller names a run id, and the config
    /// names where transcripts go: an id that held a separator, or that
    /// named the directory above, would move the file out of there, and
    /// one too long would name no file.
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
    /// whatever the file held, and makes the directories the path is
    /// missing: a path that holds the run id as a directory names one that
    /// no run has made yet.
    ///
    /// The file holds the document whole or is left as it was. The document
    /// is written to a temporary file beside the path, which takes the
    /// path's place once it's whole and on the disk, so a symbolic link at
    /// the path is replaced and what it led to is left alone. It takes the
    /// permissions of the file the path held, through a link or not, so a
    /// transcript that was made private stays private.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptWriteError::Unwritable`] when a directory or the
    /// file can't be made, and when the document can't be written whole.
    /// The path then holds what it held, and nothing is left beside it.
    pub fn write(&self, document: &TranscriptDocument) -> Result<(), TranscriptWriteError> {
        self.replace(|file| render(document, BufWriter::new(file)))
            .map_err(|error| TranscriptWriteError::Unwritable {
                path: self.path.clone(),
                reason: error.to_string(),
            })
    }

    /// Puts what `fill` writes in place of whatever the file held, once
    /// `fill` has written all of it and it's on the disk.
    fn replace(&self, fill: impl FnOnce(&File) -> io::Result<()>) -> io::Result<()> {
        if self.path.file_name().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the path names no file",
            ));
        }
        if let Some(directory) = self.path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let held = std::fs::metadata(&self.path)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map(|held| held.permissions());
        // The file is made no more open than the one it replaces, so nobody
        // the old file kept out can open it while it's written, and its mode
        // is then set exactly, since the umask may have taken bits from it.
        // The sync comes before the rename, or a crash could leave the path
        // naming a file whose text never reached the disk, and it reports an
        // error that closing the file would drop.
        let (temporary, opened) = temporary_beside(
            &self.path,
            held.as_ref()
                .map_or(NEW_FILE_MODE, |held| held.mode() & 0o7777),
            || WRITES.fetch_add(1, Ordering::Relaxed),
        )?;
        let replaced = held
            .map_or(Ok(()), |held| opened.set_permissions(held))
            .and_then(|()| fill(&opened))
            .and_then(|()| opened.sync_all())
            .and_then(|()| std::fs::rename(&temporary, &self.path));
        if replaced.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        replaced
    }
}

/// The transcripts of a config that names a place for them, each written
/// to its run's [`TranscriptFile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonTranscripts {
    /// The path, with `${VAR}` substituted, that's written to.
    real: PathBuf,
    /// The path before substitution, which is the one a message shows, so
    /// that nothing a variable holds reaches the diagnostic log.
    written: PathBuf,
}

impl JsonTranscripts {
    /// The transcripts of runs at `real`, the path the config names with
    /// its variables substituted, which `written` is before substitution.
    #[must_use]
    pub fn new(real: PathBuf, written: PathBuf) -> Self {
        Self { real, written }
    }
}

#[async_trait::async_trait]
impl TranscriptWriter for JsonTranscripts {
    fn place(&self, run_id: &RunId) -> Result<PathBuf, TranscriptError> {
        TranscriptFile::for_run(&self.real, run_id)
            .map(|file| file.path)
            .map_err(|error| TranscriptError::NoPlace(error.to_string()))
    }

    async fn write(
        &self,
        place: PathBuf,
        transcript: RunTranscript,
    ) -> Result<(), TranscriptError> {
        let RunTranscript {
            context,
            model,
            tools,
            task,
            transcript,
        } = transcript;
        let document = TranscriptDocument::new(context, model, tools, task, transcript);
        let file = TranscriptFile { path: place };
        // A file is written where waiting is allowed, as the telemetry's
        // files are.
        tokio::task::spawn_blocking(move || file.write(&document))
            .await
            .map_err(|error| format!("the transcript's write didn't run to its end: {error}"))
            .and_then(|written| {
                written.map_err(|error| match error {
                    TranscriptWriteError::Unwritable { reason, .. } => format!(
                        "the transcript couldn't be written to {}: {reason}",
                        self.written.display()
                    ),
                    error @ TranscriptWriteError::RunIdNotOneComponent { .. } => error.to_string(),
                })
            })
            .map_err(TranscriptError::Unwritten)
    }
}

/// Makes a file of this write's own beside `path`, named for this process
/// and the first number `next` gives that names no file there, with `mode`
/// less the umask, and says where it is.
///
/// The name holds nothing of the path's, so a name the file system takes
/// at the path is one it takes here too. The file is made new: a name
/// that's taken, by a file or a link, is left as it is and the next is
/// tried, [`TEMPORARY_NAMES`] in all.
fn temporary_beside(
    path: &Path,
    mode: u32,
    mut next: impl FnMut() -> u64,
) -> io::Result<(PathBuf, File)> {
    for _ in 0..TEMPORARY_NAMES {
        let temporary =
            path.with_file_name(format!(".lablet-{}-{}.tmp", std::process::id(), next()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{TEMPORARY_NAMES} temporary names beside it were taken"),
    ))
}

/// The run id as the bytes of one path component, which is a name that
/// holds no separator and no NUL, isn't one of the two every directory
/// has, and is no longer than [`RUN_ID_MAX_BYTES`].
fn one_component(run_id: &RunId) -> Result<&[u8], TranscriptWriteError> {
    let reason = match run_id.as_str() {
        "." => "it names the directory itself",
        ".." => "it names the directory above",
        id if id.contains('/') => "it holds a `/`",
        id if id.contains('\0') => "it holds a NUL",
        id if id.len() > RUN_ID_MAX_BYTES => "it's longer than 128 bytes",
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
