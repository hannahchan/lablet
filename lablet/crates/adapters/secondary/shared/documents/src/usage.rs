//! Token counts, as both documents write them and a script states them.

use lablet_model::{self as model, TokenCounts};
use serde::{Deserialize, Serialize};

/// The token counts of one provider call, or the sum over several.
///
/// `input_tokens` includes the cached tokens and `output_tokens` the
/// reasoning tokens, so the other three are parts of those two and are
/// never added to them.
///
/// All five are always written, a count the provider didn't report as
/// `null`, so a reader finds every key and never takes a gap for a zero.
/// Read, a count may be left out: the input and the output count are then
/// zero, and the other three weren't reported. A key with any other name is
/// refused, so a misspelt count isn't read as a missing one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Usage {
    /// Every token of the prompt, cached or not.
    pub input_tokens: u64,
    /// Every token the model generated, reasoning included.
    pub output_tokens: u64,
    /// The part of `output_tokens` the model spent on reasoning.
    pub reasoning_output_tokens: Option<u64>,
    /// The part of `input_tokens` served from the provider's prompt cache.
    pub cache_read_tokens: Option<u64>,
    /// The part of `input_tokens` written to the provider's prompt cache.
    pub cache_write_tokens: Option<u64>,
}

impl From<model::Usage> for Usage {
    fn from(usage: model::Usage) -> Self {
        let model::Usage {
            input_tokens,
            output_tokens,
            reasoning_output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        } = usage;
        Self {
            input_tokens,
            output_tokens,
            reasoning_output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        }
    }
}

impl From<Usage> for model::Usage {
    /// The document's input count includes the cached tokens, so it's read
    /// by the constructor of that convention.
    fn from(usage: Usage) -> Self {
        let Usage {
            input_tokens,
            output_tokens,
            reasoning_output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        } = usage;
        Self::from_inclusive(TokenCounts {
            input: input_tokens,
            output: output_tokens,
            reasoning: reasoning_output_tokens,
            cache_read: cache_read_tokens,
            cache_write: cache_write_tokens,
        })
    }
}

#[cfg(test)]
mod tests;
