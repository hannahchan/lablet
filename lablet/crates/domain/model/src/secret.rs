//! lablet's own secrets, cut out of a tool's text on its way to the
//! [`KeptOutput`] that keeps it.
//!
//! Which values are secrets is the composition root's to know. This is only
//! the cut, which every executor makes the same way and with the same
//! words. It comes before anything is kept, so the size an output reports
//! and the cut the output cap makes are of the text the model is sent.

use crate::{KeptOutput, OutputKeep};

/// The values of lablet's own secrets, which no tool result shows.
///
/// A value is cut only as it's written, and only when it's at least
/// [`Secrets::MIN_BYTES`] long once the whitespace around it is gone, since
/// whitespace a variable was set with is no part of a key.
///
/// Its `Debug` form says how many values it holds and nothing of them.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secrets {
    values: Vec<String>,
}

impl core::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Secrets")
            .field("values", &self.values.len())
            .finish()
    }
}

impl Secrets {
    /// The shortest value that's cut, in bytes. A key is far longer. A
    /// shorter value could be ordinary text, and cutting it would mangle
    /// output that only holds the same letters, and show the model what the
    /// value is by where the cuts fall.
    pub const MIN_BYTES: usize = 16;

    /// What stands where a value was cut. It says that a secret was cut, and
    /// nothing of which one or how long it was.
    pub const MARKER: &'static str = "[secret withheld]";

    /// The values among `values` that are long enough to cut.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = String>) -> Self {
        let values = values
            .into_iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| value.len() >= Self::MIN_BYTES)
            .collect();
        Self { values }
    }

    /// The longest value that `text` begins with, which is the one cut where
    /// two begin at one place.
    fn begun(&self, text: &str) -> Option<&str> {
        self.values
            .iter()
            .map(String::as_str)
            .filter(|value| text.starts_with(value))
            .max_by_key(|value| value.len())
    }

    /// Whether `text` is the start of a value that goes on past it, so that
    /// what follows could make it one.
    fn may_begin(&self, text: &str) -> bool {
        self.values
            .iter()
            .any(|value| value.len() > text.len() && value.starts_with(text))
    }

    /// Hands `pass` `text` with every value cut out of it, and says where
    /// the part it didn't hand on begins and how much of that part is cut
    /// already.
    ///
    /// Copies that overlap are one cut. A value that begins inside one that
    /// was cut stretches the cut to its own end, so no byte of either is
    /// shown: a copy that began a few bytes into the first would otherwise
    /// show all of itself but those bytes. The first `covered` bytes of
    /// `text` belong to a cut an earlier text began, so they're dropped with
    /// no second marker, and a value that begins among them stretches it.
    ///
    /// Of a text that hasn't `ended`, the part from the first place that may
    /// begin a value is left, since only what follows can say whether it
    /// does. That part is shorter than the longest value.
    ///
    /// `pass` is a trait object, so this is compiled once whoever calls it:
    /// branch coverage judges each compiled copy on its own, and no one
    /// caller takes every branch.
    fn cut(
        &self,
        text: &str,
        covered: usize,
        ended: bool,
        pass: &mut dyn FnMut(&str),
    ) -> (usize, usize) {
        // Everything before `cut_to` is handed on or cut, and the cut, if
        // there's one, ends there.
        let mut cut_to = covered;
        for (at, _) in text.char_indices() {
            let rest = &text[at..];
            if !ended && self.may_begin(rest) {
                pass(&text[cut_to.min(at)..at]);
                return (at, cut_to.saturating_sub(at));
            }
            if let Some(value) = self.begun(rest) {
                if at >= cut_to {
                    pass(&text[cut_to..at]);
                    pass(Self::MARKER);
                }
                cut_to = cut_to.max(at + value.len());
            }
        }
        pass(&text[cut_to..]);
        (text.len(), 0)
    }
}

/// A call's text on its way to the [`KeptOutput`] that keeps it, with every
/// value of its [`Secrets`] cut out.
///
/// It takes the text in whatever pieces the text arrives in, and cuts a
/// value that one piece begins and a later one ends as it would the whole
/// text. The end of the text that may begin a value is held back until what
/// follows says whether it does, and nothing else is, so it holds less than
/// the longest value.
pub struct RedactedOutput {
    secrets: Secrets,
    kept: KeptOutput,
    /// The end of the text so far, from the first place that may begin a
    /// value.
    held: String,
    /// How much of `held`, from its start, a cut that was made already
    /// covers.
    covered: usize,
}

impl RedactedOutput {
    /// Nothing yet, of an output that `keep` is kept of and that `secrets`
    /// are cut from.
    #[must_use]
    pub fn new(secrets: Secrets, keep: Option<OutputKeep>) -> Self {
        Self {
            secrets,
            kept: KeptOutput::new(keep),
            held: String::new(),
            covered: 0,
        }
    }

    /// Takes the next part of the text.
    pub fn push(&mut self, text: &str) {
        let mut held = std::mem::take(&mut self.held);
        held.push_str(text);
        let kept = &mut self.kept;
        let (left, covered) = self
            .secrets
            .cut(&held, self.covered, false, &mut |text| kept.push(text));
        held.drain(..left);
        self.held = held;
        self.covered = covered;
    }

    /// Says `line` of the output as a whole, as
    /// [`KeptOutput::close`] does, once the text is all there. What was held
    /// back is cut as the end of the text, and `line` as a text of its own.
    pub fn close(&mut self, line: &str) {
        self.end();
        let mut closing = String::new();
        self.secrets
            .cut(line, 0, true, &mut |text| closing.push_str(text));
        self.kept.close(&closing);
    }

    /// What was kept of the text, and the size of all of it.
    #[must_use]
    pub fn kept(mut self) -> KeptOutput {
        self.end();
        self.kept
    }

    /// Hands on what was held back: the text has ended, so nothing that
    /// follows can make it a value.
    fn end(&mut self) {
        let held = std::mem::take(&mut self.held);
        let covered = std::mem::take(&mut self.covered);
        let kept = &mut self.kept;
        self.secrets
            .cut(&held, covered, true, &mut |text| kept.push(text));
    }
}

#[cfg(test)]
mod tests;
