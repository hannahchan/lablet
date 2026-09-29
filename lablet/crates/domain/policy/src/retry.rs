//! How long to wait before a failed provider call is tried again, and when to
//! give up on it.

use std::time::Duration;

use lablet_model::ProviderErrorKind;

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
    /// `jitter` was below zero, above one, or not a number.
    #[error("retry jitter {0} isn't a number from 0 to 1")]
    Jitter(f64),
}

/// What a [`RetryPolicy`] is built from. The fields are named because three
/// durations passed in a row swap silently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetrySettings {
    /// How many times a failed call is tried again; `0` never retries.
    pub max_retries: u32,
    /// The backoff after the first attempt.
    pub base: Duration,
    /// The longest backoff.
    pub max: Duration,
    /// What the backoff grows by after each attempt.
    pub factor: f64,
    /// The longest wait a server may ask for. A longer one ends the retries.
    pub hint_max: Duration,
    /// The largest share of a wait that's added to it, from 0 to 1; `0` adds
    /// nothing.
    pub jitter: f64,
}

/// Exponential backoff for one provider call, with a budget of retries.
///
/// The budget belongs to a call, not to the run: the loop counts attempts from
/// one again for every provider call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    settings: RetrySettings,
}

impl RetryPolicy {
    /// A policy that tries a failed call again up to `settings.max_retries`
    /// times and waits as [`RetryPolicy::next`] describes.
    ///
    /// # Errors
    ///
    /// Returns the [`RetryPolicyError`] for the first rule broken, in the
    /// order of the fields: `base` is no longer than `max`, `factor` is a
    /// finite number of at least 1, and `jitter` is a number from 0 to 1.
    pub fn new(settings: RetrySettings) -> Result<Self, RetryPolicyError> {
        validate(&settings)?;
        Ok(Self { settings })
    }

    /// The wait before the next attempt, given that attempt number `attempt`
    /// of a call has just failed with `kind`, or `None` to stop trying.
    ///
    /// The whole rule is here rather than split with the loop. A call is
    /// tried again when the failure is one another attempt could answer
    /// differently ([`ProviderErrorKind::is_retryable`]), the call has a
    /// retry left, **and** the server didn't ask for a wait longer than
    /// `hint_max`. So `None` means `retries_exhausted` for a retryable
    /// failure and the failure's own stop reason for the rest. A policy that
    /// only measured the wait would leave the loop deciding half of it.
    ///
    /// `attempt` is one-based, as `lablet.attempt` is: the first try of a call
    /// is attempt 1, and `0` is read as 1. The backoff after attempt `n` is
    /// `base * factor^(n - 1)`, capped at `max`. `hint` is the wait the server
    /// asked for, and the wait is the longer of it and the backoff.
    ///
    /// The wait is then raised by a share of itself that's at most `jitter`,
    /// whichever of the two it came from: runs that were throttled together
    /// would otherwise come back together. The share is read from `salt` and
    /// from nothing else, so the policy draws no random number and the same
    /// salt always gives the same wait. The cap on the backoff and the cap on
    /// a hint are held before the share is added, so a wait can pass either
    /// by that share.
    ///
    /// No attempt number, factor or duration overflows or panics.
    #[must_use]
    pub fn next(
        &self,
        attempt: u32,
        kind: ProviderErrorKind,
        hint: Option<Duration>,
        salt: u64,
    ) -> Option<Duration> {
        let RetrySettings {
            max_retries,
            base,
            max,
            factor,
            hint_max,
            jitter,
        } = self.settings;
        if !kind.is_retryable() {
            return None;
        }
        let attempt = attempt.max(1);
        if attempt > max_retries {
            return None;
        }
        if hint.is_some_and(|hint| hint > hint_max) {
            return None;
        }
        let exponent = i32::try_from(attempt - 1).unwrap_or(i32::MAX);
        // Held to a finite number so that a zero base times an overflowed
        // scale is zero, not NaN.
        let scale = factor.powi(exponent).min(f64::MAX);
        let backoff = Duration::try_from_secs_f64(base.as_secs_f64() * scale)
            .map_or(max, |backoff| backoff.min(max));
        let wait = hint.map_or(backoff, |hint| hint.max(backoff));
        // The share is below one, so what's added is shorter than the wait
        // and is always a duration.
        let added = Duration::try_from_secs_f64(wait.as_secs_f64() * jitter * share(salt))
            .unwrap_or(Duration::MAX);
        Some(wait.saturating_add(added))
    }
}

/// Apart from `new` because cargo-mutants never mutates a function of that
/// name, and these comparisons are what the mutation floor should hold.
fn validate(settings: &RetrySettings) -> Result<(), RetryPolicyError> {
    let RetrySettings {
        base,
        max,
        factor,
        jitter,
        ..
    } = *settings;
    if base > max {
        return Err(RetryPolicyError::BaseAboveMax { base, max });
    }
    if !factor.is_finite() || factor < 1.0 {
        return Err(RetryPolicyError::Factor(factor));
    }
    if !(0.0..=1.0).contains(&jitter) {
        return Err(RetryPolicyError::Jitter(jitter));
    }
    Ok(())
}

/// How much of the jitter a salt takes: from 0 up to, and never reaching, 1.
///
/// It reads the upper half of the salt. A multiplicative hash such as the one
/// behind [`lablet_model::RunId::salt`] carries a change in its input upward,
/// so the upper bits are the ones every byte of the input reaches.
fn share(salt: u64) -> f64 {
    let [a, b, c, d, ..] = salt.to_be_bytes();
    f64::from(u32::from_be_bytes([a, b, c, d])) / (f64::from(u32::MAX) + 1.0)
}

#[cfg(test)]
mod tests;
