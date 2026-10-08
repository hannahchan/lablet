use std::time::Duration;

use super::*;

#[test]
fn a_clock_before_the_epoch_starts_the_run_at_the_epoch() {
    let before = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();
    let after = UNIX_EPOCH + Duration::from_nanos(1_790_000_000_000_123_456);

    assert_eq!(at_or_after_epoch(before), UNIX_EPOCH);
    assert_eq!(at_or_after_epoch(UNIX_EPOCH), UNIX_EPOCH);
    assert_eq!(at_or_after_epoch(after), after);
}

#[test]
fn the_wall_clock_reads_the_system_s_time() {
    let before = SystemTime::now();
    let read = TokioClock.wall();
    let after = SystemTime::now();

    assert!(
        (before..=after).contains(&read),
        "{before:?} <= {read:?} <= {after:?}"
    );
}
