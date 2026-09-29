//! Domain policy: pure stop and retry decisions, and pricing, over the types in
//! `lablet-model`.
//!
//! The loop owns the clock and the run; this crate owns the decisions. Every
//! comparison against a limit is "reached", never "exceeded": a run stops on
//! the turn, the instant, or the token that meets its limit.

mod pricing;
mod retry;
mod stop;

pub use pricing::Pricing;
pub use retry::{RetryPolicy, RetryPolicyError, RetrySettings};
pub use stop::StopPolicy;
