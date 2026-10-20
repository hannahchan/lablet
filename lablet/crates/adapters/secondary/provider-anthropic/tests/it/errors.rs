//! P2 and every row of the error table: what each response is classified
//! as, the hint it carries, and the message's exact form.

use std::time::Duration;

use lablet_model::{ContentBlock, Message, ProviderErrorKind, ProviderKind};
use lablet_run::{ModelProvider as _, ProviderError};
use lablet_test_support::refused_address;
use serde_json::json;
use wiremock::{MockServer, ResponseTemplate};

use crate::harness::{self, KEY, MODEL};

/// What one attempt answered with `template` comes to.
async fn classified(template: ResponseTemplate) -> ProviderError {
    let server = MockServer::start().await;
    harness::answer(&server, template).await;
    let provider = harness::provider(&server);
    let error = harness::complete(&provider).await.unwrap_err();
    assert_eq!(error.usage, None, "the envelope carries no usage");
    error
}

#[tokio::test]
async fn a_429_is_retryable_with_the_retry_after_header_as_its_hint() {
    let error = classified(
        harness::envelope(
            429,
            "rate_limit_error",
            "This request would exceed your organization's rate limit",
        )
        .insert_header("retry-after", "7")
        .insert_header("request-id", "req_hdr_01"),
    )
    .await;

    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(error.retry_after, Some(Duration::from_secs(7)));
    assert_eq!(
        error.message(),
        "HTTP 429 rate_limit_error: This request would exceed your organization's rate limit \
         (request-id req_hdr_01)"
    );
}

#[tokio::test]
async fn a_429_without_the_header_has_no_hint_and_names_the_envelope_s_request_id() {
    let error = classified(harness::envelope(429, "rate_limit_error", "Rate limited")).await;

    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(error.retry_after, None);
    assert_eq!(
        error.message(),
        "HTTP 429 rate_limit_error: Rate limited (request-id req_011CSHoEeqs5C35K2UUqR7Fy)"
    );
}

#[tokio::test]
async fn a_retry_after_that_isn_t_a_count_of_seconds_is_no_hint() {
    let error = classified(
        harness::envelope(429, "rate_limit_error", "Rate limited")
            .insert_header("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT"),
    )
    .await;

    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(error.retry_after, None);
}

#[tokio::test]
async fn a_429_for_a_spend_cap_is_fatal_with_no_hint() {
    let error = classified(
        ResponseTemplate::new(429)
            .set_body_json(json!({
                "type": "error",
                "error": {
                    "type": "rate_limit_error",
                    "message": "Your organization has reached its monthly spend limit",
                    "details": { "error_code": "enforced_spend_limit_reached" }
                },
                "request_id": "req_cap"
            }))
            .insert_header("retry-after", "60"),
    )
    .await;

    assert_eq!(error.kind, ProviderErrorKind::Fatal);
    assert_eq!(error.retry_after, None);
    assert_eq!(
        error.message(),
        "HTTP 429 rate_limit_error: Your organization has reached its monthly spend limit \
         (request-id req_cap)"
    );
}

#[tokio::test]
async fn a_529_a_500_a_503_a_504_a_408_and_a_409_are_retryable() {
    for (status, kind, message) in [
        (529, "overloaded_error", "Overloaded"),
        (
            500,
            "api_error",
            "An unexpected error has occurred internal to Anthropic's systems.",
        ),
        (503, "api_error", "Service unavailable"),
        (504, "timeout_error", "Request timed out"),
        (408, "timeout_error", "Request timed out"),
        (409, "conflict_error", "Conflict"),
    ] {
        let error = classified(harness::envelope(status, kind, message)).await;

        assert_eq!(error.kind, ProviderErrorKind::Retryable, "HTTP {status}");
        assert_eq!(
            error.message(),
            format!("HTTP {status} {kind}: {message} (request-id req_011CSHoEeqs5C35K2UUqR7Fy)")
        );
    }
}

#[tokio::test]
async fn a_5xx_carries_the_retry_after_header_as_its_hint() {
    let error = classified(
        harness::envelope(529, "overloaded_error", "Overloaded").insert_header("retry-after", "3"),
    )
    .await;

    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(error.retry_after, Some(Duration::from_secs(3)));
}

