//! What a failed attempt becomes: the kind the retry policy reads, the
//! server's hint, and a message built from the provider's own `error.type`,
//! `error.message` and request id, or from the kind of transport failure,
//! and from nothing else.

use std::fmt::Write as _;
use std::time::Duration;

use lablet_model::{ProviderErrorKind, whole_ms};
use lablet_run::ProviderError;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;

use crate::wire::ErrorEnvelope;

/// What the adapter sent that bears on how a failure reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sent {
    /// Whether the request carried a `temperature`, so that a 400 naming a
    /// sampling parameter names the setting: the API's text for one isn't
    /// published, so the row is guarded by what was sent.
    pub temperature: bool,
    /// Whether the run captures content, which is the one case a body that
    /// isn't the API's error envelope is quoted in the message.
    pub capture_content: bool,
}

/// How a 400 for a prompt that outgrew the context window begins, after a
/// position prefix; the full text isn't published.
const CONTEXT_EXHAUSTED: &str = "prompt is too long";

/// The code of a 429 for a spend cap, which no wait lifts.
const SPEND_LIMIT: &str = "enforced_spend_limit_reached";

/// The failure of an attempt that reached `deadline`.
pub(crate) fn deadline(deadline: Duration) -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Retryable,
        format!(
            "the attempt reached its deadline of {} ms",
            whole_ms(deadline)
        ),
    )
}

/// A failure before any response was read, told apart by the error's kind
/// predicates alone: its text, and its source chain, can name the host and
/// a certificate's subject, so neither is shown.
pub(crate) fn transport(error: &reqwest::Error, deadline: Duration) -> ProviderError {
    if error.is_timeout() {
        return self::deadline(deadline);
    }
    if error.is_builder() {
        return ProviderError::new(ProviderErrorKind::Fatal, "the request couldn't be built");
    }
    let what = if error.is_dns() {
        "the host couldn't be resolved"
    } else if error.is_connect() {
        "the connection couldn't be made"
    } else if error.is_body() || error.is_decode() {
        "the connection was lost while the body was read"
    } else if error.is_request() {
        "the request couldn't be sent"
    } else {
        "the request failed"
    };
    ProviderError::new(ProviderErrorKind::Retryable, format!("transport: {what}"))
}

/// A response that isn't a success, classified by its status first and the
/// envelope's `error.type` and `error.message` second, so a gateway's HTML
/// 502 is retryable and a 401 with no envelope still says a key was turned
/// away.
pub(crate) fn classify(
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
    sent: Sent,
) -> ProviderError {
    if status.is_redirection() {
        return redirected(status, "");
    }
    let envelope: Option<ErrorEnvelope> = serde_json::from_slice(body).ok();
    let message = if let Some(envelope) = &envelope {
        let mut message = format!(
            "HTTP {} {}: {}",
            status.as_u16(),
            envelope.error.kind,
            envelope.error.message
        );
        if let Some(id) = request_id(headers, envelope) {
            let _ = write!(message, " (request-id {id})");
        }
        message
    } else {
        let mut message = format!(
            "HTTP {} with a body that isn't the API's error envelope ({} bytes)",
            status.as_u16(),
            grouped(body.len())
        );
        if sent.capture_content {
            message.push_str(": ");
            message.push_str(&String::from_utf8_lossy(body));
        }
        message
    };
    classified(
        status,
        headers,
        envelope.as_ref(),
        message,
        sent.temperature,
    )
}

/// A response that isn't a success whose body is longer than `bound` bytes,
/// which was cut there unread: classified by its status and headers alone,
/// since part of a body can't be read as the envelope, and never quoted.
pub(crate) fn longer_than(status: StatusCode, headers: &HeaderMap, bound: usize) -> ProviderError {
    let cut = format!(
        "a body longer than {} bytes, which was cut there and isn't quoted",
        grouped(bound)
    );
    if status.is_redirection() {
        return redirected(status, &format!("; it came with {cut}"));
    }
    let message = format!("HTTP {} with {cut}", status.as_u16());
    classified(status, headers, None, message, false)
}

/// A 3xx, which the client never follows, with `more` said after it.
fn redirected(status: StatusCode, more: &str) -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Fatal,
        format!(
            "HTTP {} from the endpoint: the base URL redirects; set model.base_url to the \
             address it redirects to{more}",
            status.as_u16()
        ),
    )
}

