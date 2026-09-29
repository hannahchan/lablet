//! Adapter shared kernel: the documents lablet writes, and the shapes they
//! and a fake-provider script are made of.
//!
//! The domain holds values in memory and publishes no shape. Each shape here
//! is a published one: its field names, its nesting and its spellings are
//! what a reader outside lablet parses. A shape is built from the domain by
//! a mapping that takes its source apart by pattern, so a field or a variant
//! the domain gains doesn't compile here until the mapping says whether
//! it's published.
//!
//! [`TranscriptDocument`] is written and never read: lablet emits what it
//! saw, and reading a transcript belongs to whoever consumes it.
//! [`OutcomeDocument`] is read too, and a read refuses a key it doesn't
//! know, a version it doesn't know, and an outcome no run could have had.
//! [`ContentBlock`], [`ToolUse`], [`ToolInput`], [`ProviderKind`] and
//! [`Usage`] go both ways, because a script states what a transcript
//! records: a person writes a script, so a key they misspelt is an error
//! and never a default.

mod content;
mod id;
mod labels;
mod outcome;
mod tool;
mod transcript;
mod usage;

pub use content::{ContentBlock, ProviderKind, ToolInput, ToolUse};
pub use outcome::{OUTCOME_SCHEMA_VERSION, OutcomeDocument, OutcomeDocumentError};
pub use transcript::{TRANSCRIPT_SCHEMA_VERSION, TranscriptDocument};
pub use usage::Usage;
