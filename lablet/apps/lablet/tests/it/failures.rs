//! A provider that fails, as a run built from a config reports it.

use lablet::{RunId, StopReason};
use lablet_conformance::otlp::Status;
use serde_json::json;

use crate::harness::{Lab, Traced, request};
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
    let mut lablet = lablet::build(scratch.config(REJECTS_THE_KEY, json!({})))
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