/// The failure of a response that isn't a success, from its `message`:
/// the kind the table gives its status and `envelope`, made fatal where the
/// server said not to retry, with the settings a 400's text names when the
/// request carried what they name, and the hint of a retryable one.
fn classified(
    status: StatusCode,
    headers: &HeaderMap,
    envelope: Option<&ErrorEnvelope>,
    mut message: String,
    sent_temperature: bool,
) -> ProviderError {
    let (mut kind, hint) = kind_of(status, envelope, headers);
    if kind == ProviderErrorKind::Retryable && should_not_retry(headers) {
        message.push_str("; the server said not to retry");
        kind = ProviderErrorKind::Fatal;
    }
    if status == StatusCode::BAD_REQUEST
        && kind == ProviderErrorKind::Fatal
        && let Some(envelope) = envelope
        && let Some(named) = settings_named(&envelope.error.message, sent_temperature)
    {
        message.push_str(&named);
    }
    let error = ProviderError::new(kind, message);
    match hint {
        Some(wait) if kind == ProviderErrorKind::Retryable => error.with_retry_after(wait),
        _ => error,
    }
}

/// The table: the kind each status is, and the hint the retryable ones
/// carry when the server sent one.
fn kind_of(
    status: StatusCode,
    envelope: Option<&ErrorEnvelope>,
    headers: &HeaderMap,
) -> (ProviderErrorKind, Option<Duration>) {
    let hint = retry_after(headers);
    match status.as_u16() {
        400 => {
            if envelope.is_some_and(|envelope| context_exhausted(&envelope.error.message)) {
                (ProviderErrorKind::ContextExhausted, None)
            } else {
                (ProviderErrorKind::Fatal, None)
            }
        }
        401 | 403 => (ProviderErrorKind::Auth, None),
        408 | 409 | 500..=599 => (ProviderErrorKind::Retryable, hint),
        429 => {
            let code = envelope
                .and_then(|envelope| envelope.error.details.as_ref())
                .and_then(|details| details.error_code.as_deref());
            if code == Some(SPEND_LIMIT) {
                (ProviderErrorKind::Fatal, None)
            } else {
                (ProviderErrorKind::Retryable, hint)
            }
        }
        _ => (ProviderErrorKind::Fatal, None),
    }
}

/// Whether `message` is the 400 for a prompt that outgrew the window: it
/// begins with [`CONTEXT_EXHAUSTED`], in any case, after a `messages...:`
/// position prefix when there is one.
fn context_exhausted(message: &str) -> bool {
    let text = message
        .strip_prefix("messages")
        .and_then(|rest| rest.split_once(": "))
        .map_or(message, |(_, after)| after)
        .trim_start();
    text.get(..CONTEXT_EXHAUSTED.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(CONTEXT_EXHAUSTED))
}

/// The settings a 400's text names, as `; the setting is model.thinking`.
/// The table is over the API's own field words, which don't change with
/// model releases, where a table of models would. It's read over the
/// complaint, the text before the first sentence's end, since the sentence
/// after it names the field to use instead.
fn settings_named(message: &str, sent_temperature: bool) -> Option<String> {
    let complaint = message.split(". ").next().unwrap_or(message);
    let mentions = |words: &[&str]| words.iter().any(|word| complaint.contains(word));
    let mut named = Vec::new();
    if mentions(&["thinking.type"])
        || (mentions(&["thinking"])
            && mentions(&[
                "adaptive",
                "enabled",
                "disabled",
                "between_tools",
                "budget_tokens",
            ]))
    {
        named.push("model.thinking");
        if mentions(&["display"]) {
            named.push("model.thinking_display");
        }
    }
    if mentions(&["effort"]) {
        named.push("model.effort");
    }
    if sent_temperature && mentions(&["temperature", "top_p", "top_k"]) {
        named.push("model.temperature");
    }
    if mentions(&["max_tokens"]) {
        named.push("model.max_tokens");
    }
    match named.as_slice() {
        [] => None,
        [one] => Some(format!("; the setting is {one}")),
        [rest @ .., last] => Some(format!("; the settings are {} and {last}", rest.join(", "))),
    }
}

/// `retry-after` as a count of seconds; any other form is no hint.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
        .map(Duration::from_secs)
}

/// Whether the server said, with `x-should-retry: false`, that another
/// attempt can't help. `true` is read and not acted on.
fn should_not_retry(headers: &HeaderMap) -> bool {
    headers
        .get("x-should-retry")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("false"))
}

/// The request's id, from the header first and the envelope second.
fn request_id<'a>(headers: &'a HeaderMap, envelope: &'a ErrorEnvelope) -> Option<&'a str> {
    headers
        .get("request-id")
        .and_then(|value| value.to_str().ok())
        .or(envelope.request_id.as_deref())
}

/// `count` with a comma every three digits.
pub(crate) fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests;
