//! The transcript as lablet publishes it: the conversation, under a version.
//!
//! A [`Transcript`] is what a run holds; a [`TranscriptDocument`] is what a
//! run emits. They're separate because the version belongs to the document
//! and means nothing to the run: a transcript in memory has no version, and
//! whoever reads the file needs one to know what they're reading.
//!
//! It borrows rather than owns, so publishing a transcript copies nothing,
//! and it serialises without reading back. Reading one belongs to the side
//! that consumes it; lablet runs a loop and emits what it saw.

use serde::Serialize;

use super::{Transcript, Turn};

/// The version of the transcript document, raised when a reader that knows
/// the old form could misread the new one.
///
/// Dropping a field, renaming one, or changing what a field means all need a
/// raise. Adding one doesn't: a reader that doesn't know a key ignores it.
pub const TRANSCRIPT_SCHEMA_VERSION: u32 = 1;

/// One run's conversation, as the document lablet writes.
///
/// Built with [`TranscriptDocument::of`], which borrows the run's transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptDocument<'a> {
    /// Which form this document is in.
    pub schema_version: u32,
    /// The system prompt the run was given.
    pub system: &'a str,
    /// Every turn, in order.
    pub turns: &'a [Turn],
}

impl<'a> TranscriptDocument<'a> {
    /// The document for `transcript`, at the current version.
    #[must_use]
    pub const fn of(transcript: &'a Transcript) -> Self {
        // Destructured rather than read through accessors, so a field added
        // to `Transcript` fails to compile here until this says whether it's
        // published. That's why this module is a child of `transcript`: the
        // fields are private and only a descendant can see them.
        let Transcript { system, turns } = transcript;
        Self {
            schema_version: TRANSCRIPT_SCHEMA_VERSION,
            system: system.as_str(),
            turns: turns.as_slice(),
        }
    }
}

#[cfg(test)]
mod tests;
