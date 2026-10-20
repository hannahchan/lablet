//! A provider that fails, as a run built from a config reports it.

use lablet::{RunId, StopReason};
use lablet_test_support::conformance::otlp::Status;
use serde_json::json;

use crate::harness::{KEY, Lab, Traced, request, with_key};
use crate::key;

const RUN: &str = "01K5F3Z8Q4X9T2M7B6W1R0VNEC";

/// A rejected key, and a response the run never asks for.
const REJECTS_THE_KEY: &str = "
- error: { kind: auth, message: 401 invalid x-api-key }
- response: { content: [{ text: Never said. }], finish: end_turn }
";

#[tokio::test]
async fn a_rejected_key_ends_the_run_at_once_and_the_chat_span_says_auth() {
    let scratch = Lab::new("auth");
    let mut lablet = scratch
        .build(scratch.config(REJECTS_THE_KEY, json!({})))
        .await
        .unwrap();

    let finished = lablet
        .run(request().run_id(RunId::new(RUN).unwrap()).unwrap())
        .await;
    lablet.shutdown().await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::ProviderError);
    assert_eq!(outcome.error(), Some("401 invalid x-api-key"));
    assert_eq!(
        lablet::ErrorClass::of_run(outcome),
        Some(lablet::ErrorClass::Provider)
    );
    assert_eq!(outcome.turns, 0);
    assert_eq!(finished.summary.provider.retries, 0);

    let exported = scratch.exported();
    let traced = Traced::of(&exported, RUN);
    let chats = traced.chats();
    assert_eq!(chats.len(), 1, "a rejected key is never tried again");
    assert_eq!(chats[0].attributes[key::ERROR_TYPE], json!("auth"));
    assert_eq!(chats[0].attributes[key::LABLET_ATTEMPT], json!(1));
    assert!(
        matches!(chats[0].status, Status::Error(_)),
        "{:?}",
        chats[0].status
    );
    assert_eq!(chats[0].events.len(), 1);
    assert_eq!(
        chats[0].events[0].attributes[key::LABLET_RETRY_WILL_RETRY],
        json!(false)
    );

    let exceptions = exported.records_of("gen_ai.client.operation.exception");
    assert_eq!(exceptions.len(), 1);
    assert_eq!(exceptions[0].attributes[key::EXCEPTION_TYPE], json!("auth"));
    assert_eq!(exceptions[0].span_id, chats[0].span_id);

    assert_eq!(
        traced.root().attributes[key::ERROR_TYPE],
        json!("provider_error")
    );
    let wide = traced.wide();
    assert_eq!(
        wide.attributes[key::LABLET_RUN_STOP_REASON],
        json!("provider_error")
    );
    assert_eq!(wide.attributes[key::ERROR_TYPE], json!("provider_error"));
    assert_eq!(
        wide.attributes[key::LABLET_RUN_ERROR],
        json!("401 invalid x-api-key")
    );
    assert_eq!(wide.attributes[key::LABLET_PROVIDER_RETRIES], json!(0));
}

/// A run with no tools has no executor, and its secrets are held all the
/// same, for the loop to cut out of a provider's words (P11): a 401 that
/// echoes the key it refused reaches the outcome and the telemetry without
/// it.
#[tokio::test]
async fn a_run_with_no_tools_whose_provider_error_carries_a_registered_secret_holds_it() {
    let scratch = Lab::new("echoed-key");
    let echoes_the_key = format!(
        "- error: {{ kind: auth, message: 'HTTP 401 authentication_error: invalid x-api-key {KEY}' }}"
    );
    let config = with_key(scratch.config(&echoes_the_key, json!({})));
    assert!(
        config.tools.builtin.enabled.is_empty(),
        "no tools, so no executor"
    );
    let mut lablet = scratch.build(config).await.unwrap();

    let finished = lablet.run(request()).await;
    lablet.shutdown().await;

    let outcome = &finished.summary.outcome;
    assert_eq!(outcome.stop_reason(), StopReason::ProviderError);
    assert_eq!(
        outcome.error(),
        Some("HTTP 401 authentication_error: invalid x-api-key [secret withheld]")
    );
    let telemetry = scratch.exported_text();
    assert!(!telemetry.contains(KEY), "the key reached the telemetry");
    assert!(telemetry.contains("[secret withheld]"), "{telemetry}");
}
