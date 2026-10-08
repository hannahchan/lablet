use super::*;

#[tokio::test(start_paused = true)]
async fn a_run_is_never_asked_to_stop() {
    assert!(!NeverCancelled.is_cancelled());
    assert!(
        tokio::time::timeout(Duration::from_secs(3_600), NeverCancelled.cancelled())
            .await
            .is_err(),
        "an hour went by and the run was asked to stop"
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_is_asked_to_stop_once_its_time_has_passed_and_from_then_on() {
    let cancel = CancelledAfter::new(Duration::from_secs(5));
    let before = tokio::time::Instant::now();

    assert!(!cancel.is_cancelled());
    cancel.cancelled().await;

    assert_eq!(before.elapsed(), Duration::from_secs(5));
    assert!(cancel.is_cancelled());
    let again = tokio::time::Instant::now();
    cancel.cancelled().await;
    assert_eq!(
        again.elapsed(),
        Duration::ZERO,
        "a run asked to stop is told at once"
    );
}
