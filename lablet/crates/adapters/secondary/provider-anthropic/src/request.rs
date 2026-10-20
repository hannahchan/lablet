//! From a `ProviderRequest` to the request body: the fields, the messages
//! and their blocks, and the cache breakpoints.

use std::borrow::Cow;

use lablet_model::{
    ContentBlock, Message, ProviderErrorKind, ProviderKind, Thinking, ThinkingDisplay, ToolInput,
    ToolResult, ToolResultContent, ToolSpec, UserContent,
};
use lablet_run::{ProviderError, ProviderRequest};

use crate::DIAGNOSTICS;
use crate::target::Target;
use crate::wire::{
    Block, CacheControl, Known, MessageParam, OutputConfig, Request, SystemBlock, ThinkingParam,
    Tool,
};

/// The body of one attempt of `request` to `target`.
///
/// `system` is always the array form, so that one shape serves with and
/// without the run-id block and a marker has a block to sit on; an empty
/// prompt sends no `system` key, since an empty block is refused. The three
/// breakpoints, with `cache` on, are the prompt's block, the last tool and
/// the last block of the last message. The run-id block never carries one:
/// it's fixed for the run, so the prefix from it on is this run's alone
/// whether or not anything is cached.
///
/// # Errors
///
/// A fatal failure, before anything is sent, for a conversation that holds a
/// block of another provider: it recurs on every attempt, so a kind the loop
/// retries would spend the budget on it.
pub(crate) fn body<'a>(
    request: &'a ProviderRequest<'a>,
    target: &'a Target,
) -> Result<Request<'a>, ProviderError> {
    let marker = target.cache().then_some(CacheControl::Ephemeral);
    let mut system = Vec::new();
    if let Some(run) = request.cache_key {
        system.push(SystemBlock {
            text: Cow::Owned(format!("lablet run {run}")),
            cache_control: None,
        });
    }
    if !request.system.is_empty() {
        system.push(SystemBlock {
            text: Cow::Borrowed(request.system),
            cache_control: marker,
        });
    }
    let mut tools: Vec<Tool<'a>> = request.tools.iter().map(tool).collect();
    if let Some(last) = tools.last_mut() {
        last.cache_control = marker;
    }
    let mut messages = request
        .messages
        .iter()
        .map(message)
        .collect::<Result<Vec<_>, _>>()?;
    if marker.is_some()
        && let Some(last) = messages.last_mut().and_then(|last| last.content.last_mut())
    {
        last.mark();
    }
    Ok(Request {
        model: &target.model().name,
        max_tokens: request.max_tokens,
        system,
        messages,
        tools,
        thinking: thinking(request.thinking, request.thinking_display),
        output_config: request.effort.map(|effort| OutputConfig {
            effort: effort.as_str(),
        }),
        temperature: request.temperature,
    })
}

fn tool(spec: &ToolSpec) -> Tool<'_> {
    Tool {
        name: spec.name.as_str(),
        description: &spec.description,
        input_schema: &spec.input_schema,
        cache_control: None,
    }
}

/// The `thinking` parameter. `display` goes inside the `adaptive` or
/// `enabled` object; with nothing sent, or `disabled`, there's no object to
/// carry it, and sending one would turn thinking on where leaving the key
/// out doesn't, so it isn't sent.
fn thinking(thinking: Thinking, display: Option<ThinkingDisplay>) -> Option<ThinkingParam> {
    let display = display.map(ThinkingDisplay::as_str);
    let without_display = || {
        if display.is_some() {
            tracing::debug!(
                target: DIAGNOSTICS,
                "thinking_display is stated, and the thinking setting has no object to carry it, so it isn't sent"
            );
        }
    };
    match thinking {
        Thinking::ProviderDefault => {
            without_display();
            None
        }
        Thinking::Adaptive => Some(ThinkingParam::Adaptive { display }),
        Thinking::Budget(budget) => Some(ThinkingParam::Enabled {
            budget_tokens: budget.get(),
            display,
        }),
        Thinking::Disabled => {
            without_display();
            Some(ThinkingParam::Disabled)
        }
    }
}

/// One message: a user message holds its tool results, in call order, then
/// its input, since the API takes a turn's results in one user message
/// ahead of its text; an assistant message holds its blocks in order.
fn message<'a>(message: &'a Message<'a>) -> Result<MessageParam<'a>, ProviderError> {
    match message {
        Message::User {
            tool_results,
            input,
        } => {
            let mut content: Vec<Block<'a>> = tool_results.iter().map(tool_result).collect();
            content.extend(
                input
                    .iter()
                    .map(|UserContent::Text(text)| Block::text(text)),
            );
            Ok(MessageParam {
                role: "user",
                content,
            })
        }
        Message::Assistant(blocks) => Ok(MessageParam {
            role: "assistant",
            content: blocks
                .iter()
                .map(assistant_block)
                .collect::<Result<_, _>>()?,
        }),
    }
}

fn tool_result<'a>(result: &ToolResult<'a>) -> Block<'a> {
    let content: Vec<Block<'a>> = result
        .content
        .iter()
        .map(|ToolResultContent::Text(text)| Block::text(text))
        .collect();
    Block::Known(Known::ToolResult {
        tool_use_id: result.call_id.as_str(),
        content: (!content.is_empty()).then_some(content),
        is_error: result.is_error,
        cache_control: None,
    })
}

/// One block of a model response, sent back as it came: the `thinking` text
/// even when empty, since the API requires it, and the signature when the
/// provider gave one.
fn assistant_block(block: &ContentBlock) -> Result<Block<'_>, ProviderError> {
    Ok(match block {
        ContentBlock::Text(text) => Block::text(text),
        ContentBlock::Thinking { text, signature } => Block::Known(Known::Thinking {
            thinking: text,
            signature: signature.as_deref(),
        }),
        ContentBlock::RedactedThinking { data } => Block::Known(Known::RedactedThinking { data }),
        ContentBlock::ToolUse(call) => Block::Known(Known::ToolUse {
            id: call.id.as_str(),
            name: call.name.as_str(),
            input: match &call.input {
                ToolInput::Json(value) => Cow::Borrowed(value),
                // The API hands a tool's input over parsed, so this is a
                // block a script wrote; an empty object is what the API
                // takes for arguments that aren't JSON.
                ToolInput::Unparsed(_) => {
                    tracing::debug!(
                        target: DIAGNOSTICS,
                        "a tool call's arguments didn't parse, so an empty object is sent in their place"
                    );
                    Cow::Owned(serde_json::Value::Object(serde_json::Map::new()))
                }
            },
        }),
        ContentBlock::Opaque {
            provider: ProviderKind::Anthropic,
            payload,
        } => Block::Opaque(payload),
        ContentBlock::Opaque { provider, .. } => {
            return Err(ProviderError::new(
                ProviderErrorKind::Fatal,
                format!(
                    "the conversation holds a block of provider {provider}, which the Anthropic \
                     adapter can't send"
                ),
            ));
        }
    })
}
