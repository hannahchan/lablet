//! What a provider says a call cost in tokens.

use std::ops::{Add, AddAssign};

/// What a provider reports about one call, named for the counts themselves so
/// that five adjacent numbers can't be given in the wrong order.
///
/// `input` means what the constructor taking it says it means:
/// [`Usage::from_inclusive`] reads it as the whole prompt, and
/// [`Usage::from_uncached`] as the part of the prompt that touched no cache.
///
/// A count the provider didn't report is `None`, which isn't a count of zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TokenCounts {
    /// The prompt tokens, counted as the constructor says.
    pub input: u64,
    /// Every token the model generated, reasoning included.
    pub output: u64,
    /// The part of `output` the model spent on reasoning.
    pub reasoning: Option<u64>,
    /// Prompt tokens served from the provider's cache.
    pub cache_read: Option<u64>,
    /// Prompt tokens written to the provider's cache.
    pub cache_write: Option<u64>,
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
/// The reasoning count and the two cache counts are `None` when the provider
/// didn't report them, so a zero is a count of zero and never a gap: several
/// servers answer to one provider name and report different things, and a
/// consumer couldn't otherwise tell which zeros to believe. A sum is `None` in
/// a field only when nothing it sums reported the count, and whatever reads
/// the counts as numbers reads a missing one as nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
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
    /// Anthropic reports it: both cache counts are added in, a missing one as
    /// nothing.
    #[must_use]
    pub const fn from_uncached(counts: TokenCounts) -> Self {
        Self::from_inclusive(TokenCounts {
            input: counts
                .input
                .saturating_add(or_nothing(counts.cache_read))
                .saturating_add(or_nothing(counts.cache_write)),
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

    /// The input tokens that touched no cache: `input_tokens` less both cache
    /// fields, of which a missing one takes nothing away.
    #[must_use]
    pub const fn uncached_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_sub(or_nothing(self.cache_read_tokens))
            .saturating_sub(or_nothing(self.cache_write_tokens))
    }
}

impl Add for Usage {
    type Output = Self;

    /// Field by field, saturating, so a sum never panics. A count is missing
    /// from the sum only when it's missing from both sides.
    fn add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            reasoning_output_tokens: reported(
                self.reasoning_output_tokens,
                other.reasoning_output_tokens,
            ),
            cache_read_tokens: reported(self.cache_read_tokens, other.cache_read_tokens),
            cache_write_tokens: reported(self.cache_write_tokens, other.cache_write_tokens),
        }
    }
}

/// A count read as a number, a missing one as nothing.
const fn or_nothing(count: Option<u64>) -> u64 {
    match count {
        Some(count) => count,
        None => 0,
    }
}

/// The sum of what was reported of one count: a side that reported nothing
/// adds nothing, and the sum is missing only when both did.
fn reported(one: Option<u64>, other: Option<u64>) -> Option<u64> {
    one.zip(other)
        .map(|(one, other)| one.saturating_add(other))
        .or(one)
        .or(other)
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

#[cfg(test)]
mod tests;
