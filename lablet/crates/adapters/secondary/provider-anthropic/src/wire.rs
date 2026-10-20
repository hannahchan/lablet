//! The Messages API's shapes, as serde sees them: the request body and the
//! two bodies a response can be, a message and an error envelope.
//!
//! The request borrows from the `ProviderRequest` it's built from; a
//! response's content blocks are read as values and mapped one by one, since
//! a block of a kind the domain has no name for is kept whole.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// `POST /v1/messages`. A field left `None` or empty isn't sent, so the
/// API's default applies.
#[derive(Debug, Serialize)]
pub(crate) struct Request<'a> {
    pub model: &'a str,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub system: Vec<SystemBlock<'a>>,
    pub messages: Vec<MessageParam<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingParam>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<OutputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

/// One block of `system`, which is always sent in its array form.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename = "text")]
pub(crate) struct SystemBlock<'a> {
    pub text: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// One tool offered, as the API takes it: no `strict`, no `defer_loading`.
#[derive(Debug, Serialize)]
pub(crate) struct Tool<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub input_schema: &'a serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// The `thinking` parameter.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ThinkingParam {
    Adaptive {
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<&'static str>,
    },
    Enabled {
        budget_tokens: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<&'static str>,
    },
    Disabled,
}

/// The `output_config` parameter, of which lablet sends the effort alone.
#[derive(Debug, Serialize)]
pub(crate) struct OutputConfig {
    pub effort: &'static str,
}

/// A cache breakpoint: the 5-minute entry, since an agent loop's turns
/// start less than 5 minutes apart and the hour's entry buys a doubled
/// write for nothing.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum CacheControl {
    Ephemeral,
}

/// One message of the conversation.
#[derive(Debug, Serialize)]
pub(crate) struct MessageParam<'a> {
    pub role: &'static str,
    pub content: Vec<Block<'a>>,
}

/// One content block of a message: one of the kinds lablet writes, or a
/// block the provider sent and lablet replays as it was stored.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum Block<'a> {
    Known(Known<'a>),
    Opaque(&'a serde_json::Value),
}

/// The blocks lablet writes. A field left out isn't sent: a `tool_result`
/// with no text has no `content`, and `is_error` is sent only when it's
/// true, since the API's default is false.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Known<'a> {
    Text {
        text: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Thinking {
        thinking: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<&'a str>,
    },
    RedactedThinking {
        data: &'a str,
    },
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: Cow<'a, serde_json::Value>,
    },
    ToolResult {
        tool_use_id: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Vec<Block<'a>>>,
        #[serde(skip_serializing_if = "is_false")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` hands the field by reference"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

impl<'a> Block<'a> {
    /// A text block with no marker.
    pub(crate) const fn text(text: &'a str) -> Self {
        Self::Known(Known::Text {
            text,
            cache_control: None,
        })
    }

    /// Puts a cache breakpoint on the block, when it's a kind that takes one.
    pub(crate) const fn mark(&mut self) {
        if let Self::Known(
            Known::Text { cache_control, .. } | Known::ToolResult { cache_control, .. },
        ) = self
        {
            *cache_control = Some(CacheControl::Ephemeral);
        }
    }
}

/// A message, as a 200 carries one. Fields the adapter doesn't read are
/// left to serde to skip.
#[derive(Debug, Deserialize)]
pub(crate) struct Message {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub content: Vec<serde_json::Value>,
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub stop_details: Option<StopDetails>,
    pub usage: Usage,
}

/// Why the model stopped, beyond the reason: read for its category alone.
#[derive(Debug, Deserialize)]
pub(crate) struct StopDetails {
    #[serde(default)]
    pub category: Option<String>,
}

/// What the call used. `input_tokens` is the uncached remainder, and a count
/// the body doesn't carry is `None`, which isn't a count of zero.
#[derive(Debug, Deserialize)]
pub(crate) struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens_details: Option<OutputTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OutputTokensDetails {
    #[serde(default)]
    pub thinking_tokens: Option<u64>,
}

/// The error envelope every failure the API itself answers carries.
#[derive(Debug, Deserialize)]
pub(crate) struct ErrorEnvelope {
    pub error: ErrorBody,
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorBody {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<ErrorDetails>,
}

/// What a 429 for a spend cap carries beside its message.
#[derive(Debug, Deserialize)]
pub(crate) struct ErrorDetails {
    #[serde(default)]
    pub error_code: Option<String>,
}
