use lablet_model::{ProviderErrorKind, RunId};

use super::*;

const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

const fn secs(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

/// A salt whose share of the jitter is nothing.
const NONE: u64 = 0;
/// A salt whose share of the jitter is a half.
const HALF: u64 = 1 << 63;
/// The salt whose share of the jitter is the largest there is.
const MOST: u64 = u64::MAX;

/// 100 ms doubling to a cap of 10 s, a hint of up to 60 s, and no jitter.
const fn settings(max_retries: u32) -> RetrySettings {
    RetrySettings {
        max_retries,
        base: ms(100),
        max: ms(10_000),
        factor: 2.0,
        hint_max: secs(60),
        jitter: 0.0,
    }
}

fn doubling(max_retries: u32) -> RetryPolicy {
    RetryPolicy::new(settings(max_retries)).unwrap()
}

/// The same, with waits spread by up to a quarter.
fn spread(max_retries: u32) -> RetryPolicy {
    RetryPolicy::new(RetrySettings {
        jitter: 0.25,
        ..settings(max_retries)
    })
    .unwrap()
}

/// What follows a retryable failure of `attempt` that came with no hint.
///
/// The salt is the one that would add the most, so a test of a policy without
/// jitter also holds that nothing is added.
fn after(policy: &RetryPolicy, attempt: u32) -> Option<Duration> {
    policy.next(attempt, ProviderErrorKind::Retryable, None, MOST)
}

/// What follows a retryable first failure that came with `hint`.
fn hinted(policy: &RetryPolicy, hint: Duration, salt: u64) -> Option<Duration> {
    policy.next(1, ProviderErrorKind::Retryable, Some(hint), salt)
}

// Exhaustion

#[test]
fn the_failure_of_the_attempt_after_the_last_retry_exhausts_the_call() {
    let policy = doubling(2);

    assert_eq!(after(&policy, 1), Some(ms(100)));
    assert_eq!(after(&policy, 2), Some(ms(200)));
    assert_eq!(after(&policy, 3), None);
    assert_eq!(after(&policy, 4), None);
}

#[test]
fn three_retries_allow_four_attempts() {
    let policy = doubling(3);

    assert_eq!(after(&policy, 3), Some(ms(400)));
    assert_eq!(after(&policy, 4), None);
}

#[test]
fn a_policy_of_no_retries_is_valid_and_never_retries() {
    assert_eq!(after(&doubling(0), 1), None);
    assert_eq!(after(&doubling(0), 2), None);
}

#[test]
fn attempt_zero_is_read_as_the_first_attempt() {
    assert_eq!(after(&doubling(2), 0), Some(ms(100)));
    assert_eq!(after(&doubling(1), 0), Some(ms(100)));
    assert_eq!(after(&doubling(0), 0), None);
}

// Growth and the cap

#[test]
fn the_wait_grows_by_the_factor_after_each_failure() {
    let policy = doubling(10);
    let waits: Vec<_> = (1..=7).map(|attempt| after(&policy, attempt)).collect();

    assert_eq!(
        waits,
        [100, 200, 400, 800, 1_600, 3_200, 6_400].map(|millis| Some(ms(millis)))
    );
}

#[test]
fn the_backoff_never_exceeds_the_cap() {
    let policy = doubling(10);

    assert_eq!(after(&policy, 7), Some(ms(6_400)));
    assert_eq!(after(&policy, 8), Some(ms(10_000)));
    assert_eq!(after(&policy, 9), Some(ms(10_000)));
}

#[test]
fn a_backoff_that_lands_on_the_cap_is_the_cap() {
    let policy = RetryPolicy::new(RetrySettings {
        base: ms(1_000),
        max: ms(4_000),
        ..settings(10)
    })
    .unwrap();

    assert_eq!(after(&policy, 3), Some(ms(4_000)));
}

#[test]
fn a_factor_that_is_not_a_whole_number_grows_the_wait_too() {
    let policy = RetryPolicy::new(RetrySettings {
        factor: 1.5,
        ..settings(10)
    })
    .unwrap();

    assert_eq!(after(&policy, 1), Some(ms(100)));
    assert_eq!(after(&policy, 2), Some(ms(150)));
    assert_eq!(after(&policy, 3), Some(ms(225)));
}

#[test]
fn a_factor_of_one_waits_the_base_every_time() {
    let policy = RetryPolicy::new(RetrySettings {
        factor: 1.0,
        ..settings(u32::MAX)
    })
    .unwrap();

    assert_eq!(after(&policy, 1), Some(ms(100)));
    assert_eq!(after(&policy, 50), Some(ms(100)));
    assert_eq!(after(&policy, u32::MAX - 1), Some(ms(100)));
}

// The server's hint

#[test]
fn the_wait_is_the_longer_of_the_hint_and_the_backoff() {
    let policy = doubling(10);

    assert_eq!(hinted(&policy, secs(5), MOST), Some(secs(5)));
    assert_eq!(hinted(&policy, ms(100), MOST), Some(ms(100)));
    assert_eq!(hinted(&policy, ms(40), MOST), Some(ms(100)));
    assert_eq!(hinted(&policy, Duration::ZERO, MOST), Some(ms(100)));
    assert_eq!(
        policy.next(4, ProviderErrorKind::Retryable, Some(ms(500)), MOST),
        Some(ms(800)),
        "the backoff has grown past the hint"
    );
}

#[test]
fn a_hint_longer_than_the_cap_on_hints_ends_the_retries_and_one_equal_to_it_is_waited() {
    let policy = doubling(10);

    assert_eq!(hinted(&policy, secs(60), MOST), Some(secs(60)));
    assert_eq!(
        hinted(&policy, secs(60) + Duration::from_nanos(1), MOST),
        None
    );
    assert_eq!(hinted(&policy, secs(120), MOST), None);
    assert_eq!(hinted(&policy, Duration::MAX, MOST), None);
}

/// The two caps are apart: one bounds what lablet works out, and the other
/// what it will take from a server.
#[test]
fn a_hint_may_be_longer_than_the_cap_on_the_backoff() {
    let policy = doubling(10);

    assert_eq!(hinted(&policy, secs(30), MOST), Some(secs(30)));
}

#[test]
fn a_cap_on_hints_below_the_backoff_refuses_only_a_hint() {
    let policy = RetryPolicy::new(RetrySettings {
        hint_max: ms(50),
        ..settings(10)
    })
    .unwrap();

    assert_eq!(after(&policy, 1), Some(ms(100)));
    assert_eq!(hinted(&policy, ms(50), MOST), Some(ms(100)));
    assert_eq!(hinted(&policy, ms(51), MOST), None);
}

#[test]
fn a_hint_does_not_buy_a_call_a_retry_it_has_not_got() {
    assert_eq!(
        doubling(1).next(2, ProviderErrorKind::Retryable, Some(secs(5)), MOST),
        None
    );
    assert_eq!(
        doubling(10).next(1, ProviderErrorKind::Fatal, Some(secs(5)), MOST),
        None
    );
}

// Jitter

#[test]
fn a_wait_is_raised_by_the_share_of_the_jitter_its_salt_takes() {
    let policy = spread(10);
    let waits = |salt| policy.next(1, ProviderErrorKind::Retryable, None, salt);

    assert_eq!(waits(NONE), Some(ms(100)));
    assert_eq!(waits(1 << 62), Some(Duration::from_micros(106_250)));
    assert_eq!(waits(HALF), Some(Duration::from_micros(112_500)));
    assert_eq!(waits(3 << 62), Some(Duration::from_micros(118_750)));
}

/// A wait long enough for a nanosecond to tell the two apart: the largest
/// share of a quarter of 60 s is 3.5 ns short of 15 s.
#[test]
fn the_largest_share_stays_below_the_whole_jitter() {
    let most = hinted(&spread(10), secs(60), MOST).unwrap();

    assert!(most < secs(75), "{most:?}");
    assert!(most >= Duration::new(74, 999_999_996), "{most:?}");
}

/// A change in the input of the hash behind a salt reaches its upper bits, so
/// those are the ones read.
#[test]
fn the_share_is_read_from_the_upper_half_of_the_salt() {
    let policy = spread(10);

    assert_eq!(
        hinted(&policy, secs(60), u64::from(u32::MAX)),
        Some(secs(60))
    );
    assert_eq!(
        hinted(&policy, secs(60), HALF | u64::from(u32::MAX)),
        hinted(&policy, secs(60), HALF)
    );
    assert_eq!(
        hinted(&policy, secs(60), 1 << 32),
        Some(secs(60) + Duration::from_nanos(3)),
        "the lowest bit of the upper half takes 15 s over 2^32, which is 3.5 ns"
    );
}

#[test]
fn a_wait_that_came_from_a_hint_is_spread_like_any_other() {
    let policy = spread(10);

    assert_eq!(hinted(&policy, secs(5), NONE), Some(secs(5)));
    assert_eq!(hinted(&policy, secs(5), HALF), Some(ms(5_625)));
    assert_eq!(
        hinted(&policy, secs(60), HALF),
        Some(ms(67_500)),
        "the cap is on the hint, so the wait may pass it by its jitter"
    );
}

#[test]
fn a_backoff_at_the_cap_is_the_cap_with_its_jitter() {
    let policy = spread(u32::MAX);

    assert_eq!(
        policy.next(8, ProviderErrorKind::Retryable, None, HALF),
        Some(ms(11_250))
    );
    assert_eq!(
        policy.next(u32::MAX, ProviderErrorKind::Retryable, None, HALF),
        Some(ms(11_250))
    );
}

#[test]
fn a_jitter_of_one_may_double_a_wait_and_never_more() {
    let policy = RetryPolicy::new(RetrySettings {
        jitter: 1.0,
        ..settings(10)
    })
    .unwrap();

    assert_eq!(
        policy.next(1, ProviderErrorKind::Retryable, None, HALF),
        Some(ms(150))
    );
    assert_eq!(
        policy.next(1, ProviderErrorKind::Retryable, None, MOST),
        Some(ms(200)),
        "the largest share of 100 ms is within a nanosecond of all of it"
    );
    let most = hinted(&policy, secs(60), MOST).unwrap();
    assert!(most < secs(120), "{most:?}");
}

#[test]
fn without_jitter_every_salt_gives_the_backoff_exactly() {
    let policy = doubling(10);

    for salt in [NONE, 1, HALF, MOST] {
        assert_eq!(
            policy.next(3, ProviderErrorKind::Retryable, None, salt),
            Some(ms(400)),
            "{salt}"
        );
    }
}

/// The law behind the examples, over the salts a run would really have.
#[test]
fn every_wait_is_from_the_backoff_up_to_a_quarter_more_and_a_larger_share_never_waits_less() {
    let policy = spread(10);
    let run = RunId::new("01K5F3Z8Q4X9T2M7B6W1R0VNEC").unwrap();
    let mut salts: Vec<u64> = (1..=20)
        .flat_map(|turn| (1..=10).map(move |attempt| (turn, attempt)))
        .map(|(turn, attempt)| run.salt(turn, attempt))
        .collect();
    salts.sort_unstable();

    let waits: Vec<Duration> = salts
        .iter()
        .map(|&salt| {
            policy
                .next(4, ProviderErrorKind::Retryable, None, salt)
                .unwrap()
        })
        .collect();

    assert!(
        waits.iter().all(|wait| (ms(800)..ms(1_000)).contains(wait)),
        "{waits:?}"
    );
    assert!(waits.is_sorted(), "{waits:?}");
    let mut distinct = waits.clone();
    distinct.dedup();
    assert_eq!(distinct.len(), waits.len(), "no two of these salts agree");
}

#[test]
fn the_same_salt_always_gives_the_same_wait() {
    let policy = spread(10);
    let salt = 0x205a_d082_d84b_3829;

    assert_eq!(
        policy.next(4, ProviderErrorKind::Retryable, Some(secs(2)), salt),
        policy.next(4, ProviderErrorKind::Retryable, Some(secs(2)), salt)
    );
}

// Extremes

#[test]
fn an_attempt_number_whose_wait_overflows_gives_the_cap() {
    let policy = doubling(u32::MAX);

    for attempt in [64, 1_100, 1 << 31, u32::MAX] {
        assert_eq!(after(&policy, attempt), Some(ms(10_000)), "{attempt}");
    }
    assert_eq!(after(&doubling(u32::MAX - 1), u32::MAX), None);
}

#[test]
fn the_largest_factor_gives_the_base_and_then_the_cap() {
    let policy = RetryPolicy::new(RetrySettings {
        factor: f64::MAX,
        ..settings(u32::MAX)
    })
    .unwrap();

    assert_eq!(after(&policy, 1), Some(ms(100)));
    assert_eq!(after(&policy, 2), Some(ms(10_000)));
    assert_eq!(after(&policy, u32::MAX - 1), Some(ms(10_000)));
}

#[test]
fn a_base_of_zero_waits_zero_however_far_the_scale_overflows_and_whatever_the_jitter() {
    let policy = RetryPolicy::new(RetrySettings {
        base: Duration::ZERO,
        jitter: 1.0,
        ..settings(u32::MAX)
    })
    .unwrap();

    assert_eq!(after(&policy, 1), Some(Duration::ZERO));
    assert_eq!(after(&policy, 5_000), Some(Duration::ZERO));
    assert_eq!(after(&policy, u32::MAX - 1), Some(Duration::ZERO));
    assert_eq!(
        hinted(&policy, secs(2), HALF),
        Some(secs(3)),
        "a hint is waited whatever the base"
    );
}

#[test]
fn the_longest_durations_are_a_valid_policy_whose_waits_stop_at_the_longest() {
    let policy = RetryPolicy::new(RetrySettings {
        base: Duration::MAX,
        max: Duration::MAX,
        hint_max: Duration::MAX,
        jitter: 1.0,
        ..settings(u32::MAX)
    })
    .unwrap();

    assert_eq!(after(&policy, 1), Some(Duration::MAX));
    assert_eq!(after(&policy, 2), Some(Duration::MAX));
    assert_eq!(hinted(&policy, Duration::MAX, MOST), Some(Duration::MAX));
    assert_eq!(hinted(&policy, Duration::MAX, NONE), Some(Duration::MAX));
}

// Validation

#[test]
fn a_base_longer_than_the_cap_is_refused_and_one_equal_to_it_is_not() {
    let with_base = |base| {
        RetryPolicy::new(RetrySettings {
            base,
            ..settings(3)
        })
    };

    assert_eq!(
        with_base(ms(10_001)),
        Err(RetryPolicyError::BaseAboveMax {
            base: ms(10_001),
            max: ms(10_000),
        })
    );
    with_base(ms(10_000)).unwrap();
    with_base(ms(9_999)).unwrap();
}

fn with_factor(factor: f64) -> Result<RetryPolicy, RetryPolicyError> {
    RetryPolicy::new(RetrySettings {
        factor,
        ..settings(3)
    })
}

#[test]
fn a_factor_below_one_is_refused_and_one_itself_is_not() {
    for factor in [0.999, 0.5, 0.0, -2.0] {
        assert_eq!(
            with_factor(factor),
            Err(RetryPolicyError::Factor(factor)),
            "{factor}"
        );
    }
    with_factor(1.0).unwrap();
    with_factor(1.001).unwrap();
}

#[test]
fn a_factor_that_is_not_a_finite_number_is_refused() {
    for factor in [f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            with_factor(factor),
            Err(RetryPolicyError::Factor(factor)),
            "{factor}"
        );
    }
    assert!(matches!(
        with_factor(f64::NAN),
        Err(RetryPolicyError::Factor(factor)) if factor.is_nan()
    ));
}

