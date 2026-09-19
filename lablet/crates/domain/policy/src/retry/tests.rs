use super::*;

const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// 100 ms doubling to a cap of 10 s.
fn doubling(max_attempts: u32) -> RetryPolicy {
    RetryPolicy::new(max_attempts, ms(100), ms(10_000), 2.0).unwrap()
}

// Exhaustion

#[test]
fn the_failure_of_the_last_allowed_attempt_exhausts_the_call() {
    let policy = doubling(3);

    assert_eq!(policy.delay(1), Some(ms(100)));
    assert_eq!(policy.delay(2), Some(ms(200)));
    assert_eq!(policy.delay(3), None);
    assert_eq!(policy.delay(4), None);
}

#[test]
fn a_policy_of_one_attempt_never_retries() {
    assert_eq!(doubling(1).delay(1), None);
}

#[test]
fn attempt_zero_is_read_as_the_first_attempt() {
    assert_eq!(doubling(3).delay(0), Some(ms(100)));
    assert_eq!(doubling(1).delay(0), None);
}

// Growth and the cap

#[test]
fn the_wait_grows_by_the_factor_after_each_failure() {
    let policy = doubling(10);
    let waits: Vec<_> = (1..=7).map(|attempt| policy.delay(attempt)).collect();

    assert_eq!(
        waits,
        [100, 200, 400, 800, 1_600, 3_200, 6_400].map(|millis| Some(ms(millis)))
    );
}

#[test]
fn the_wait_never_exceeds_the_cap() {
    let policy = doubling(10);

    assert_eq!(policy.delay(7), Some(ms(6_400)));
    assert_eq!(policy.delay(8), Some(ms(10_000)));
    assert_eq!(policy.delay(9), Some(ms(10_000)));
}

#[test]
fn a_wait_that_lands_on_the_cap_is_the_cap() {
    let policy = RetryPolicy::new(10, ms(1_000), ms(4_000), 2.0).unwrap();

    assert_eq!(policy.delay(3), Some(ms(4_000)));
}

#[test]
fn a_factor_that_is_not_a_whole_number_grows_the_wait_too() {
    let policy = RetryPolicy::new(10, ms(100), ms(10_000), 1.5).unwrap();

    assert_eq!(policy.delay(1), Some(ms(100)));
    assert_eq!(policy.delay(2), Some(ms(150)));
    assert_eq!(policy.delay(3), Some(ms(225)));
}

#[test]
fn a_factor_of_one_waits_the_base_every_time() {
    let policy = RetryPolicy::new(u32::MAX, ms(100), ms(10_000), 1.0).unwrap();

    assert_eq!(policy.delay(1), Some(ms(100)));
    assert_eq!(policy.delay(50), Some(ms(100)));
    assert_eq!(policy.delay(u32::MAX - 1), Some(ms(100)));
}

#[test]
fn the_same_attempt_always_gives_the_same_wait() {
    let policy = doubling(10);

    assert_eq!(policy.delay(4), policy.delay(4));
}

// Extremes

#[test]
fn an_attempt_number_whose_wait_overflows_gives_the_cap() {
    let policy = doubling(u32::MAX);

    for attempt in [64, 1_100, 1 << 31, u32::MAX - 1] {
        assert_eq!(policy.delay(attempt), Some(ms(10_000)), "{attempt}");
    }
    assert_eq!(policy.delay(u32::MAX), None);
}

#[test]
fn the_largest_factor_gives_the_base_and_then_the_cap() {
    let policy = RetryPolicy::new(u32::MAX, ms(100), ms(10_000), f64::MAX).unwrap();

    assert_eq!(policy.delay(1), Some(ms(100)));
    assert_eq!(policy.delay(2), Some(ms(10_000)));
    assert_eq!(policy.delay(u32::MAX - 1), Some(ms(10_000)));
}

#[test]
fn a_base_of_zero_waits_zero_however_far_the_scale_overflows() {
    let policy = RetryPolicy::new(u32::MAX, Duration::ZERO, ms(10_000), 2.0).unwrap();

    assert_eq!(policy.delay(1), Some(Duration::ZERO));
    assert_eq!(policy.delay(5_000), Some(Duration::ZERO));
    assert_eq!(policy.delay(u32::MAX - 1), Some(Duration::ZERO));
}

#[test]
fn the_longest_durations_are_a_valid_policy() {
    let policy = RetryPolicy::new(u32::MAX, Duration::MAX, Duration::MAX, 2.0).unwrap();

    assert_eq!(policy.delay(1), Some(Duration::MAX));
    assert_eq!(policy.delay(2), Some(Duration::MAX));
}

// Validation

#[test]
fn a_policy_of_zero_attempts_is_refused() {
    assert_eq!(
        RetryPolicy::new(0, ms(100), ms(10_000), 2.0),
        Err(RetryPolicyError::NoAttempts)
    );
}

#[test]
fn a_base_longer_than_the_cap_is_refused_and_one_equal_to_it_is_not() {
    assert_eq!(
        RetryPolicy::new(3, ms(10_001), ms(10_000), 2.0),
        Err(RetryPolicyError::BaseAboveMax {
            base: ms(10_001),
            max: ms(10_000),
        })
    );
    assert!(RetryPolicy::new(3, ms(10_000), ms(10_000), 2.0).is_ok());
    assert!(RetryPolicy::new(3, ms(9_999), ms(10_000), 2.0).is_ok());
}

#[test]
fn a_factor_below_one_is_refused_and_one_itself_is_not() {
    for factor in [0.999, 0.5, 0.0, -2.0] {
        assert_eq!(
            RetryPolicy::new(3, ms(100), ms(10_000), factor),
            Err(RetryPolicyError::Factor(factor)),
            "{factor}"
        );
    }
    assert!(RetryPolicy::new(3, ms(100), ms(10_000), 1.0).is_ok());
    assert!(RetryPolicy::new(3, ms(100), ms(10_000), 1.001).is_ok());
}

#[test]
fn a_factor_that_is_not_a_finite_number_is_refused() {
    for factor in [f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            RetryPolicy::new(3, ms(100), ms(10_000), factor),
            Err(RetryPolicyError::Factor(factor)),
            "{factor}"
        );
    }
    assert!(matches!(
        RetryPolicy::new(3, ms(100), ms(10_000), f64::NAN),
        Err(RetryPolicyError::Factor(factor)) if factor.is_nan()
    ));
}

#[test]
fn the_first_broken_rule_in_argument_order_is_the_one_reported() {
    assert_eq!(
        RetryPolicy::new(0, ms(2), ms(1), f64::NAN),
        Err(RetryPolicyError::NoAttempts)
    );
    assert_eq!(
        RetryPolicy::new(3, ms(2), ms(1), f64::NAN),
        Err(RetryPolicyError::BaseAboveMax {
            base: ms(2),
            max: ms(1),
        })
    );
}

#[test]
fn each_error_says_what_was_wrong() {
    assert_eq!(
        RetryPolicyError::NoAttempts.to_string(),
        "max attempts is 0; 1 means a failed call isn't retried"
    );
    assert_eq!(
        RetryPolicyError::Factor(0.5).to_string(),
        "backoff factor 0.5 isn't a finite number of at least 1"
    );
    assert_eq!(
        RetryPolicyError::BaseAboveMax {
            base: ms(2_000),
            max: ms(500),
        }
        .to_string(),
        "backoff base 2s is longer than the backoff cap 500ms"
    );
}
