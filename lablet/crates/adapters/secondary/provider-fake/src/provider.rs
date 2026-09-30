//! The provider that plays a script.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use humantime::format_duration;
use lablet_model::{Endpoint, ModelRef, ProviderApi, ProviderErrorKind, ProviderResponse};
use lablet_run::{ModelProvider, ProviderError, ProviderRequest};

use crate::script::{Entry, Script};

/// A model provider that answers from a [`Script`], one entry for each
/// attempt of a provider call, in the order the script holds them.
///
/// It reads nothing of a request but its deadline: a script says what the
/// model answers, whatever it was asked.
///
/// It keeps its place from one call to the next and knows nothing of runs,
/// which the port tells a provider nothing of. [`FakeProvider::rewind`] is
/// for whoever starts a run, so that each run hears the script from its
/// first entry.
#[derive(Debug)]
pub struct FakeProvider {
    model: ModelRef,
    script: Script,
    /// How many attempts have been made since the script was last at its
    /// start, the ones made after it ran out among them.
    attempts: AtomicUsize,
}

impl FakeProvider {
    /// A provider that plays `script` under the model name `model`, which
    /// is the name the config gave and the one a run's record reports.
    #[must_use]
    pub fn new(model: impl Into<String>, script: Script) -> Self {
        Self {
            model: ModelRef {
                api: ProviderApi::Script,
                name: model.into(),
                replays_reasoning: false,
            },
            script,
            attempts: AtomicUsize::new(0),
        }
    }

    /// Puts the script back at its first entry, which the next attempt is
    /// then answered with.
    pub fn rewind(&self) {
        self.attempts.store(0, Ordering::Relaxed);
    }

    /// The failure of an attempt the script has no entry for.
    ///
    /// It's [`ProviderErrorKind::Fatal`], so the run ends there with
    /// `provider_error` and these words as its error. Asking again
    /// changes nothing, which is what that kind says: a kind the loop
    /// retries would have it wait out every backoff, in real time, and
    /// then report `retries_exhausted`, as if a provider had been
    /// unreliable and not a script short. The two kinds left name causes
    /// this isn't, a rejected key and a full context window.
    fn ran_out(&self) -> ProviderError {
        ProviderError::new(
            ProviderErrorKind::Fatal,
            format!(
                "script {name:?} ran out: entry {last} was its last, and the run made another \
                 attempt of a provider call. Add an entry to answer it, or have the last \
                 response end the run.",
                name = self.script.name(),
                last = self.script.entries(),
            ),
        )
    }

    /// The failure of an attempt that was given `deadline`, when `entry`,
    /// the script's entry `number`, takes that long or longer.
    fn reached(&self, deadline: Duration, number: usize, entry: &Entry) -> ProviderError {
        ProviderError::new(
            ProviderErrorKind::Retryable,
            format!(
                "entry {number} of script {name:?} answers after {latency}, and the attempt's \
                 deadline was {deadline}",
                name = self.script.name(),
                latency = format_duration(entry.latency),
                deadline = format_duration(deadline),
            ),
        )
    }
}

#[async_trait::async_trait]
impl ModelProvider for FakeProvider {
    fn model(&self) -> &ModelRef {
        &self.model
    }

    fn endpoint(&self) -> Option<Endpoint> {
        None
    }

    /// Plays the script's next entry: waits for its latency, in real time,
    /// and answers with its response or its failure.
    ///
    /// An entry is played once, whatever came of the attempt, so the
    /// attempt after a failure is answered by the entry after it. An
    /// attempt that's dropped stops its wait there, and has played its
    /// entry all the same.
    ///
    /// # Errors
    ///
    /// Returns the failure the entry injects.
    ///
    /// Returns a failure of kind [`ProviderErrorKind::Retryable`] when the
    /// entry's latency is no shorter than `request.deadline`, once the
    /// deadline has passed. A limit is met when it's reached, so a deadline
    /// of zero is reached by every entry, at once. What the entry held is
    /// never answered, its usage included: the attempt heard nothing.
    ///
    /// Returns a failure of kind [`ProviderErrorKind::Fatal`] when the
    /// script has no entry left, at once and whatever the deadline.
    async fn complete(
        &self,
        request: ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ProviderError> {
        let played = self.attempts.fetch_add(1, Ordering::Relaxed);
        let Some(entry) = self.script.entry(played) else {
            return Err(self.ran_out());
        };
        let wait = entry.latency.min(request.deadline);
        // A script that waits for nothing needs no timer, and so no
        // runtime that has one.
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        if entry.latency >= request.deadline {
            return Err(self.reached(request.deadline, played + 1, entry));
        }
        entry.answer.clone()
    }
}

#[cfg(test)]
mod tests;
