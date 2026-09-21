use serde_json::json;

use super::*;
use crate::{ContentBlock, FinishReason, ProviderResponse, TokenCounts, Usage, UserContent};

fn ms(count: u64) -> std::time::Duration {
    std::time::Duration::from_millis(count)
}

/// A run of one turn, built the way the loop builds one.
fn one_turn() -> Transcript {
    let mut transcript = Transcript::new("Be brief.".to_owned());
    let response = ProviderResponse::new(
        vec![ContentBlock::Text("Done.".to_owned())],
        Usage::from_inclusive(TokenCounts {
            input: 10,
            output: 2,
            reasoning: 0,
            cache_read: 0,
            cache_write: 0,
        }),
        FinishReason::EndTurn,
        None,
        None,
    )
    .expect("distinct call ids");
    transcript
        .record(
            &mut vec![UserContent::Text("Hi.".to_owned())],
            response,
            ms(0),
            ms(5),
            1,
        )
        .expect("the first turn takes the prompt");
    transcript
}

#[test]
fn a_document_is_the_transcript_under_a_version() {
    let transcript = one_turn();

    let value = serde_json::to_value(TranscriptDocument::of(&transcript)).unwrap();

    assert_eq!(value["schema_version"], json!(1));
    assert_eq!(value["system"], json!("Be brief."));
    assert_eq!(
        value["turns"],
        serde_json::to_value(transcript.turns()).unwrap()
    );
    assert_eq!(
        value.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["schema_version", "system", "turns"],
        "the version comes first, so a reader knows the form before the content"
    );
}

/// The document borrows, so publishing one copies nothing and the transcript
/// is still the run's afterwards.
#[test]
fn a_document_borrows_the_transcript_it_publishes() {
    let transcript = one_turn();

    let document = TranscriptDocument::of(&transcript);

    assert_eq!(document.system, transcript.system());
    assert!(std::ptr::eq(document.turns, transcript.turns()));
}

#[test]
fn a_run_with_no_turns_publishes_its_system_prompt_alone() {
    let transcript = Transcript::new("Be brief.".to_owned());

    let value = serde_json::to_value(TranscriptDocument::of(&transcript)).unwrap();

    assert_eq!(
        value,
        json!({ "schema_version": 1, "system": "Be brief.", "turns": [] })
    );
}

/// The constant is what a reader tells forms apart by, so a change to it is
/// a change to the contract and this test is where that is noticed.
#[test]
fn the_published_version_is_one() {
    assert_eq!(TRANSCRIPT_SCHEMA_VERSION, 1);
}
