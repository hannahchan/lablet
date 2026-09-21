use lablet_model::{StopReason, TranscriptError};

use super::*;

/// The loop can't produce a sequence the transcript refuses, so this text is
/// only ever read by whoever is looking at a defect. It says three things:
/// that lablet is at fault rather than the provider, which of its own rules
/// it broke, and that the run still came back with an outcome.
#[test]
fn a_refused_sequence_is_reported_as_lablets_own_defect() {
    let refusal = TranscriptError::NothingFromTheUser { turn: 2 };

    let stopped = Stopped::defect(&refusal);

    assert_eq!(stopped.reason, StopReason::ProviderError);
    assert_eq!(
        stopped.error.as_deref(),
        Some(
            "lablet defect: the transcript refused the loop's own sequence: turn 2 has no input \
             and no tool results come before it, so nothing from the user would precede its \
             response"
        )
    );
}

/// Whichever rule was broken, the reading is the same: lablet's fault, and
/// the run still ends with a stop reason rather than a panic.
#[test]
fn every_refusal_reads_as_a_defect_and_carries_its_own_words() {
    let refusals = [
        TranscriptError::UnansweredCalls {
            turn: 1,
            calls: vec!["call_0".to_owned()],
        },
        TranscriptError::AlreadyAnswered { turn: 1 },
        TranscriptError::OutcomesDontAnswerCalls {
            calls: vec!["call_0".to_owned()],
            outcomes: Vec::new(),
        },
    ];

    for refusal in refusals {
        let stopped = Stopped::defect(&refusal);

        assert_eq!(stopped.reason, StopReason::ProviderError, "{refusal}");
        let error = stopped.error.expect("a defect always says what happened");
        assert!(error.starts_with("lablet defect: "), "{error}");
        assert!(error.ends_with(&refusal.to_string()), "{error}");
    }
}
