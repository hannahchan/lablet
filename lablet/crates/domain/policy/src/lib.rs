//! Domain policy: pure stop and retry decisions, and pricing, over the types in
//! `lablet-model`.
//!
//! The loop owns the state and the clock; this crate owns the decisions. Every
//! comparison against a limit is "reached", never "exceeded": a run stops on
//! the turn, the instant, the token, or the error that meets its limit.

mod pricing;
mod retry;
mod stop;

pub use pricing::{Pricing, PricingError};
pub use retry::{RetryPolicy, RetryPolicyError};
pub use stop::{RunState, StopPoint, StopPolicy};
