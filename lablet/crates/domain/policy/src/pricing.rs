//! What a run's tokens cost.

use lablet_model::{Cost, Rates, Usage};

/// What a run's tokens cost, at the [`Rates`] it was configured with.
///
/// The rates are a model type, because the run reports them on its wide event
/// beside the cost; the arithmetic is here, because it's a policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pricing {
    rates: Rates,
}

impl Pricing {
    /// Prices usage at `rates`. [`Rates::new`] is where a rate is checked,
    /// so there's nothing left to refuse here.
    #[must_use]
    pub const fn new(rates: Rates) -> Self {
        Self { rates }
    }

    /// The rates a run reports beside its cost.
    #[must_use]
    pub const fn rates(&self) -> Rates {
        self.rates
    }

    /// The cost of `usage`, or `None` when the rates and counts multiply out
    /// past what an `f64` holds, which takes rates no real price list has.
    /// Such a cost would reach JSON as `null` anyway, so the run reports none.
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
    /// comes out low. Refusing to price it would be the worse failure for a
    /// tool that reports what happened: all four counts reach the wide event
    /// beside the cost, so a consumer can see the inconsistency.
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
