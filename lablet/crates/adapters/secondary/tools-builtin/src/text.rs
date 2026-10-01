//! What a tool wrote, read as text while it arrives.

use std::sync::Arc;

use lablet_model::{KeptOutput, OutputKeep, RedactedOutput, Secrets};

/// What stands for bytes that are no character.
const NO_CHARACTER: &str = "\u{FFFD}";

/// The text of one call, fed as bytes in whatever pieces they arrive in.
///
/// Bytes that aren't UTF-8 are read as U+FFFD, as a lossy conversion of the
/// whole output would read them, and a character that one piece begins and
/// the next ends is read whole. Nothing is held but the bytes of such a
/// character, and the end that may begin a secret, so the output is never
/// here in full: what's read has the secrets cut out and goes to the
/// [`KeptOutput`], which keeps what the call's cut can use.
pub(crate) struct Text {
    kept: RedactedOutput,
    /// The bytes of a character the last piece began and didn't end.
    begun: Vec<u8>,
    /// Whether the text so far ends where a line would begin.
    at_a_line: bool,
}

impl Text {
    /// No text yet, of an output that `keep` is kept of and that `secrets`
    /// are cut from.
    pub(crate) fn new(keep: Option<OutputKeep>, secrets: &Arc<Secrets>) -> Self {
        Self {
            kept: RedactedOutput::new(Arc::clone(secrets), keep),
            begun: Vec::new(),
            at_a_line: true,
        }
    }

    /// Takes the next bytes.
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        if self.begun.is_empty() {
            self.read(bytes);
        } else {
            let mut joined = std::mem::take(&mut self.begun);
            joined.extend_from_slice(bytes);
            self.read(&joined);
        }
    }

    /// Says `line` of the text as a whole, which is all there: the text
    /// ends its last line, and `line` is its closing line, which the model is
    /// sent whatever the call's cut leaves out of the text.
    pub(crate) fn close(&mut self, line: &str) {
        self.end_the_character();
        if !self.at_a_line {
            self.push("\n");
        }
        self.kept.close(line);
    }

    /// What was kept of the text, and the size of all of it.
    pub(crate) fn kept(mut self) -> KeptOutput {
        self.end_the_character();
        self.kept.kept()
    }

    fn read(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.utf8_chunks().peekable();
        while let Some(chunk) = chunks.next() {
            self.push(chunk.valid());
            let invalid = chunk.invalid();
            if invalid.is_empty() {
                continue;
            }
            let may_go_on = chunks.peek().is_none()
                && std::str::from_utf8(invalid).is_err_and(|error| error.error_len().is_none());
            if may_go_on {
                self.begun = invalid.to_vec();
            } else {
                self.push(NO_CHARACTER);
            }
        }
    }

    /// A character that was begun and never ended is no character.
    fn end_the_character(&mut self) {
        if !std::mem::take(&mut self.begun).is_empty() {
            self.push(NO_CHARACTER);
        }
    }

    fn push(&mut self, text: &str) {
        if let Some(last) = text.as_bytes().last() {
            self.at_a_line = *last == b'\n';
        }
        self.kept.push(text);
    }
}

#[cfg(test)]
mod tests;