fn with_jitter(jitter: f64) -> Result<RetryPolicy, RetryPolicyError> {
    RetryPolicy::new(RetrySettings {
        jitter,
        ..settings(3)
    })
}

#[test]
fn a_jitter_outside_zero_to_one_is_refused_and_both_ends_are_not() {
    for jitter in [-0.001, -1.0, 1.001, 2.0, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            with_jitter(jitter),
            Err(RetryPolicyError::Jitter(jitter)),
            "{jitter}"
        );
    }
    assert!(matches!(
        with_jitter(f64::NAN),
        Err(RetryPolicyError::Jitter(jitter)) if jitter.is_nan()
    ));
    for jitter in [0.0, 0.25, 1.0] {
        assert!(with_jitter(jitter).is_ok(), "{jitter}");
    }
}

#[test]
fn the_first_broken_rule_in_the_order_of_the_fields_is_the_one_reported() {
    let broken = RetrySettings {
        max_retries: 0,
        base: ms(2),
        max: ms(1),
        factor: f64::NAN,
        hint_max: Duration::ZERO,
        jitter: 2.0,
    };

    assert_eq!(
        RetryPolicy::new(broken),
        Err(RetryPolicyError::BaseAboveMax {
            base: ms(2),
            max: ms(1),
        })
    );
    assert_eq!(
        RetryPolicy::new(RetrySettings {
            factor: 0.5,
            max: ms(2),
            ..broken
        }),
        Err(RetryPolicyError::Factor(0.5))
    );
}

#[test]
fn each_error_says_what_was_wrong() {
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
    assert_eq!(
        RetryPolicyError::Jitter(1.5).to_string(),
        "retry jitter 1.5 isn't a number from 0 to 1"
    );
}

/// The policy owns the whole rule, so a failure another attempt can't answer
/// stops the call however much budget is left.
#[test]
fn a_failure_that_another_attempt_cannot_answer_is_never_retried() {
    let policy = doubling(u32::MAX);

    for kind in ProviderErrorKind::ALL {
        let expected = match kind {
            ProviderErrorKind::ContextExhausted
            | ProviderErrorKind::Auth
            | ProviderErrorKind::Fatal => None,
            ProviderErrorKind::Retryable | ProviderErrorKind::Malformed => Some(ms(100)),
        };
        assert_eq!(policy.next(1, kind, None, NONE), expected, "{kind}");
    }
}
