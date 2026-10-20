//! The model provider a checked config selects, built from what was read
//! or made for it, and readied by whichever root starts a run. Adding an
//! adapter is a crate and a match arm of [`build`].

use std::sync::Arc;

use lablet_prepare::{BuildError, Ready, Unsupported};
use lablet_provider_anthropic::AnthropicProvider;
use lablet_provider_fake::{FakeProvider, Script};
use lablet_run::ModelProvider;
use opentelemetry::propagation::TextMapPropagator;

/// The provider, as whoever starts a run knows it.
pub enum Played {
    /// A script, which every run hears from its first entry.
    Script(Arc<FakeProvider>),
    /// A provider over the network, which has no run to begin.
    Live,
}

impl Played {
    /// Readies the provider for a run that's about to start. The port
    /// names no run, so a provider can't tell where one begins.
    pub fn begin_run(&self) {
        match self {
            Self::Script(provider) => provider.rewind(),
            Self::Live => {}
        }
    }
}

/// The provider `ready` selects, as the loop calls it and as whoever
/// starts a run knows it: the Anthropic adapter over its target, injecting
/// each attempt's context through `propagator`, the one its root resolved,
/// and quoting a body that isn't the API's envelope only when
/// `capture_content` is on; or the fake provider of the model `model`,
/// which plays its script. Nothing is made here that the check didn't
/// make: the target holds the HTTP client already.
///
/// # Errors
///
/// Returns [`BuildError::Unsupported`] for a provider there's no adapter
/// for yet, which `lablet_prepare::Prepared::wiring` refused already.
pub fn build(
    ready: Ready,
    model: String,
    propagator: Arc<dyn TextMapPropagator + Send + Sync>,
    capture_content: bool,
) -> Result<(Arc<dyn ModelProvider>, Played), BuildError> {
    match ready {
        Ready::Anthropic(target) => {
            let provider = Arc::new(AnthropicProvider::new(target, propagator, capture_content));
            Ok((provider, Played::Live))
        }
        Ready::Openai => Err(Unsupported::Openai.into()),
        Ready::Fake(script) => Ok(fake(model, script)),
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
