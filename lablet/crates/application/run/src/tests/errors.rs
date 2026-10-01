//! What the two port errors hold, whatever adapter built them.

use std::time::Duration;

use lablet_model::{ProviderErrorKind, TokenCounts, Usage};

use crate::{ERROR_MESSAGE_MAX_BYTES, ProviderError, ToolError, ToolErrorKind};

/// A provider error's message, built from `text`.
fn provider_message(text: &str) -> String {
    ProviderError::new(ProviderErrorKind::Fatal, text)
        .message()
        .to_owned()
}

#[test]
fn a_provider_error_s_message_up_to_the_bound_is_kept_whole() {
    for text in [
        String::new(),
        "400 bad request".to_owned(),
        "x".repeat(ERROR_MESSAGE_MAX_BYTES),
    ] {
        assert_eq!(provider_message(&text), text);
    }
}

#[test]
fn a_provider_error_s_message_over_the_bound_is_cut_to_it() {
    let long = "x".repeat(ERROR_MESSAGE_MAX_BYTES + 1);

    assert_eq!(ERROR_MESSAGE_MAX_BYTES, 2_048);
    assert_eq!(provider_message(&long), "x".repeat(ERROR_MESSAGE_MAX_BYTES));
    assert_eq!(
        provider_message(&"x".repeat(1_000_000)).len(),
        ERROR_MESSAGE_MAX_BYTES
    );
}

/// The bound counts bytes, and a character that would straddle it is left
/// out whole, so a cut message is still text.
#[test]
fn a_provider_error_s_message_is_cut_at_the_last_character_boundary_the_bound_allows() {
    let straddling = format!("{}é", "x".repeat(ERROR_MESSAGE_MAX_BYTES - 1));
    let fitting = format!("{}é", "x".repeat(ERROR_MESSAGE_MAX_BYTES - 2));

    assert_eq!(
        provider_message(&straddling),
        "x".repeat(ERROR_MESSAGE_MAX_BYTES - 1)
    );
    assert_eq!(provider_message(&fitting), fitting);
}

/// The loop bounds a tool error's message, once the run's secrets are cut
/// out of it, so the error holds it whole: a bound made here would leave a
/// value's edge at the bound for the cut to miss.
#[test]
fn a_tool_error_holds_its_message_whole_for_the_loop_to_cut_and_then_bound() {
    let long = "x".repeat(ERROR_MESSAGE_MAX_BYTES * 4);

    let error = ToolError::new(ToolErrorKind::Failed, long.clone());

    assert_eq!(error.message(), long);
    assert_eq!(error.to_string(), long);
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
