//! What a composer groups runs by.

use serde::{Deserialize, Serialize};

/// The task, the experiment and the trial a run belongs to, as whoever asked
/// for the run named them. Each is `None` when the request named none.
///
/// Lablet gives them no meaning of its own: it echoes them in the outcome
/// document, so a composer can group the runs it started without keeping a
/// table of run ids.
///
/// Every label is always serialised, a missing one as `null`. A key with any
/// other name is an error when deserialising, since every label may be left
/// out and a misspelt one would otherwise read as a run that has none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunLabels {
    /// The task the run attempts.
    pub task: Option<String>,
    /// The experiment the run is part of.
    pub experiment: Option<String>,
    /// Which repetition of the task the run is.
    pub trial: Option<String>,
}

#[cfg(test)]
mod tests;
