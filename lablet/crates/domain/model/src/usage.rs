//! What a provider says a call cost in tokens.

use std::ops::{Add, AddAssign};

use serde::{Deserialize, Serialize};

/// What a provider reports about one call, named for the counts themselves so
/// that five adjacent numbers can't be given in the wrong order.
///
/// `input` means what the constructor taking it says it means:
/// [`Usage::from_inclusive`] reads it as the whole prompt, and
/// [`Usage::from_uncached`] as the part of the prompt that touched no cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TokenCounts {
    /// The prompt tokens, counted as the constructor says.
    pub input: u64,
    /// Every token the model generated, reasoning included.
    pub output: u64,
    /// The part of `output` the model spent on reasoning; zero for a provider
    /// that doesn't report it.
    pub reasoning: u64,
    /// Prompt tokens served from the provider's cache.
    pub cache_read: u64,
    /// Prompt tokens written to the provider's cache.
    pub cache_write: u64,
}

/// Token counts of one provider response, or the sum over several.
///
/// Two fields are subsets of others, both because the GenAI semantic
/// conventions count them that way. `input_tokens` is the whole prompt and
/// **includes** the cached tokens, so `cache_read_tokens` and
/// `cache_write_tokens` are parts of it and
/// [`Usage::uncached_input_tokens`] is the subtraction.
/// `reasoning_output_tokens` is the part of `output_tokens` the model spent
/// thinking, so it's billed at the output rate and adding it to a total would
/// count it twice.
///
/// An adapter builds a `Usage` through the constructor named for its
/// provider's convention, [`Usage::from_inclusive`] or
/// [`Usage::from_uncached`], so the cache addition can't be forgotten.
///
/// A field is zero for a provider that doesn't report it. A field left out
/// when deserialising is zero and a field with any other name is an error, so
/// a misspelt count isn't read as zero; all five are always serialised.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Usage {
    /// Every token of the prompt, cached or not.
    pub input_tokens: u64,
    /// Every token the model generated, reasoning included.
    pub output_tokens: u64,
    /// The part of `output_tokens` the model spent on reasoning.
    pub reasoning_output_tokens: u64,
    /// The part of `input_tokens` served from the provider's prompt cache.
    pub cache_read_tokens: u64,
    /// The part of `input_tokens` written to the provider's prompt cache.
    pub cache_write_tokens: u64,
}

impl Usage {
    /// From a provider whose input count already includes the cached tokens,
    /// as OpenAI-compatible servers report it.
    #[must_use]
    pub const fn from_inclusive(counts: TokenCounts) -> Self {
        Self {
            input_tokens: counts.input,
            output_tokens: counts.output,
            reasoning_output_tokens: counts.reasoning,
            cache_read_tokens: counts.cache_read,
            cache_write_tokens: counts.cache_write,
        }
    }

    /// From a provider whose input count leaves the cached tokens out, as
    /// Anthropic reports it: both cache counts are added in.
    #[must_use]
    pub const fn from_uncached(counts: TokenCounts) -> Self {
        Self::from_inclusive(TokenCounts {
            input: counts
                .input
                .saturating_add(counts.cache_read)
                .saturating_add(counts.cache_write),
            ..counts
        })
    }

    /// `input_tokens + output_tokens`, the number a token budget counts.
    /// Neither the cache fields nor the reasoning tokens are added, because
    /// the two totals already hold them.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// The input tokens that touched no cache: `input_tokens` less both cache fields.
    #[must_use]
    pub const fn uncached_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_sub(self.cache_read_tokens)
            .saturating_sub(self.cache_write_tokens)
    }
}

impl Add for Usage {
    type Output = Self;

    /// Field by field, saturating, so a sum never panics.
    fn add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            reasoning_output_tokens: self
                .reasoning_output_tokens
                .saturating_add(other.reasoning_output_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_add(other.cache_read_tokens),
            cache_write_tokens: self
                .cache_write_tokens
                .saturating_add(other.cache_write_tokens),
        }
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

#[cfg(test)]
mod tests;
