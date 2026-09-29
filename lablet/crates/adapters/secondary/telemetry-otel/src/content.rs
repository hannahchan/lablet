//! Captured content, in the JSON forms the GenAI conventions give it.
//!
//! Every value here is a JSON string. The conventions would rather have a
//! structured value on a log record, and the SDK's can't hold one as it was
//! written: it has no null, which a tool's arguments may hold, and its maps
//! keep no order. A string is also what the limit on a value's length can
//! cut.

use std::collections::HashMap;

use lablet_model::{ContentBlock, ToolCallId, ToolInput, ToolResultContent, ToolSource, ToolSpec};
use serde_json::{Value, json};

/// The `gen_ai.tool.type` of a tool from `source`: a built-in tool runs in
/// lablet's own process, and an MCP tool is served from outside it.
pub(crate) const fn tool_type(source: &ToolSource) -> &'static str {
    match source {
        ToolSource::Builtin => "function",
        ToolSource::Mcp { .. } => "extension",
    }
}

/// The tools a run offered, for `gen_ai.tool.definitions`.
pub(crate) fn tool_definitions(specs: &[ToolSpec]) -> String {
    Value::Array(
        specs
            .iter()
            .map(|spec| {
                json!({
                    "type": tool_type(&spec.source),
                    "name": spec.name.as_str(),
                    "description": spec.description,
                    "parameters": spec.input_schema,
                })
            })
            .collect(),
    )
    .to_string()
}

/// What a tool call returned, for `gen_ai.tool.call.result` and for the
/// part of a message that answers a call. It's the shape MCP gives a tool's
/// result, which is an object, as the conventions ask.
pub(crate) fn tool_result(content: &[ToolResultContent], is_error: bool) -> Value {
    let content: Vec<_> = content
        .iter()
        .map(|ToolResultContent::Text(text)| json!({ "type": "text", "text": text }))
        .collect();
    json!({ "content": content, "isError": is_error })
}

/// One block of a response, as a part of a message.
///
/// The conventions name no part for reasoning a provider encrypted or for a
/// block only its provider understands, so each is a part of lablet's own
/// type, which the schema allows, and holds what the block held.
fn part(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text(text) => json!({ "type": "text", "content": text }),
        ContentBlock::Thinking { text, .. } => json!({ "type": "reasoning", "content": text }),
        ContentBlock::RedactedThinking { data } => {
            json!({ "type": "redacted_reasoning", "data": data })
        }
        ContentBlock::ToolUse(call) => {
            let arguments = match &call.input {
                ToolInput::Json(arguments) => arguments.clone(),
                ToolInput::Unparsed(text) => Value::String(text.clone()),
            };
            json!({
                "type": "tool_call",
                "id": call.id.as_str(),
                "name": call.name.as_str(),
                "arguments": arguments,
            })
        }
        ContentBlock::Opaque { provider, payload } => {
            json!({ "type": "opaque", "provider": provider.as_str(), "payload": payload })
        }
    }
}

fn message(role: &str, parts: Vec<Value>) -> Value {
    json!({ "role": role, "parts": Value::Array(parts) })
}

/// The tool calls of `response`, in call order.
fn calls(response: &[ContentBlock]) -> Vec<ToolCallId> {
    response
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse(call) => Some(call.id.clone()),
            _ => None,
        })
        .collect()
}

/// What one provider call was sent and what it answered, as the three
/// attributes of its content record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Exchange {
    /// `gen_ai.system_instructions`.
    pub(crate) system: String,
    /// `gen_ai.input.messages`.
    pub(crate) input: String,
    /// `gen_ai.output.messages`; `None` for an attempt that failed.
    pub(crate) output: Option<String>,
}

/// The conversation of a run that captures content, as its events have told
/// it so far.
///
/// The loop sends an observer what each step produced and never the
/// messages of a request, so the observer keeps them: each provider call is
/// sent every message before it.
#[derive(Debug, Clone)]
pub(crate) struct Conversation {
    system: String,
    /// The messages so far, each as JSON, with a comma between one and the
    /// next. A message is written once, when it joins the conversation, and
    /// every call after it is sent the text.
    messages: String,
    /// The calls of the last response, in call order, which is the order
    /// the model is sent their results in.
    unanswered: Vec<ToolCallId>,
    /// The results that have come in for them, in whatever order the calls
    /// finished.
    results: HashMap<ToolCallId, Value>,
}

impl Conversation {
    /// The conversation of a run with this system prompt, which opens with
    /// the task `prompt`.
    pub(crate) fn opening(system: &str, prompt: &str) -> Self {
        let mut conversation = Self {
            system: Value::Array(vec![json!({ "type": "text", "content": system })]).to_string(),
            messages: String::new(),
            unanswered: Vec::new(),
            results: HashMap::new(),
        };
        conversation.join(&message(
            "user",
            vec![json!({ "type": "text", "content": prompt })],
        ));
        conversation
    }

    /// `message` follows the messages so far.
    fn join(&mut self, message: &Value) {
        if !self.messages.is_empty() {
            self.messages.push(',');
        }
        self.messages.push_str(&message.to_string());
    }

    /// What a tool call returned.
    pub(crate) fn answered(
        &mut self,
        call: &ToolCallId,
        content: &[ToolResultContent],
        is_error: bool,
    ) {
        self.results
            .insert(call.clone(), tool_result(content, is_error));
    }

    /// The results of the last response's calls become the message that
    /// follows it, in call order. A call with no result was never run, and
    /// the model was sent nothing for it.
    fn settle(&mut self) {
        let parts: Vec<_> = std::mem::take(&mut self.unanswered)
            .into_iter()
            .filter_map(|call| {
                let response = self.results.remove(&call)?;
                Some(json!({
                    "type": "tool_call_response",
                    "id": call.as_str(),
                    "response": response,
                }))
            })
            .collect();
        self.results.clear();
        if !parts.is_empty() {
            self.join(&message("tool", parts));
        }
    }

    fn input(&self) -> String {
        format!("[{}]", self.messages)
    }

    /// What an attempt that failed was sent.
    pub(crate) fn unanswered(&mut self) -> Exchange {
        self.settle();
        Exchange {
            system: self.system.clone(),
            input: self.input(),
            output: None,
        }
    }

    /// What the attempt that returned `response` was sent, and the
    /// response, which joins the conversation.
    pub(crate) fn responded(&mut self, response: &[ContentBlock]) -> Exchange {
        self.settle();
        let input = self.input();
        let answer = message("assistant", response.iter().map(part).collect());
        self.join(&answer);
        self.unanswered = calls(response);
        let output = Value::Array(vec![answer]).to_string();
        Exchange {
            system: self.system.clone(),
            input,
            output: Some(output),
        }
    }
}

#[cfg(test)]
mod tests;