#[tokio::test]
async fn a_body_that_isn_t_the_envelope_is_classified_by_status_and_quoted_only_when_content_is_captured()
 {
    let html = "<html><body>502 Bad Gateway</body></html>";
    let error = classified(ResponseTemplate::new(502).set_body_string(html)).await;
    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(
        error.message(),
        format!(
            "HTTP 502 with a body that isn't the API's error envelope ({} bytes)",
            html.len()
        )
    );

    let server = MockServer::start().await;
    harness::answer(&server, ResponseTemplate::new(502).set_body_string(html)).await;
    let capturing = harness::with(
        harness::target(&server, false),
        opentelemetry::trace::noop::NoopTextMapPropagator::new(),
        true,
    );
    let error = harness::complete(&capturing).await.unwrap_err();
    assert_eq!(
        error.message(),
        format!(
            "HTTP 502 with a body that isn't the API's error envelope ({} bytes): {html}",
            html.len()
        )
    );

    let error = classified(ResponseTemplate::new(401).set_body_string("denied")).await;
    assert_eq!(
        error.kind,
        ProviderErrorKind::Auth,
        "a 401 with no envelope is still a key turned away"
    );
}

#[tokio::test]
async fn a_400_for_a_prompt_that_outgrew_the_window_is_context_exhausted() {
    for message in [
        "prompt is too long: 210000 tokens > 200000 maximum",
        "messages.12.content.0: Prompt is too long",
    ] {
        let error = classified(harness::envelope(400, "invalid_request_error", message)).await;

        assert_eq!(error.kind, ProviderErrorKind::ContextExhausted, "{message}");
    }
}

#[tokio::test]
async fn any_other_400_is_fatal_with_the_provider_s_words() {
    let error = classified(harness::envelope(
        400,
        "invalid_request_error",
        "messages: roles must alternate between \"user\" and \"assistant\"",
    ))
    .await;

    assert_eq!(error.kind, ProviderErrorKind::Fatal);
    assert_eq!(
        error.message(),
        "HTTP 400 invalid_request_error: messages: roles must alternate between \"user\" and \
         \"assistant\" (request-id req_011CSHoEeqs5C35K2UUqR7Fy)"
    );
}

#[tokio::test]
async fn a_401_and_a_403_are_auth() {
    for (status, kind) in [(401, "authentication_error"), (403, "permission_error")] {
        let error = classified(harness::envelope(status, kind, "invalid x-api-key")).await;

        assert_eq!(error.kind, ProviderErrorKind::Auth, "HTTP {status}");
    }
}

#[tokio::test]
async fn a_402_a_404_a_413_and_any_other_4xx_are_fatal() {
    for (status, kind, message) in [
        (402, "billing_error", "Your account has a billing problem"),
        (404, "not_found_error", "model: claude-sonnet-6"),
        (
            413,
            "request_too_large",
            "Request exceeds the maximum size of 32 MB",
        ),
        (422, "invalid_request_error", "Unprocessable"),
    ] {
        let error = classified(harness::envelope(status, kind, message)).await;

        assert_eq!(error.kind, ProviderErrorKind::Fatal, "HTTP {status}");
        assert!(
            error
                .message()
                .starts_with(&format!("HTTP {status} {kind}: {message}"))
        );
    }
}

#[tokio::test]
async fn x_should_retry_false_makes_a_retryable_response_fatal_and_true_changes_nothing() {
    let error = classified(
        harness::envelope(529, "overloaded_error", "Overloaded")
            .insert_header("x-should-retry", "false"),
    )
    .await;
    assert_eq!(error.kind, ProviderErrorKind::Fatal);
    assert_eq!(
        error.message(),
        "HTTP 529 overloaded_error: Overloaded (request-id req_011CSHoEeqs5C35K2UUqR7Fy); the \
         server said not to retry"
    );
    assert_eq!(error.retry_after, None);

    let error = classified(
        harness::envelope(529, "overloaded_error", "Overloaded")
            .insert_header("x-should-retry", "true"),
    )
    .await;
    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert!(!error.message().contains("not to retry"));

    let error = classified(
        harness::envelope(400, "invalid_request_error", "bad request")
            .insert_header("x-should-retry", "true"),
    )
    .await;
    assert_eq!(
        error.kind,
        ProviderErrorKind::Fatal,
        "true never makes a failure retryable"
    );
    assert!(!error.message().contains("not to retry"));
}

#[tokio::test]
async fn a_refused_connection_is_retryable_and_names_neither_the_host_nor_the_port() {
    let target = lablet_provider_anthropic::Target::new(
        MODEL,
        lablet_provider_anthropic::Key(KEY),
        Some(&format!("http://{}", refused_address())),
        false,
    )
    .unwrap();
    let provider = harness::with(
        target,
        opentelemetry::trace::noop::NoopTextMapPropagator::new(),
        false,
    );

    let error = harness::complete(&provider).await.unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(
        error.message(),
        "transport: the connection couldn't be made"
    );
}

