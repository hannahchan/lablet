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
