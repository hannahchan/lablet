use serde_json::json;

use super::*;

#[test]
fn a_run_has_no_labels_unless_it_is_given_them() {
    assert_eq!(
        RunLabels::default(),
        RunLabels {
            task: None,
            experiment: None,
            trial: None,
        }
    );
}

#[test]
fn every_label_is_written_and_a_missing_one_is_null() {
    let labels = RunLabels {
        task: Some("fix-failing-test".to_owned()),
        experiment: None,
        trial: Some("3".to_owned()),
    };

    assert_eq!(
        serde_json::to_string(&labels).unwrap(),
        r#"{"task":"fix-failing-test","experiment":null,"trial":"3"}"#
    );
}

#[test]
fn a_label_left_out_or_null_reads_as_none() {
    let read = serde_json::from_value::<RunLabels>(json!({ "task": null, "trial": "3" })).unwrap();

    assert_eq!(
        read,
        RunLabels {
            task: None,
            experiment: None,
            trial: Some("3".to_owned()),
        }
    );
}

#[test]
fn a_misspelt_label_is_an_error_not_a_run_without_one() {
    let error =
        serde_json::from_value::<RunLabels>(json!({ "tsak": "fix-failing-test" })).unwrap_err();

    assert!(
        error.to_string().contains("unknown field `tsak`"),
        "{error}"
    );
}
