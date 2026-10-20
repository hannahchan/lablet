//! Secondary adapter: a `ModelProvider` over the Anthropic Messages API.
//!
//! One attempt is one `POST {base_url}/v1/messages`, sent without `stream`,
//! read as one JSON document and mapped to the domain. `thinking` and
//! `redacted_thinking` blocks are kept as the strings they came as and sent
//! back unchanged; a `tool_use` input, and a block of a kind the domain has
//! no name for, are `serde_json::Value`s, which serialise with sorted keys
//! and compact whitespace, so lablet's own requests repeat each other byte
//! for byte up to the turn each appends. With `model.cache` on,
//! `cache_control` breakpoints sit on the system prompt, the last tool spec
//! and the latest message; under `cache_scope: run` a system block holding
//! the run id comes ahead of the prompt, marker or none. The loop's
//! deadline is the request's timeout, enforced in real time.
//!
//! The adapter is built from three things and reads nothing else: a
//! [`Target`], which parses the base URL once, holds the key as a sensitive
//! header value and makes the HTTP client that follows no redirect and
//! resends nothing, so one attempt is one request to the host the base URL
//! names; the propagator its root resolved, through which the context
//! current as each attempt is built, the attempt's chat span's, goes into
//! the request headers; and whether the run captures content, which decides
//! whether a body that isn't the API's error envelope is quoted in a
//! failure's message.
//!
//! A failure is classified by status first and the envelope's `error.type`
//! and `error.message` second, into the five kinds the port names. Its
//! message is built from the provider's type, message and request id, or
//! from the kind of transport failure, and from nothing else: never the
//! URL, a header value or the text of a transport error, whose source chain
//! can name the host. The loop cuts the run's secrets out of it and bounds
//! it.
//!
//! The adapter emits no span, record or attribute. Its `tracing` lines are
//! diagnostics, and hold no content and no credential.

mod error;
mod propagate;
mod provider;
mod request;
mod response;
mod target;
mod wire;

pub use provider::AnthropicProvider;
pub use target::{Key, Target, TargetError};

/// The target the adapter's diagnostics are written under.
const DIAGNOSTICS: &str = "lablet::anthropic";
