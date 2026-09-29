//! The output cap: what an executor keeps of a tool's text, and what the
//! model is sent of it.
//!
//! Text goes one way through these types. An executor feeds it to a
//! [`KeptOutput`], which keeps and counts and does nothing else. The cut
//! consumes that and returns content, and nothing here takes content. So a
//! cut can't be applied to what a cut produced, which would count the first
//! cut's line as the tool's own text and report the wrong size.

use crate::ToolResultContent;

/// The most of one tool call's text the model is sent, and how a longer
/// output is cut.
///
/// The fields are private because [`OutputCap::new`] refuses a preview longer
/// than the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputCap {
    max_bytes: u64,
    cut: OutputCut,
}

/// What's sent of an output longer than the cap. Each way adds one line that
/// says what was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputCut {
    /// The start, up to the cap.
    Head,
    /// Half the cap from each end, rounded down, around the line.
    HeadTail,
    /// A short preview of the start.
    Preview {
        /// How long the preview is.
        bytes: u64,
    },
}

/// Why a cap was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutputCapError {
    /// The preview is longer than the cap. An executor keeps the cap's worth
    /// of the start and no more, so the preview would ask for text nobody
    /// kept.
    #[error("a preview of {bytes} bytes is longer than the output cap of {max_bytes} bytes")]
    PreviewAboveCap {
        /// The preview that was asked for.
        bytes: u64,
        /// The cap it was asked for with.
        max_bytes: u64,
    },
}

/// What an executor keeps of a call's text: its first `head` bytes, and the
/// last `tail` bytes of what follows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputKeep {
    /// How much of the start is kept.
    pub head: u64,
    /// How much of the end is kept, of the text the start had no room for.
    pub tail: u64,
}

impl OutputKeep {
    const EVERYTHING: Self = Self {
        head: u64::MAX,
        tail: 0,
    };
}

impl OutputCap {
    /// What a run without a cap is measured against. No output is longer, so
    /// the only one it cuts is one an executor kept less of than it was fed,
    /// which has to say so however the run is configured.
    const NONE: Self = Self {
        max_bytes: u64::MAX,
        cut: OutputCut::Head,
    };

    /// A cap of `max_bytes`, which cuts a longer output as `cut` says.
    ///
    /// # Errors
    ///
    /// Returns [`OutputCapError::PreviewAboveCap`] when `cut` is a preview
    /// longer than `max_bytes`.
    pub const fn new(max_bytes: u64, cut: OutputCut) -> Result<Self, OutputCapError> {
        match cut {
            OutputCut::Preview { bytes } if bytes > max_bytes => {
                Err(OutputCapError::PreviewAboveCap { bytes, max_bytes })
            }
            OutputCut::Head | OutputCut::HeadTail | OutputCut::Preview { .. } => {
                Ok(Self { max_bytes, cut })
            }
        }
    }

    /// What an executor keeps so that this cap can be applied.
    ///
    /// The start it keeps is the cap's worth whatever the cut, a preview
    /// included: whether an output is over the cap isn't known until it ends,
    /// and one that isn't is sent whole. Only `HeadTail` sends an end, so
    /// only it has one kept.
    #[must_use]
    pub const fn keeps(self) -> OutputKeep {
        OutputKeep {
            head: self.max_bytes,
            tail: match self.cut {
                OutputCut::HeadTail => self.half(),
                OutputCut::Head | OutputCut::Preview { .. } => 0,
            },
        }
    }

    /// What's sent of an output that's cut: at most `head` bytes of its start
    /// and `tail` of its end.
    const fn sends(self) -> OutputKeep {
        match self.cut {
            OutputCut::Head => OutputKeep {
                head: self.max_bytes,
                tail: 0,
            },
            OutputCut::HeadTail => OutputKeep {
                head: self.half(),
                tail: self.half(),
            },
            OutputCut::Preview { bytes } => OutputKeep {
                head: bytes,
                tail: 0,
            },
        }
    }

    const fn half(self) -> u64 {
        self.max_bytes / 2
    }

    /// The line that says what was left out of an output of `total` bytes, of
    /// which `start` bytes were sent before the line and `end` after it.
    /// Worded here for the same reason as [`ToolResultContent::omitted`].
    fn line(self, start: u64, end: u64, total: u64) -> ToolResultContent {
        ToolResultContent::Text(match self.cut {
            OutputCut::Head => format!("[truncated: the first {start} of {total} bytes]"),
            OutputCut::HeadTail => {
                let left_out = total - start - end;
                format!("[truncated: {left_out} of {total} bytes left out]")
            }
            OutputCut::Preview { .. } => {
                format!("[output too large: the first {start} of {total} bytes]")
            }
        })
    }
}

/// What was kept of one call's text, and the size of all of it.
///
/// An executor makes one from the call's [`OutputKeep`] and feeds it the text
/// as it arrives. It keeps the first `head` bytes, counts everything after
/// them, and holds the last `tail` bytes of what it counted. So it holds no
/// more than the two together however much a tool writes, and the size it
/// reports is still exact.
///
/// The start keeps the items it was fed as items, none of them empty. The end
/// is one run of text, whatever items it came from. Neither begins or ends
/// inside a character.
///
/// A closing line is held apart from both, and beside them: it's short,
/// and it's counted as text that was fed and kept. It's what an executor
/// says of the output as a whole once all of it is there, such as how a
/// command ended, and the model is sent it last whatever the cut leaves
/// out, where a line that was fed last is lost to every cut that sends only
/// the start.
///
/// The fields are private, so the size can't disagree with what was fed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptOutput {
    keep: OutputKeep,
    content: Vec<ToolResultContent>,
    /// Whether the next text the start keeps begins an item.
    fresh: bool,
    tail: String,
    closing: String,
    /// The size of what was fed, which the closing line is no part of.
    fed_bytes: u64,
}

