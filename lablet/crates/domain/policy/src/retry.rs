//! How long to wait before a failed provider call is tried again, and when to
//! give up on it.

use std::time::Duration;

/// Why a [`RetryPolicy`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum RetryPolicyError {
    /// `base` was longer than `max`.
    #[error("backoff base {base:?} is longer than the backoff cap {max:?}")]
    BaseAboveMax {
        /// The refused base delay.
        base: Duration,
        /// The cap it was held against.
        max: Duration,
    },
    /// `factor` was below one, infinite, or not a number.
    #[error("backoff factor {0} isn't a finite number of at least 1")]
    Factor(f64),
}

/// Exponential backoff for one provider call, with a budget of retries.
///
/// The budget belongs to a call, not to the run: the loop counts attempts from
/// one again for every provider call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    max_retries: u32,
    base: Duration,
    max: Duration,
    factor: f64,
}

impl RetryPolicy {
    /// A policy that tries a failed call again up to `max_retries` times, so
    /// `0` never retries, and waits `base` after the first failure, `factor`
    /// times longer after each further one, and never longer than `max`.
    ///
    /// # Errors
    ///
    /// Returns the [`RetryPolicyError`] for the first rule broken, in argument
    /// order: `base` is no longer than `max`, and `factor` is a finite number
    /// of at least 1.
    pub fn new(
        max_retries: u32,
        base: Duration,
        max: Duration,
        factor: f64,
    ) -> Result<Self, RetryPolicyError> {
        let policy = Self {
            max_retries,
            base,
            max,
            factor,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Apart from `new` because cargo-mutants never mutates a function of that
    /// name, and these comparisons are what the mutation floor should hold.
    fn validate(&self) -> Result<(), RetryPolicyError> {
        if self.base > self.max {
            return Err(RetryPolicyError::BaseAboveMax {
                base: self.base,
                max: self.max,
            });
        }
        if !self.factor.is_finite() || self.factor < 1.0 {
            return Err(RetryPolicyError::Factor(self.factor));
        }
        Ok(())
    }

    /// The wait before the next attempt, given that attempt number `attempt`
    /// of a call has just failed, or `None` when the call has had all its
    /// retries: attempt `n` is retried when `n` is at most `max_retries`.
    ///
    /// `attempt` is one-based, as `lablet.attempt` is: the first try of a call
    /// is attempt 1, and `0` is read as 1. The wait after attempt `n` is
    /// `base * factor^(n - 1)`, capped at `max`. There is no jitter, so the
    /// same attempt always gives the same wait and a test on a fake clock can
    /// assert it. No attempt number overflows or panics.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Option<Duration> {
        let attempt = attempt.max(1);
        if attempt > self.max_retries {
            return None;
        }
        let exponent = i32::try_from(attempt - 1).unwrap_or(i32::MAX);
        // Held to a finite number so that a zero base times an overflowed
        // scale is zero, not NaN.
        let scale = self.factor.powi(exponent).min(f64::MAX);
        let wait = Duration::try_from_secs_f64(self.base.as_secs_f64() * scale)
            .map_or(self.max, |wait| wait.min(self.max));
        Some(wait)
    }
}

#[cfg(test)]
mod tests;
