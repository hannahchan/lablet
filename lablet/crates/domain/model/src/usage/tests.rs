use serde_json::json;

use super::*;
use crate::TokenCounts;

const fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Usage {
    Usage {
        input_tokens: input,
        output_tokens: output,
        reasoning_output_tokens: 0,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    }
}

#[test]
fn the_default_usage_is_zero() {
    assert_eq!(Usage::default(), usage(0, 0, 0, 0));
}

#[test]
fn usage_serialises_all_four_fields_and_reads_a_missing_one_as_zero() {
    assert_eq!(
        serde_json::to_value(usage(1, 2, 3, 4)).unwrap(),
        json!({
            "input_tokens": 1,
            "output_tokens": 2,
            "reasoning_output_tokens": 0,
            "cache_read_tokens": 3,
            "cache_write_tokens": 4,
        })
    );
    assert_eq!(
        serde_json::from_value::<Usage>(json!({ "input_tokens": 9, "output_tokens": 1 })).unwrap(),
        usage(9, 1, 0, 0)
    );
}

#[test]
fn from_inclusive_takes_the_input_count_as_it_is() {
    assert_eq!(
        Usage::from_inclusive(TokenCounts {
            input: 1000,
            output: 200,
            reasoning: 0,
            cache_read: 700,
            cache_write: 100
        }),
        usage(1000, 200, 700, 100)
    );
}

#[test]
fn from_uncached_adds_both_cache_counts_into_the_input() {
    assert_eq!(
        Usage::from_uncached(TokenCounts {
            input: 200,
            output: 50,
            reasoning: 0,
            cache_read: 700,
            cache_write: 100
        }),
        usage(1000, 50, 700, 100)
    );
    assert_eq!(
        Usage::from_uncached(TokenCounts {
            input: 200,
            output: 50,
            reasoning: 0,
            cache_read: 700,
            cache_write: 100
        })
        .uncached_input_tokens(),
        200
    );
    assert_eq!(
        Usage::from_uncached(TokenCounts {
            input: u64::MAX,
            output: 0,
            reasoning: 0,
            cache_read: 1,
            cache_write: 1
        })
        .input_tokens,
        u64::MAX
    );
}

#[test]
fn a_misspelt_usage_field_is_an_error_not_a_silent_zero() {
    let error = serde_json::from_value::<Usage>(json!({ "input_token": 12, "output_tokens": 3 }))
        .unwrap_err();

    assert!(error.to_string().contains("input_token"), "{error}");
}

proptest::prop_compose! {
    /// Counts across the whole range, biased to the small values and the
    /// extremes where saturation bites.
    fn any_usage()(
        input in proptest::prop_oneof![0u64..1_000_000, u64::MAX - 8..=u64::MAX],
        output in proptest::prop_oneof![0u64..1_000_000, u64::MAX - 8..=u64::MAX],
        reasoning in 0u64..1_000_000,
        cache_read in proptest::prop_oneof![0u64..1_000_000, u64::MAX - 8..=u64::MAX],
        cache_write in 0u64..1_000_000,
    ) -> Usage {
        Usage::from_inclusive(TokenCounts { input, output, reasoning, cache_read, cache_write })
    }
}

#[test]
fn total_is_input_plus_output_because_input_already_holds_the_cached_tokens() {
    assert_eq!(usage(1000, 200, 700, 100).total(), 1200);
}

#[test]
fn uncached_input_is_input_less_both_cache_fields() {
    assert_eq!(usage(1000, 200, 700, 100).uncached_input_tokens(), 200);
    assert_eq!(usage(1000, 200, 0, 0).uncached_input_tokens(), 1000);
}

#[test]
fn uncached_input_stops_at_zero_when_a_provider_reports_more_cache_than_input() {
    assert_eq!(usage(10, 0, 8, 8).uncached_input_tokens(), 0);
}

#[test]
fn usage_adds_field_by_field() {
    let sum = usage(1, 20, 300, 4000) + usage(5, 60, 700, 8000);

    assert_eq!(sum, usage(6, 80, 1000, 12000));
}

#[test]
fn add_assign_accumulates_like_add() {
    let mut running = usage(1, 2, 3, 4);
    running += usage(10, 20, 30, 40);
    running += usage(100, 200, 300, 400);

    assert_eq!(running, usage(111, 222, 333, 444));
}

#[test]
fn sums_saturate_rather_than_overflow() {
    let full = usage(u64::MAX, u64::MAX, u64::MAX, u64::MAX);

    assert_eq!(full + usage(1, 1, 1, 1), full);
    assert_eq!(full.total(), u64::MAX);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

    /// Summing usage is a commutative monoid: a run's totals are the same
    /// whatever order the turns are added in, and an empty run adds nothing.
    #[test]
    fn summing_usage_is_a_commutative_monoid(
        a in any_usage(), b in any_usage(), c in any_usage()
    ) {
        proptest::prop_assert_eq!(a + Usage::default(), a);
        proptest::prop_assert_eq!(Usage::default() + a, a);
        proptest::prop_assert_eq!(a + b, b + a);
        proptest::prop_assert_eq!((a + b) + c, a + (b + c));
    }

    /// Both totals hold their subsets, so neither is ever added to.
    #[test]
    fn the_totals_never_double_count_the_parts_they_hold(usage in any_usage()) {
        proptest::prop_assert_eq!(
            usage.total(),
            usage.input_tokens.saturating_add(usage.output_tokens)
        );
        proptest::prop_assert!(usage.uncached_input_tokens() <= usage.input_tokens);
    }

}
