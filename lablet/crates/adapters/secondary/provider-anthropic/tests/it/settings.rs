//! A 400 for a setting the model rejects is fatal, and its message names
//! the lablet setting the provider's words name.

use lablet_model::{Message, ProviderErrorKind};
use lablet_run::{ModelProvider as _, ProviderError};
use wiremock::MockServer;

use crate::harness;

/// What the 400 `message` comes to, with or without a `temperature` sent.
async fn rejected(message: &str, temperature: Option<f64>) -> ProviderError {
    let server = MockServer::start().await;
    harness::answer(
        &server,
        harness::envelope(400, "invalid_request_error", message),
    )
    .await;
    let provider = harness::provider(&server);
    let input = harness::prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];
    let mut request = harness::request(&messages, &[]);
    request.temperature = temperature;
    provider.complete(request).await.unwrap_err()
}

/// Checks that `message` is fatal and ends with `suffix`.
async fn names(message: &str, temperature: Option<f64>, suffix: &str) {
    let error = rejected(message, temperature).await;
    assert_eq!(error.kind, ProviderErrorKind::Fatal, "{message}");
    assert_eq!(
        error.message(),
        format!(
            "HTTP 400 invalid_request_error: {message} (request-id req_011CSHoEeqs5C35K2UUqR7Fy){suffix}"
        )
    );
}

#[tokio::test]
async fn a_thinking_type_the_model_rejects_names_model_thinking() {
    for message in [
        "\"thinking.type.enabled\" is not supported for this model. Use \"thinking.type.adaptive\" \
         and \"output_config.effort\" to control thinking behavior.",
        "\"thinking.type.disabled\" is not supported for this model. Use \
         \"thinking.type.between_tools\" for the lowest thinking setting, or \
         \"thinking.type.adaptive\" and \"output_config.effort\" to control thinking behavior.",
        "To turn thinking off on this model, send \"thinking\": {\"type\": \"between_tools\"} \
         instead of {\"type\": \"disabled\"}. The model does not think before responding.",
        "adaptive thinking is not supported on this model",
        "thinking.budget_tokens: Input should be greater than or equal to 1024",
    ] {
        names(message, None, "; the setting is model.thinking").await;
    }
}

#[tokio::test]
async fn a_display_the_model_rejects_names_model_thinking_display_beside_model_thinking() {
    names(
        "thinking.type.adaptive: display 'updates' is not supported for this model",
        None,
        "; the settings are model.thinking and model.thinking_display",
    )
    .await;
}

#[tokio::test]
async fn an_effort_the_model_rejects_names_model_effort_and_the_thinking_it_mentions() {
    names(
        "output_config.effort 'xhigh' is not supported when thinking is disabled on this model. \
         Use effort 'high' or below, or enable thinking.",
        None,
        "; the settings are model.thinking and model.effort",
    )
    .await;
    names(
        "output_config.effort: Input should be 'low', 'medium', 'high', 'xhigh' or 'max'",
        None,
        "; the setting is model.effort",
    )
    .await;
}

#[tokio::test]
async fn a_sampling_parameter_names_model_temperature_only_when_one_was_sent() {
    names(
        "temperature: Extra inputs are not permitted",
        Some(0.5),
        "; the setting is model.temperature",
    )
    .await;
    names(
        "top_p: Extra inputs are not permitted",
        Some(0.5),
        "; the setting is model.temperature",
    )
    .await;
    names("temperature: Extra inputs are not permitted", None, "").await;
}

#[tokio::test]
async fn a_max_tokens_the_model_rejects_names_model_max_tokens() {
    names(
        "max_tokens: 200000 > 64000, which is the maximum allowed number of output tokens for \
         claude-sonnet-5-5",
        None,
        "; the setting is model.max_tokens",
    )
    .await;
    names(
        "thinking.budget_tokens: 8000 must be less than max_tokens: 4096",
        None,
        "; the settings are model.thinking and model.max_tokens",
    )
    .await;
}

#[tokio::test]
async fn a_400_that_names_no_setting_adds_nothing() {
    names(
        "messages: roles must alternate between user and assistant",
        Some(0.5),
        "",
    )
    .await;
    names(
        "messages.3.content.1: `thinking` or `redacted_thinking` blocks in the latest assistant \
         message cannot be modified. These blocks must remain as they were in the original \
         response.",
        None,
        "",
    )
    .await;
}
