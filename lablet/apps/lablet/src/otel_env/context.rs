//! The resource's and the inbound context's share of the seam.

use super::Variables;

/// The resource's and the inbound context's variables, as the seam reads
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Context {}

impl Context {
    pub(super) fn read(_variables: &Variables<'_>) -> Self {
        Self {}
    }
}
