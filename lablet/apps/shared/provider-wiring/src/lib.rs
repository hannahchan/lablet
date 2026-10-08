//! The model provider a checked config selects, built from what was read
//! for it, and readied by whichever root starts a run.

use std::sync::Arc;

use lablet_provider_fake::{FakeProvider, Script};
use lablet_run::ModelProvider;

/// The provider, as whoever starts a run knows it.
pub enum Played {
    /// A script, which every run hears from its first entry.
    Script(Arc<FakeProvider>),
}

impl Played {
    /// Readies the provider for a run that's about to start. The port
    /// names no run, so a provider can't tell where one begins.
    pub fn begin_run(&self) {
        match self {
            Self::Script(provider) => provider.rewind(),
        }
    }
}

/// The fake provider of the model `model`, which plays `script`: as the
/// loop calls it, and as whoever starts a run knows it.
#[must_use]
pub fn fake(model: String, script: Script) -> (Arc<dyn ModelProvider>, Played) {
    let provider = Arc::new(FakeProvider::new(model, script));
    (
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Played::Script(provider),
    )
}
