//! What a composer groups runs by, as both documents write it.

use lablet_model::RunLabels;
use serde::{Deserialize, Serialize};

/// The task, the experiment and the trial a run request named.
///
/// Every label is always written, one the request didn't name as `null`.
/// Read, a label may be left out. A key with any other name is refused,
/// since a misspelt label would otherwise read as a run that has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Labels {
    task: Option<String>,
    experiment: Option<String>,
    trial: Option<String>,
}

impl From<RunLabels> for Labels {
    fn from(labels: RunLabels) -> Self {
        let RunLabels {
            task,
            experiment,
            trial,
        } = labels;
        Self {
            task,
            experiment,
            trial,
        }
    }
}

impl From<Labels> for RunLabels {
    fn from(labels: Labels) -> Self {
        let Labels {
            task,
            experiment,
            trial,
        } = labels;
        Self {
            task,
            experiment,
            trial,
        }
    }
}

#[cfg(test)]
mod tests;