#[tokio::test]
async fn a_block_of_another_provider_fails_before_anything_is_sent() {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::provider(&server);
    let input = harness::prompt();
    let foreign = [ContentBlock::Opaque {
        provider: ProviderKind::Openai,
        payload: json!({ "type": "reasoning", "encrypted_content": "abc" }),
    }];
    let messages = [
        Message::User {
            tool_results: Vec::new(),
            input: &input,
        },
        Message::Assistant(&foreign),
        Message::User {
            tool_results: Vec::new(),
            input: &input,
        },
    ];

    let error = provider
        .complete(harness::request(&messages, &[]))
        .await
        .unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Fatal);
    assert_eq!(
        error.message(),
        "the conversation holds a block of provider openai, which the Anthropic adapter can't \
         send"
    );
    assert!(harness::received(&server).await.is_empty());
}

#[tokio::test]
async fn a_200_that_isn_t_a_message_is_malformed() {
    let error =
        classified(ResponseTemplate::new(200).set_body_string("<html>not json</html>")).await;
    assert_eq!(error.kind, ProviderErrorKind::Malformed);
    assert!(
        error
            .message()
            .starts_with("the response couldn't be read as a message: "),
        "{}",
        error.message()
    );
    assert!(
        !error.message().contains("not json"),
        "the body isn't echoed"
    );

    let mut without_usage = harness::text("Hello");
    without_usage.as_object_mut().unwrap().remove("usage");
    let error = classified(harness::ok(without_usage)).await;
    assert_eq!(error.kind, ProviderErrorKind::Malformed);
    assert!(error.message().contains("usage"), "{}", error.message());

    let mut without_stop_reason = harness::text("Hello");
    without_stop_reason["stop_reason"] = json!(null);
    let error = classified(harness::ok(without_stop_reason)).await;
    assert_eq!(error.kind, ProviderErrorKind::Malformed);
    assert_eq!(error.message(), "the response has no stop_reason");

    let mut wrong_type = harness::text("Hello");
    wrong_type["usage"]["input_tokens"] = json!("BODY-\"SCALAR\"-LEAK-0123456789");
    let error = classified(harness::ok(wrong_type)).await;
    assert_eq!(error.kind, ProviderErrorKind::Malformed);
    assert!(
        !error.message().contains("LEAK") && !error.message().contains("SCALAR"),
        "the value serde refused is a piece of the body: {}",
        error.message()
    );
    assert!(
        error.message().contains("expected u64 at line"),
        "serde's kind and position stay: {}",
        error.message()
    );
}

#[tokio::test]
async fn a_thinking_block_without_a_signature_is_malformed() {
    let error = classified(harness::ok(harness::message(
        json!([{ "type": "thinking", "thinking": "unsigned" }]),
        "end_turn",
    )))
    .await;

    assert_eq!(error.kind, ProviderErrorKind::Malformed);
    assert_eq!(error.message(), "a thinking block has no signature");
}

#[tokio::test]
async fn a_tool_use_with_a_refused_id_or_name_or_a_repeated_id_is_malformed() {
    let error = classified(harness::ok(harness::message(
        json!([{ "type": "tool_use", "id": "", "name": "read_file", "input": {} }]),
        "tool_use",
    )))
    .await;
    assert_eq!(error.kind, ProviderErrorKind::Malformed);
    assert_eq!(
        error.message(),
        "a tool_use block has an id the domain refuses"
    );

    let error = classified(harness::ok(harness::message(
        json!([{ "type": "tool_use", "id": "toolu_01", "name": "read file! LEAK-0123456789", "input": {} }]),
        "tool_use",
    )))
    .await;
    assert_eq!(error.kind, ProviderErrorKind::Malformed);
    assert_eq!(
        error.message(),
        "a tool_use block has a name the domain refuses",
        "the name is the server's text of any length, so it isn't echoed"
    );

    let error = classified(harness::ok(harness::message(
        json!([
            { "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": { "path": "a" } },
            { "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": { "path": "b" } }
        ]),
        "tool_use",
    )))
    .await;
    assert_eq!(error.kind, ProviderErrorKind::Malformed);
    assert!(error.message().contains("toolu_01"), "{}", error.message());
}
