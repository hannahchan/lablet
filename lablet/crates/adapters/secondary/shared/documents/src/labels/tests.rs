use lablet_model::RunLabels;
use serde_json::json;

use super::*;

fn partly() -> RunLabels {
    RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: None,
        trial: Some("3".to_owned()),
    }
}

#[test]
fn every_label_is_written_under_its_own_name_and_a_missing_one_is_null() {
    assert_eq!(
        serde_json::to_string(&Labels::from(partly())).unwrap(),
        r#"{"task":"fix-failing-test","experiment":null,"trial":"3"}"#
    );
}

/// O11, the outcome's half: a request that named no label leaves three
/// keys in the document, each `null`.
#[test]
fn a_run_asked_for_under_no_label_writes_each_of_the_three_as_null() {
    assert_eq!(
        serde_json::to_string(&Labels::from(RunLabels::default())).unwrap(),
        r#"{"task":null,"experiment":null,"trial":null}"#
    );
}

#[test]
fn labels_read_back_as_the_labels_they_were_written_from() {
    for labels in [partly(), RunLabels::default()] {
        let written = serde_json::to_string(&Labels::from(labels.clone())).unwrap();

        let read = serde_json::from_str::<Labels>(&written).unwrap();

        assert_eq!(RunLabels::from(read), labels);
    }
}

#[test]
fn a_label_left_out_reads_as_one_the_request_did_not_name() {
    let read = serde_json::from_value::<Labels>(json!({ "task": null, "trial": "3" })).unwrap();

    assert_eq!(
        RunLabels::from(read),
        RunLabels {
            task: None,
            experiment: None,
            trial: Some("3".to_owned()),
        }
    );
}

#[test]
fn a_misspelt_label_is_an_error_not_a_run_without_one() {
    let refused =
        serde_json::from_value::<Labels>(json!({ "tsak": "fix-failing-test" })).unwrap_err();

    assert!(
        refused.to_string().contains("unknown field `tsak`"),
        "{refused}"
    );
}
