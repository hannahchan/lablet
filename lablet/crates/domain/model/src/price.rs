//! What a run's tokens cost, and the rates it was priced at.
//!
//! The arithmetic is a policy and lives in `lablet-policy`; the amounts live
//! here because a run reports them, on its wide event and in its summary.

/// An amount of money in US dollars: finite, and never negative.
///
/// JSON has no infinity or NaN, so an amount that overflowed would be written
/// as `null`, which is how a record says that no pricing was configured: the
/// two would be indistinguishable. A negative cost is no more meaningful.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Cost(f64);

/// Why an amount isn't a cost.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("{0} isn't a finite number of US dollars of at least 0")]
pub struct CostError(f64);

impl Cost {
    /// An amount in US dollars.
    ///
    /// # Errors
    ///
    /// Returns [`CostError`] unless `usd` is finite and at least 0.
    pub fn new(usd: f64) -> Result<Self, CostError> {
        if !usd.is_finite() || usd < 0.0 {
            return Err(CostError(usd));
        }
        Ok(Self(usd))
    }

    /// The amount in US dollars.
    #[must_use]
    pub const fn usd(self) -> f64 {
        self.0
    }
}

/// A model's prices in US dollars per million tokens, each finite and at
/// least 0.
///
/// The rates live in the model, though the arithmetic is a policy, because a
/// run reports them: the wide event carries them beside the cost, so a
/// consumer can recompute the number rather than trust it. A run whose
/// provider reported cache counts above its own input count is priced for its
/// cached tokens alone, and the rates are what let a consumer see that.
/// Reasoning tokens need no rate of their own: they're billed at the output
/// rate and are already part of `output_tokens`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    /// Per million input tokens that touched no cache.
    pub input: f64,
    /// Per million generated tokens, reasoning included.
    pub output: f64,
    /// Per million input tokens served from the prompt cache.
    pub cache_read: f64,
    /// Per million input tokens written to the prompt cache.
    pub cache_write: f64,
}

/// Why a number isn't a rate.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("{name} rate {value} isn't a finite number of at least 0")]
pub struct RateError {
    /// Which rate, as the config spells it.
    pub name: &'static str,
    /// The refused rate.
    pub value: f64,
}

impl Rates {
    /// Prices per million tokens.
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
        let rates = Self {
            input,
            output,
            cache_read,
            cache_write,
        };
        rates.checked()
    }

    /// Apart from `new` because cargo-mutants never mutates a function of that
    /// name, and these comparisons are what the mutation floor should hold.
    fn checked(self) -> Result<Self, RateError> {
        for (name, value) in [
            ("input", self.input),
            ("output", self.output),
            ("cache_read", self.cache_read),
            ("cache_write", self.cache_write),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(RateError { name, value });
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests;
