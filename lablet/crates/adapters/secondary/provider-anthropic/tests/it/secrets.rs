//! P11, the adapter's half: a message holds the provider's words, which the
//! loop cuts, and never the URL's user information or a transport error's
//! text; a gateway's credentials go in a header of their own.

use std::sync::Arc;

use lablet_model::{ProviderErrorKind, Secrets};
use lablet_provider_anthropic::{Key, Target};
use lablet_run::ERROR_MESSAGE_MAX_BYTES;
use lablet_test_support::refused_address;
use opentelemetry::trace::noop::NoopTextMapPropagator;
use wiremock::matchers::{basic_auth, method, path};
use wiremock::{Mock, MockServer};

use crate::harness::{self, KEY, MODEL};

#[tokio::test]
async fn a_401_whose_body_repeats_the_key_reaches_the_outcome_with_the_key_cut() {
    let server = MockServer::start().await;
    let echo = format!("invalid x-api-key: {KEY}");
    harness::answer(
        &server,
        harness::envelope(401, "authentication_error", &echo),
    )
    .await;

    let provider = harness::provider(&server);
    let error = harness::complete(&provider).await.unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Auth);
    assert!(
        error.message().contains(KEY),
        "the adapter hands the provider's words over whole; the loop cuts them"
    );

    let provider = harness::provider(&server);
    let finished = harness::run(
        harness::looped(provider, Vec::new(), 0).secrets(Arc::new(Secrets::new([KEY.to_owned()]))),
    )
    .await;

    let shown = finished.summary.outcome.error().unwrap();
    assert!(!shown.contains(KEY), "{shown}");
    assert_eq!(
        shown,
        format!(
            "HTTP 401 authentication_error: invalid x-api-key: {} (request-id \
             req_011CSHoEeqs5C35K2UUqR7Fy)",
            Secrets::MARKER
        )
    );
    assert!(shown.len() <= ERROR_MESSAGE_MAX_BYTES);
}

#[tokio::test]
async fn a_transport_error_through_a_base_url_with_user_information_names_neither() {
    let user = "gateway-user";
    let password = "gateway-pass-0123456789abcdef";
    let target = Target::new(
        MODEL,
        Key(KEY),
        Some(&format!(
            "http://{user}:{password}@{}/anthropic",
            refused_address()
        )),
        false,
    )
    .unwrap();
    let provider = harness::with(target, NoopTextMapPropagator::new(), false);

    let error = harness::complete(&provider).await.unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Retryable);
    assert_eq!(
        error.message(),
        "transport: the connection couldn't be made"
    );
    for secret in [user, password, KEY, "127.0.0.1", "anthropic"] {
        assert!(!error.message().contains(secret), "{secret}");
    }
}

#[tokio::test]
async fn user_information_on_the_base_url_is_sent_as_basic_authorization_and_not_in_the_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/anthropic/v1/messages"))
        .and(basic_auth("gw-user", "p@ss:w/rd"))
        .respond_with(harness::ok(harness::text("Hello")))
        .mount(&server)
        .await;
    let base_url = format!(
        "http://gw-user:p%40ss%3Aw%2Frd@127.0.0.1:{}/anthropic/",
        server.address().port()
    );
    let target = Target::new(MODEL, Key(KEY), Some(&base_url), false).unwrap();
    assert_eq!(
        target.url(),
        format!(
            "http://127.0.0.1:{}/anthropic/v1/messages",
            server.address().port()
        ),
        "the URL the client holds carries no user information"
    );
    let provider = harness::with(target, NoopTextMapPropagator::new(), false);

    harness::complete(&provider).await.unwrap();

    let received = harness::received(&server).await;
    assert_eq!(
        received.len(),
        1,
        "the matcher took the decoded credentials"
    );
    assert_eq!(received[0].url.username(), "");
    assert_eq!(received[0].url.password(), None);
    assert_eq!(harness::header(&received[0], "x-api-key"), Some(KEY));
}

#[tokio::test]
async fn no_debug_form_shows_the_key() {
    let server = MockServer::start().await;
    let target = Target::new(
        MODEL,
        Key(KEY),
        Some(&format!(
            "http://gw-user:gw-pass-0123456789@{}",
            server.address()
        )),
        true,
    )
    .unwrap();
    assert_eq!(format!("{:?}", Key(KEY)), "[secret]");

    let shown = format!("{target:?}");
    assert!(!shown.contains(KEY), "{shown}");
    assert!(!shown.contains("gw-pass"), "{shown}");
    assert_eq!(
        shown.matches("Sensitive").count(),
        2,
        "the key and the credentials: {shown}"
    );

    let provider = harness::with(target, NoopTextMapPropagator::new(), false);
    let shown = format!("{provider:?}");
    assert!(!shown.contains(KEY), "{shown}");
    assert!(!shown.contains("gw-pass"), "{shown}");
}
