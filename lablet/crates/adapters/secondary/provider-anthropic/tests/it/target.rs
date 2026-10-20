//! What `Target::new` refuses, and what it makes of a base URL.

use lablet_model::{Endpoint, ModelRef, ProviderApi};
use lablet_provider_anthropic::{Key, Target, TargetError};

use crate::harness::{KEY, MODEL};

fn target(base_url: Option<&str>) -> Result<Target, TargetError> {
    Target::new(MODEL, Key(KEY), base_url, true)
}

#[tokio::test]
async fn a_base_url_with_a_query_a_fragment_no_host_or_another_scheme_is_refused() {
    for base_url in [
        "https://gw.example/anthropic?token=abc",
        "https://gw.example/anthropic#v1",
        "https://",
        "mailto:ops@gw.example",
        "ftp://gw.example/anthropic",
        "file:///anthropic",
        "not a url",
        "",
    ] {
        let error = target(Some(base_url)).unwrap_err();
        assert_eq!(
            error,
            TargetError::BaseUrl {
                reason: "a base URL names a scheme, a host, an optional port and a path, as \
                         https://gateway.example/anthropic does, and has no query or fragment"
                    .to_owned()
            },
            "{base_url}"
        );
        assert!(!error.to_string().contains("token"), "{base_url}");
    }
}

#[tokio::test]
async fn a_key_a_header_can_t_carry_is_refused_without_being_shown() {
    for key in [
        "sk-ant-with-a-newline\n",
        "sk-ant-with-a-nul\0",
        "sk-ant-with-a-cr\r",
        "sk-ant-with-a-tab\t",
        "sk-ant-with-a-non-ascii-byte-\u{e9}",
        "sk-ant-with-a-no-break-space\u{a0}",
    ] {
        let error = Target::new(MODEL, Key(key), None, true).unwrap_err();
        assert_eq!(
            error,
            TargetError::Key {
                reason: "the key holds a character a header can't carry".to_owned()
            }
        );
        assert!(!format!("{error}").contains("sk-ant"));
    }
}

#[tokio::test]
async fn the_messages_url_and_the_endpoint_follow_the_base_url_and_the_default_is_the_api() {
    let target = target(None).unwrap();
    assert_eq!(target.url(), "https://api.anthropic.com/v1/messages");
    assert_eq!(
        target.endpoint(),
        &Endpoint {
            host: "api.anthropic.com".to_owned(),
            port: 443,
        }
    );

    let target = target_at("http://gw.example:8080/base/");
    assert_eq!(target.url(), "http://gw.example:8080/base/v1/messages");
    assert_eq!(
        target.endpoint(),
        &Endpoint {
            host: "gw.example".to_owned(),
            port: 8080,
        }
    );

    let target = target_at("https://gw.example");
    assert_eq!(target.url(), "https://gw.example/v1/messages");
    assert_eq!(target.endpoint().port, 443);
}

fn target_at(base_url: &str) -> Target {
    target(Some(base_url)).unwrap()
}

#[tokio::test]
async fn the_model_is_reached_through_the_messages_api_with_its_reasoning_replayed() {
    let target = target(None).unwrap();
    assert_eq!(
        target.model(),
        &ModelRef {
            api: ProviderApi::Messages,
            name: MODEL.to_owned(),
            replays_reasoning: true,
        }
    );
    assert!(target.cache());
    assert!(!Target::new(MODEL, Key(KEY), None, false).unwrap().cache());
}
