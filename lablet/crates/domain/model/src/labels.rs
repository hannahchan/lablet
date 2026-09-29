//! What a composer groups runs by.

/// The task, the experiment and the trial a run belongs to, as whoever asked
/// for the run named them. Each is `None` when the request named none.
///
/// Lablet gives them no meaning of its own: it echoes them in the outcome, so
/// a composer can group the runs it started without keeping a table of run
/// ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
