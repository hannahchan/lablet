//! The OTLP exporter's share of the seam.

use super::Variables;

/// The OTLP exporter's variables, as the seam reads them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Exporter {}

impl Exporter {
    pub(super) fn read(_variables: &Variables<'_>) -> Self {
        Self {}
    }
}
