//! The bound on what's read of a response's body: a body up to it is read
//! whole, and one past it is cut there, never parsed and never quoted.

use std::time::Duration;

use lablet_model::{ContentBlock, ProviderErrorKind};
use wiremock::{MockServer, ResponseTemplate};

use crate::harness;

/// The bound, one MiB, as the specification states it.
const BOUND: usize = 1024 * 1024;

/// A 200 whose body is a message of one text block, `bytes` long whole.
fn message_of(bytes: usize) -> ResponseTemplate {
    let around = serde_json::to_vec(&harness::text("")).unwrap().len();
    let body = serde_json::to_vec(&harness::text(&"a".repeat(bytes - around))).unwrap();
    assert_eq!(body.len(), bytes);
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

/// What one attempt answered with `template` comes to, from a provider
/// that captures content, which would quote a body it read.
async fn answered(
    template: ResponseTemplate,
) -> Result<lablet_model::ProviderResponse, lablet_run::ProviderError> {
    let server = MockServer::start().await;
    harness::answer(&server, template).await;
    let provider = harness::with(
        harness::target(&server, false),
        opentelemetry::trace::noop::NoopTextMapPropagator::new(),
        true,
    );
    harness::complete(&provider).await
}

#[tokio::test]
async fn a_200_as_long_as_the_bound_is_read_whole() {
    let response = answered(message_of(BOUND)).await.unwrap();

    let [ContentBlock::Text(text)] = response.content() else {
        panic!("one text block: {:?}", response.content());
    };
    assert_eq!(
        text.len(),
        BOUND - serde_json::to_vec(&harness::text("")).unwrap().len()
    );
}

/// A 200 past the bound held an answer the model finished and was billed
/// for, and the same request would bring one past the bound again, so what
/// was read of it is dropped and the run stops rather than ask again.
#[tokio::test]
async fn a_200_longer_than_the_bound_drops_what_was_read_and_is_fatal() {
    let error = answered(message_of(BOUND + 1)).await.unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Fatal);
    assert_eq!(
        error.message(),
        "the response is longer than 1,048,576 bytes; the answer was complete, so the run stops \
         rather than ask for it again"
    );
}

/// A failure's body past the bound can't be read as the envelope, so it's
/// classified by its status and its headers alone, and nothing of it is
/// quoted, though the run captures content; the request id the headers
/// carry is shown, as it is beside an envelope that was read.
#[tokio::test]
async fn a_failure_longer_than_the_bound_is_classified_by_status_and_never_quoted() {
    let long = format!("<html>{}</html>", "x".repeat(BOUND));
    let error = answered(
        ResponseTemplate::new(502)
            .set_body_string(long.clone())
            .insert_header("retry-after", "3")
            .insert_header("request-id", "req_011CSHoEeqs5C35K2UUqR7Fy"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(error.retry_after, Some(Duration::from_secs(3)));
    assert_eq!(
        error.message(),
        "HTTP 502 with a body longer than 1,048,576 bytes, which was cut there and isn't quoted \
         (request-id req_011CSHoEeqs5C35K2UUqR7Fy)"
    );

    let error = answered(harness::envelope(
        400,
        "invalid_request_error",
        &format!("prompt is too long: {long}"),
    ))
    .await
    .unwrap_err();
    assert_eq!(
        error.kind,
        ProviderErrorKind::Fatal,
        "an envelope past the bound isn't read"
    );
    assert_eq!(
        error.message(),
        "HTTP 400 with a body longer than 1,048,576 bytes, which was cut there and isn't quoted"
    );

    let error = answered(
        ResponseTemplate::new(529)
            .set_body_string(long.clone())
            .insert_header("x-should-retry", "false"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Fatal);
    assert_eq!(
        error.message(),
        "HTTP 529 with a body longer than 1,048,576 bytes, which was cut there and isn't quoted; \
         the server said not to retry"
    );

    // A redirect keeps its fix, and says its body was cut as well.
    let error = answered(ResponseTemplate::new(308).set_body_string(long))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Fatal);
    assert_eq!(
        error.message(),
        "HTTP 308 from the endpoint: the base URL redirects; set model.base_url to the address \
         it redirects to; it came with a body longer than 1,048,576 bytes, which was cut there \
         and isn't quoted"
    );
}
