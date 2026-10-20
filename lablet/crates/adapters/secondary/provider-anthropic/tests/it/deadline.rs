//! The per-call deadline, enforced in real time as the request's timeout.

use std::time::Duration;

use lablet_model::{Message, ProviderErrorKind};
use lablet_run::ModelProvider as _;
use wiremock::MockServer;

use crate::harness;

#[tokio::test]
async fn a_deadline_of_zero_fails_before_anything_is_sent() {
    let server = MockServer::start().await;
    harness::answer(&server, harness::ok(harness::text("Hello"))).await;
    let provider = harness::provider(&server);
    let input = harness::prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];
    let mut request = harness::request(&messages, &[]);
    request.deadline = Duration::ZERO;

    let error = provider.complete(request).await.unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(error.message(), "the attempt reached its deadline of 0 ms");
    assert!(harness::received(&server).await.is_empty());
}

#[tokio::test]
async fn a_response_that_comes_after_the_deadline_is_retryable_and_names_the_deadline() {
    let server = MockServer::start().await;
    harness::answer(
        &server,
        harness::ok(harness::text("Hello")).set_delay(Duration::from_secs(5)),
    )
    .await;
    let provider = harness::provider(&server);
    let input = harness::prompt();
    let messages = [Message::User {
        tool_results: Vec::new(),
        input: &input,
    }];
    let mut request = harness::request(&messages, &[]);
    request.deadline = Duration::from_millis(200);

    let error = provider.complete(request).await.unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(
        error.message(),
        "the attempt reached its deadline of 200 ms"
    );
    assert_eq!(error.retry_after, None);
}
