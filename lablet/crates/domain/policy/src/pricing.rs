//! What a run's tokens cost.

use lablet_model::{Cost, Usage};

/// Why a [`Pricing`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum PricingError {
    /// A rate was negative, infinite, or not a number.
    #[error("{name} rate {value} isn't a finite number of at least 0")]
    Rate {
        /// Which rate, as the config spells it.
        name: &'static str,
        /// The refused rate.
        value: f64,
    },
}

/// A model's prices in US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pricing {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

impl Pricing {
    /// Prices per million tokens: `input` for input that touched no cache,
    /// `output` for generated tokens, `cache_read` for input served from the
    /// prompt cache, and `cache_write` for input written to it.
    ///
    /// # Errors
    ///
    /// Returns [`PricingError::Rate`] for the first rate, in argument order,
    /// that isn't a finite number of at least 0.
    pub fn new(
        input: f64,
        output: f64,
        cache_read: f64,
        cache_write: f64,
    ) -> Result<Self, PricingError> {
        let pricing = Self {
            input,
            output,
            cache_read,
            cache_write,
        };
        pricing.validate()?;
        Ok(pricing)
    }

    /// Apart from `new` because cargo-mutants never mutates a function of that
    /// name, and these comparisons are what the mutation floor should hold.
    fn validate(&self) -> Result<(), PricingError> {
        for (name, value) in [
            ("input", self.input),
            ("output", self.output),
            ("cache_read", self.cache_read),
            ("cache_write", self.cache_write),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PricingError::Rate { name, value });
            }
        }
        Ok(())
    }

    /// The cost of `usage`, or `None` when the rates and counts multiply out
    /// past what an `f64` holds. Rates are finite and counts are exact, so
    /// that takes rates no real price list has; the run then reports no cost
    /// rather than one that would reach JSON as `null`.
    ///
    /// `Usage::input_tokens` includes the cached tokens, so the input rate
    /// applies to `Usage::uncached_input_tokens` only and each cache field is
    /// billed once, at its own rate. Pricing `input_tokens` whole and adding
    /// the cache fields would bill the cached tokens twice.
    ///
    /// A provider that reports cache counts above its own input count leaves
    /// no uncached part, so that run is priced for its cached tokens alone and
    /// the cost comes out low. The alternative is refusing to price a run
    /// because a provider's arithmetic didn't agree with itself, which is the
    /// worse failure for a tool whose job is to report what happened: all four
    /// counts reach the wide event beside the cost, so a consumer can see the
    /// inconsistency and discount the number.
    #[must_use]
    pub fn cost(&self, usage: &Usage) -> Option<Cost> {
        Cost::new(
            per_million(usage.uncached_input_tokens(), self.input)
                + per_million(usage.output_tokens, self.output)
                + per_million(usage.cache_read_tokens, self.cache_read)
                + per_million(usage.cache_write_tokens, self.cache_write),
        )
        .ok()
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a token count is exact in an f64 up to 2^53, and past that the rounding is one part in 2^53"
)]
fn per_million(tokens: u64, rate: f64) -> f64 {
    tokens as f64 * rate / 1_000_000.0
}

#[cfg(test)]
mod tests;
