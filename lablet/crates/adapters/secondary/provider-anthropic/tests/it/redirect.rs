//! A 3xx is a response the adapter reads, since the client follows no
//! redirect: the key goes to the host the base URL names and nowhere else.

use lablet_model::ProviderErrorKind;
use lablet_test_support::refused_address;
use wiremock::{MockServer, ResponseTemplate};

use crate::harness;

#[tokio::test]
async fn a_308_is_fatal_with_the_fix_and_is_followed_nowhere() {
    let server = MockServer::start().await;
    harness::answer(
        &server,
        ResponseTemplate::new(308).insert_header(
            "location",
            format!("http://{}/v1/messages", refused_address()).as_str(),
        ),
    )
    .await;
    let provider = harness::provider(&server);

    let error = harness::complete(&provider).await.unwrap_err();

    assert_eq!(error.kind, ProviderErrorKind::Fatal);
    assert_eq!(
        error.message(),
        "HTTP 308 from the endpoint: the base URL redirects; set model.base_url to the address \
         it redirects to"
    );
    assert_eq!(
        harness::received(&server).await.len(),
        1,
        "one request, and none to the address it redirects to, which would have been refused \
         and read as a transport failure"
    );
}