impl KeptOutput {
    /// The most that's kept of a closing line, in bytes. The line is sent on
    /// top of what the cap allows, so this is what bounds it.
    pub const CLOSING_MAX_BYTES: usize = 256;

    /// Nothing yet, of an output that `keep` is kept of. `None` keeps
    /// everything.
    #[must_use]
    pub fn new(keep: Option<OutputKeep>) -> Self {
        Self {
            keep: keep.unwrap_or(OutputKeep::EVERYTHING),
            content: Vec::new(),
            fresh: true,
            tail: String::new(),
            closing: String::new(),
            fed_bytes: 0,
        }
    }

    /// A result the loop wrote itself, which it holds whole: one item, or
    /// none for no text.
    #[must_use]
    pub fn whole(text: &str) -> Self {
        let mut whole = Self::new(None);
        whole.push(text);
        whole
    }

    /// Begins a new item: the text that follows isn't part of the item
    /// before it.
    pub const fn item(&mut self) {
        self.fresh = true;
    }

    /// Takes the next part of the current item.
    ///
    /// It's kept while the start has room. The first byte that isn't kept
    /// closes the start, so text that comes later is never kept in room a
    /// character left over, and what the start holds is always the output's
    /// own beginning with nothing missing.
    pub fn push(&mut self, text: &str) {
        let room = self.keep.head.saturating_sub(self.fed_bytes);
        let room = usize::try_from(room).unwrap_or(usize::MAX);
        let (kept, rest) = text.split_at(text.floor_char_boundary(room));
        self.fed_bytes = self.fed_bytes.saturating_add(text.len() as u64);
        if !kept.is_empty() {
            match self.content.last_mut() {
                Some(ToolResultContent::Text(item)) if !self.fresh => item.push_str(kept),
                _ => self.content.push(ToolResultContent::Text(kept.to_owned())),
            }
            self.fresh = false;
        }
        self.hold(rest);
    }

    /// Takes the next text that follows the start, and holds the last `tail`
    /// bytes of all it has taken.
    ///
    /// No more of `text` is copied than can be held, so one long write costs
    /// what a short one does.
    fn hold(&mut self, text: &str) {
        let max = usize::try_from(self.keep.tail).unwrap_or(usize::MAX);
        let over = (self.tail.len() + text.len()).saturating_sub(max);
        let gone = self.tail.ceil_char_boundary(over);
        let skipped = text.ceil_char_boundary(over.saturating_sub(self.tail.len()));
        self.tail.drain(..gone);
        self.tail.push_str(&text[skipped..]);
    }

    /// Says `line` of the output as a whole, in place of a line said before.
    ///
    /// At most [`Self::CLOSING_MAX_BYTES`] of it are kept, up to a character
    /// boundary, so an executor can't send the model text past the cap by
    /// saying it here.
    pub fn close(&mut self, line: &str) {
        let kept = line.floor_char_boundary(Self::CLOSING_MAX_BYTES);
        line[..kept].clone_into(&mut self.closing);
    }

    /// The size in bytes of everything that was fed, kept or not, and of the
    /// closing line.
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.fed_bytes.saturating_add(self.closing.len() as u64)
    }

    /// The size in bytes of the text that's held, the closing line's with
    /// it.
    #[must_use]
    pub fn kept_bytes(&self) -> u64 {
        ToolResultContent::bytes(&self.content)
            .saturating_add(self.tail.len() as u64)
            .saturating_add(self.closing.len() as u64)
    }

    /// What the model is sent, and the size of the whole output when that's
    /// less than all of it.
    ///
    /// An output no longer than the cap, its closing line counted, is sent
    /// whole: the line goes on from the text before it, as if it had been
    /// fed. A longer one is cut as the cap says, from what was kept, and so
    /// is one of any length that wasn't all kept: it can't be sent whole, so
    /// it says what's missing. The closing line follows whatever a cut
    /// sends, as an item of its own.
    pub(crate) fn cut(self, cap: Option<OutputCap>) -> (Vec<ToolResultContent>, Option<u64>) {
        let cap = cap.unwrap_or(OutputCap::NONE);
        let total_bytes = self.total_bytes();
        let gap = self.kept_bytes() < total_bytes;
        let Self {
            mut content,
            fresh,
            tail,
            closing,
            ..
        } = self;
        if !gap {
            // Nothing was left out, so the end goes on from where the start
            // stopped and the two are the whole output.
            let mut whole = Self {
                content,
                fresh: false,
                ..Self::new(None)
            };
            whole.push(&tail);
            if total_bytes <= cap.max_bytes {
                // The line begins an item where the text before it ended
                // one. The end is one run whatever items it was fed as, so
                // a line that follows it goes on from it.
                whole.fresh = fresh && tail.is_empty();
                whole.push(&closing);
                return (whole.content, None);
            }
            content = whole.content;
        }

        let mut sent = Self::new(Some(cap.sends()));
        for ToolResultContent::Text(text) in &content {
            sent.item();
            sent.push(text);
        }
        if gap {
            // The end can't reach back over text that wasn't kept, so none
            // of it comes from the start.
            sent.tail.clear();
            sent.hold(&tail);
        }

        let Self {
            mut content,
            tail: end,
            ..
        } = sent;
        let start = ToolResultContent::bytes(&content);
        // The closing line is sent, so it's no part of what was left out.
        let after = (end.len() + closing.len()) as u64;
        content.push(cap.line(start, after, total_bytes));
        content.extend(
            [end, closing]
                .into_iter()
                .filter(|text| !text.is_empty())
                .map(ToolResultContent::Text),
        );
        (content, Some(total_bytes))
    }
}

#[cfg(test)]
mod tests;
