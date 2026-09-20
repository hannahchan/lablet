//! What a run's tokens cost.

use lablet_model::{Cost, RateError, Rates, Usage};

/// What a run's tokens cost, at the [`Rates`] it was configured with.
///
/// The rates are a model type, because the run reports them on its wide event
/// beside the cost; the arithmetic is here, because it's a policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pricing {
    rates: Rates,
}

impl Pricing {
    /// Prices per million tokens: `input` for input that touched no cache,
    /// `output` for generated tokens, `cache_read` for input served from the
    /// prompt cache, and `cache_write` for input written to it.
    ///
    /// # Errors
    ///
    /// Returns [`RateError`] for the first rate, in argument order, that isn't
    /// a finite number of at least 0.
    pub fn new(
        input: f64,
        output: f64,
        cache_read: f64,
        cache_write: f64,
    ) -> Result<Self, RateError> {
        Ok(Self {
            rates: Rates::new(input, output, cache_read, cache_write)?,
        })
    }

    /// The rates a run reports beside its cost.
    #[must_use]
    pub const fn rates(&self) -> Rates {
        self.rates
    }

    /// The cost of `usage`, or `None` when the rates and counts multiply out
    /// past what an `f64` holds. Rates are finite and counts are exact, so
    /// that takes rates no real price list has; the run then reports no cost
    /// rather than one that would reach JSON as `null`.
    ///
    /// `Usage::input_tokens` includes the cached tokens, so the input rate
    /// applies to `Usage::uncached_input_tokens` only and each cache field is
    /// billed once, at its own rate. Pricing `input_tokens` whole and adding
    /// the cache fields would bill the cached tokens twice. Reasoning tokens
    /// need no rate: both providers bill them at the output rate, and
    /// `Usage::reasoning_output_tokens` is already part of `output_tokens`.
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
            per_million(usage.uncached_input_tokens(), self.rates.input)
                + per_million(usage.output_tokens, self.rates.output)
                + per_million(usage.cache_read_tokens, self.rates.cache_read)
                + per_million(usage.cache_write_tokens, self.rates.cache_write),
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
