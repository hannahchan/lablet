//! From a 200's body to a `ProviderResponse`: the blocks, the usage and the
//! finish reason.

use lablet_model::{
    ContentBlock, FinishReason, ProviderErrorKind, ProviderKind, ProviderResponse, ResponseError,
    TokenCounts, ToolCallId, ToolInput, ToolName, ToolUse, Usage,
};
use lablet_run::ProviderError;

use crate::DIAGNOSTICS;
use crate::wire::Message;

/// The response `body` carries.
///
/// # Errors
///
/// A malformed failure for a body that isn't a message, one with no
/// `stop_reason`, a `thinking` block with no signature, a `tool_use` whose
/// id or name the domain refuses, and two `tool_use` blocks with one id.
pub(crate) fn parse(body: &[u8]) -> Result<ProviderResponse, ProviderError> {
    let message: Message = serde_json::from_slice(body).map_err(|error| {
        malformed(format!(
            "the response couldn't be read as a message: {}",
            without_quoted(&error.to_string())
        ))
    })?;
    let stop_reason = message
        .stop_reason
        .ok_or_else(|| malformed("the response has no stop_reason"))?;
    let content = message
        .content
        .iter()
        .map(block)
        .collect::<Result<Vec<_>, _>>()?;
    if stop_reason == "refusal"
        && let Some(category) = message.stop_details.and_then(|details| details.category)
    {
        tracing::info!(
            target: DIAGNOSTICS,
            "the model declined to answer (category: {})",
            category_label(&category)
        );
    }
    let usage = Usage::from_uncached(TokenCounts {
        input: message.usage.input_tokens,
        output: message.usage.output_tokens,
        reasoning: message
            .usage
            .output_tokens_details
            .and_then(|details| details.thinking_tokens),
        cache_read: message.usage.cache_read_input_tokens,
        cache_write: message.usage.cache_creation_input_tokens,
    });
    // The domain's reason quotes the repeated id, the server's text of any
    // length, so the message says what's wrong instead.
    ProviderResponse::new(
        content,
        usage,
        finish(stop_reason),
        message.id,
        message.model,
    )
    .map_err(|ResponseError::DuplicateToolUse { .. }| {
        malformed("the response has two tool_use blocks with one id")
    })
}

/// The Messages API's column of the finish-reason table, in a closed match;
/// a value it doesn't name is kept as the API spelled it.
fn finish(stop_reason: String) -> FinishReason {
    match stop_reason.as_str() {
        "end_turn" | "stop_sequence" => FinishReason::EndTurn,
        "tool_use" => FinishReason::ToolUse,
        "max_tokens" => FinishReason::MaxTokens,
        "model_context_window_exceeded" => FinishReason::ContextWindow,
        "refusal" => FinishReason::Refusal,
        _ => FinishReason::from(stop_reason),
    }
}

/// One block by its `type`. A kind the domain has no name for is kept whole,
/// to be sent back as it came.
fn block(block: &serde_json::Value) -> Result<ContentBlock, ProviderError> {
    let kind = text_of(block, "type").ok_or_else(|| malformed("a content block has no type"))?;
    Ok(match kind {
        "text" => ContentBlock::Text(required(block, "text")?.to_owned()),
        "thinking" => ContentBlock::Thinking {
            text: required(block, "thinking")?.to_owned(),
            signature: Some(
                text_of(block, "signature")
                    .ok_or_else(|| malformed("a thinking block has no signature"))?
                    .to_owned(),
            ),
        },
        "redacted_thinking" => ContentBlock::RedactedThinking {
            data: required(block, "data")?.to_owned(),
        },
        // The domain's reason quotes the value it refused, which is the
        // server's text of any length; the message says which field instead.
        "tool_use" => ContentBlock::ToolUse(ToolUse {
            id: ToolCallId::new(required(block, "id")?)
                .map_err(|_| malformed("a tool_use block has an id the domain refuses"))?,
            name: ToolName::new(required(block, "name")?)
                .map_err(|_| malformed("a tool_use block has a name the domain refuses"))?,
            input: ToolInput::Json(
                block
                    .get("input")
                    .cloned()
                    .ok_or_else(|| malformed("a tool_use block has no input"))?,
            ),
        }),
        _ => ContentBlock::Opaque {
            provider: ProviderKind::Anthropic,
            payload: block.clone(),
        },
    })
}

fn text_of<'v>(block: &'v serde_json::Value, field: &str) -> Option<&'v str> {
    block.get(field).and_then(serde_json::Value::as_str)
}

fn required<'v>(block: &'v serde_json::Value, field: &str) -> Result<&'v str, ProviderError> {
    text_of(block, field).ok_or_else(|| {
        malformed(format!(
            "a {} block has no {field}",
            text_of(block, "type").unwrap_or("content")
        ))
    })
}

/// The failure of a 200 whose body is longer than `bound` bytes, which was
/// cut there unread. It's fatal, where a garbled body is retried: the model
/// finished the answer and it was billed, and the same request would bring
/// one past the bound again.
pub(crate) fn longer_than(bound: usize) -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Fatal,
        format!(
            "the response is longer than {} bytes; the answer was complete, so the run stops \
             rather than ask for it again",
            crate::error::grouped(bound)
        ),
    )
}

fn malformed(message: impl Into<String>) -> ProviderError {
    ProviderError::new(ProviderErrorKind::Malformed, message)
}

/// `text` with each double-quoted span replaced by `"..."`. serde quotes the
/// value it refused, in `Debug` form with `\"` for an inner quote, and that
/// value is a piece of the body, which the message never holds.
fn without_quoted(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            kept.push(c);
            continue;
        }
        kept.push_str("\"...\"");
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => break,
                _ => {}
            }
        }
    }
    kept
}

/// The refusal category as the API documents them, a short lowercase
/// identifier, or `other`, since a gateway's text of any length and content
/// is no diagnostic.
fn category_label(category: &str) -> &str {
    let identifier = !category.is_empty()
        && category.len() <= 32
        && category
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_');
    if identifier { category } else { "other" }
}

#[cfg(test)]
mod tests {
    use super::{category_label, without_quoted};

    #[test]
    fn a_quoted_value_leaves_serde_s_text_and_its_field_and_position_stay() {
        assert_eq!(
            without_quoted(
                r#"invalid type: string "a \"quoted\" value", expected u64 at line 1 column 9"#
            ),
            r#"invalid type: string "...", expected u64 at line 1 column 9"#
        );
        assert_eq!(
            without_quoted("missing field `usage` at line 1 column 80"),
            "missing field `usage` at line 1 column 80"
        );
        assert_eq!(
            without_quoted(r#"unterminated "tail"#),
            r#"unterminated "...""#
        );
    }

    #[test]
    fn a_category_that_isn_t_a_short_identifier_is_other() {
        assert_eq!(category_label("cyber"), "cyber");
        assert_eq!(
            category_label("reasoning_extraction"),
            "reasoning_extraction"
        );
        assert_eq!(category_label(""), "other");
        assert_eq!(category_label("Cyber"), "other");
        assert_eq!(
            category_label("the model declined; see https://x.example"),
            "other"
        );
        assert_eq!(category_label(&"a".repeat(33)), "other");
    }
}
