//! What the two port errors hold, whatever adapter built them.

use std::time::Duration;

use lablet_model::{ProviderErrorKind, TokenCounts, Usage};

use crate::{ERROR_MESSAGE_MAX_BYTES, ProviderError, ToolError, ToolErrorKind};

/// Both errors' messages, built from the same text.
fn messages(text: &str) -> [String; 2] {
    [
        ProviderError::new(ProviderErrorKind::Fatal, text)
            .message()
            .to_owned(),
        ToolError::new(ToolErrorKind::Failed, text)
            .message()
            .to_owned(),
    ]
}

#[test]
fn a_message_up_to_the_bound_is_kept_whole() {
    for text in [
        String::new(),
        "400 bad request".to_owned(),
        "x".repeat(ERROR_MESSAGE_MAX_BYTES),
    ] {
        assert_eq!(messages(&text), [text.clone(), text]);
    }
}

#[test]
fn a_message_over_the_bound_is_cut_to_it() {
    let long = "x".repeat(ERROR_MESSAGE_MAX_BYTES + 1);

    assert_eq!(ERROR_MESSAGE_MAX_BYTES, 2_048);
    for message in messages(&long) {
        assert_eq!(message, "x".repeat(ERROR_MESSAGE_MAX_BYTES));
    }
    for message in messages(&"x".repeat(1_000_000)) {
        assert_eq!(message.len(), ERROR_MESSAGE_MAX_BYTES);
    }
}

/// The bound counts bytes, and a character that would straddle it is left
/// out whole, so a cut message is still text.
#[test]
fn a_message_is_cut_at_the_last_character_boundary_the_bound_allows() {
    let straddling = format!("{}é", "x".repeat(ERROR_MESSAGE_MAX_BYTES - 1));
    let fitting = format!("{}é", "x".repeat(ERROR_MESSAGE_MAX_BYTES - 2));

    for message in messages(&straddling) {
        assert_eq!(message, "x".repeat(ERROR_MESSAGE_MAX_BYTES - 1));
    }
    assert_eq!(messages(&fitting), [fitting.clone(), fitting]);
}

#[test]
fn an_error_prints_as_its_message() {
    assert_eq!(
        ProviderError::new(ProviderErrorKind::Retryable, "529 overloaded").to_string(),
        "529 overloaded"
    );
    assert_eq!(
        ToolError::new(ToolErrorKind::Timeout, "bash ran past 30s").to_string(),
        "bash ran past 30s"
    );
}

#[test]
fn a_provider_error_reports_no_usage_and_no_hint_until_it_is_given_them() {
    let usage = Usage::from_inclusive(TokenCounts {
        input: 70,
        output: 5,
        ..TokenCounts::default()
    });
    let bare = ProviderError::new(ProviderErrorKind::Retryable, "529 overloaded");

    assert_eq!(bare.kind, ProviderErrorKind::Retryable);
    assert_eq!(bare.usage, None);
    assert_eq!(bare.retry_after, None);

    let billed = bare.clone().with_usage(usage);
    assert_eq!(billed.usage, Some(usage));
    assert_eq!(billed.retry_after, None);

    let throttled = billed.with_retry_after(Duration::from_secs(5));
    assert_eq!(throttled.usage, Some(usage));
    assert_eq!(throttled.retry_after, Some(Duration::from_secs(5)));
    assert_eq!(throttled.kind, bare.kind);
    assert_eq!(throttled.message(), bare.message());
}
