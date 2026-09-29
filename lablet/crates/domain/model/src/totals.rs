//! What a run's calls sum to: the totals of its provider call attempts and
//! of its tool calls, and each tool's share of those.
//!
//! Each is a value that adds, as [`crate::Usage`] does, and every count in
//! one is added the same way, saturating. So a run's totals are the sum of
//! what each of its calls adds, whatever order they're added in, and a total
//! that a summary gains is a field of one of these, which nothing builds
//! without naming it.

use std::ops::Add;

use crate::{ToolCallOutcome, ToolUse, TurnRecord};

/// The sum of `parts`, which is nothing when there are none.
pub(crate) fn sum<T>(parts: impl Iterator<Item = T>) -> T
where
    T: Default + Add<Output = T>,
{
    parts.fold(T::default(), Add::add)
}

/// How long some calls took, in whole milliseconds: all of them together,
/// and the slowest of them.
///
/// The slowest is never longer than all of them together, because a
/// `Latency` is that of one call or a sum of two, and the fields aren't
/// public.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Latency {
    total_ms: u64,
    max_ms: u64,
}

impl Latency {
    /// The latency of one call, which took `latency_ms`.
    #[must_use]
    pub const fn of(latency_ms: u64) -> Self {
        Self {
            total_ms: latency_ms,
            max_ms: latency_ms,
        }
    }

    /// The summed latency of the calls.
    #[must_use]
    pub const fn total_ms(self) -> u64 {
        self.total_ms
    }

    /// The latency of the slowest call.
    #[must_use]
    pub const fn max_ms(self) -> u64 {
        self.max_ms
    }
}

impl Add for Latency {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            total_ms: self.total_ms.saturating_add(other.total_ms),
            max_ms: self.max_ms.max(other.max_ms),
        }
    }
}

/// What a run's provider call attempts came to, the ones that failed
/// included.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ProviderTotals {
    /// How many attempts were made beyond the first of their call. A call
    /// that fails on its only attempt adds none.
    pub retries: u64,
    /// How long the attempts took.
    pub latency: Latency,
}

impl ProviderTotals {
    /// What the provider call behind a turn adds: the attempts it took
    /// beyond its first, and the latency of the one that answered. The
    /// latencies of the ones that failed aren't in a turn's record.
    pub(crate) fn of(record: &TurnRecord) -> Self {
        Self {
            retries: u64::from(record.attempts.saturating_sub(1)),
            latency: Latency::of(record.latency_ms),
        }
    }
}

impl Add for ProviderTotals {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            retries: self.retries.saturating_add(other.retries),
            latency: self.latency + other.latency,
        }
    }
}

/// What a run's tool calls came to. How many there were is the outcome's to
/// say ([`crate::RunOutcome::tool_calls`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ToolCallTotals {
    /// How many calls returned an error result.
    pub errors: u64,
    /// How many calls named a tool the run didn't offer. They're in the
    /// other totals and have no share of their own.
    pub unknown: u64,
    /// How many calls had their output cut by the output cap.
    pub truncated: u64,
    /// The summed latency of the calls, in whole milliseconds.
    pub latency_ms: u64,
    /// The summed size of the calls' input, in bytes.
    pub input_bytes: u64,
    /// The summed size of the calls' output as the model was sent it, in
    /// bytes.
    pub output_bytes: u64,
}

impl ToolCallTotals {
    /// What one call adds.
    pub(crate) fn of(call: &ToolUse, outcome: &ToolCallOutcome) -> Self {
        Self {
            errors: u64::from(outcome.status.is_error()),
            unknown: u64::from(!outcome.status.names_an_offered_tool()),
            truncated: u64::from(outcome.truncated_from_bytes.is_some()),
            latency_ms: outcome.latency_ms,
            input_bytes: call.input_bytes(),
            output_bytes: outcome.output_bytes(),
        }
    }
}

impl Add for ToolCallTotals {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            errors: self.errors.saturating_add(other.errors),
            unknown: self.unknown.saturating_add(other.unknown),
            truncated: self.truncated.saturating_add(other.truncated),
            latency_ms: self.latency_ms.saturating_add(other.latency_ms),
            input_bytes: self.input_bytes.saturating_add(other.input_bytes),
            output_bytes: self.output_bytes.saturating_add(other.output_bytes),
        }
    }
}

/// One tool's share of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ToolStats {
    /// How many times the tool was called.
    pub calls: u64,
    /// How many of those calls returned an error result.
    pub errors: u64,
    /// The summed latency of those calls, in whole milliseconds.
    pub latency_ms: u64,
}

impl ToolStats {
    /// What one call adds to the share of the tool it named.
    pub(crate) fn of(outcome: &ToolCallOutcome) -> Self {
        Self {
            calls: 1,
            errors: u64::from(outcome.status.is_error()),
            latency_ms: outcome.latency_ms,
        }
    }
}

impl Add for ToolStats {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            calls: self.calls.saturating_add(other.calls),
            errors: self.errors.saturating_add(other.errors),
            latency_ms: self.latency_ms.saturating_add(other.latency_ms),
        }
    }
}

#[cfg(test)]
mod tests;
