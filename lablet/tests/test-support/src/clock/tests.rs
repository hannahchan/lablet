use super::*;

#[tokio::test(start_paused = true)]
async fn on_a_paused_clock_a_sleep_passes_at_once_and_is_measured_as_exactly_what_was_asked() {
    let clock = TokioClock;
    let began = std::time::Instant::now();
    let before = clock.now();

    clock.sleep(Duration::from_secs(250)).await;

    assert_eq!(clock.now() - before, Duration::from_secs(250));
    // A paused sleep returns in microseconds, far inside this margin.
    let waited = began.elapsed();
    assert!(
        waited < Duration::from_secs(1),
        "a sleep of 250 s took {waited:?} in real time"
    );
}

#[tokio::test(start_paused = true)]
async fn on_a_paused_clock_the_time_stands_still_until_something_waits() {
    let clock = TokioClock;
    let before = clock.now();

    std::thread::sleep(Duration::from_millis(5));

    assert_eq!(clock.now(), before);
}

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
