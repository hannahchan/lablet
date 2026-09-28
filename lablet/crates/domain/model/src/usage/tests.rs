use serde_json::json;

use super::*;
use crate::TokenCounts;

/// What a provider that reports every count gives, with no reasoning.
const fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Usage {
    Usage {
        input_tokens: input,
        output_tokens: output,
        reasoning_output_tokens: Some(0),
        cache_read_tokens: Some(cache_read),
        cache_write_tokens: Some(cache_write),
    }
}

/// What a provider that reports only its input and output gives.
const fn bare(input: u64, output: u64) -> Usage {
    Usage {
        input_tokens: input,
        output_tokens: output,
        reasoning_output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

#[test]
fn the_default_usage_is_no_tokens_and_nothing_reported() {
    assert_eq!(Usage::default(), bare(0, 0));
}

#[test]
fn usage_serialises_all_five_fields_and_a_count_nobody_reported_as_null() {
    assert_eq!(
        serde_json::to_string(&Usage {
            reasoning_output_tokens: Some(5),
            ..usage(1, 2, 3, 4)
        })
        .unwrap(),
        concat!(
            r#"{"input_tokens":1,"output_tokens":2,"reasoning_output_tokens":5,"#,
            r#""cache_read_tokens":3,"cache_write_tokens":4}"#
        )
    );
    assert_eq!(
        serde_json::to_string(&Usage {
            cache_read_tokens: Some(0),
            ..bare(1, 2)
        })
        .unwrap(),
        concat!(
            r#"{"input_tokens":1,"output_tokens":2,"reasoning_output_tokens":null,"#,
            r#""cache_read_tokens":0,"cache_write_tokens":null}"#
        )
    );
}

#[test]
fn a_count_left_out_reads_as_zero_for_the_totals_and_as_not_reported_for_their_parts() {
    assert_eq!(
        serde_json::from_value::<Usage>(json!({})).unwrap(),
        bare(0, 0)
    );
    assert_eq!(
        serde_json::from_value::<Usage>(json!({ "input_tokens": 9, "output_tokens": 1 })).unwrap(),
        bare(9, 1)
    );
    assert_eq!(
        serde_json::from_value::<Usage>(json!({
            "input_tokens": 9,
            "output_tokens": 1,
            "reasoning_output_tokens": null,
            "cache_read_tokens": 0,
            "cache_write_tokens": 4,
        }))
        .unwrap(),
        Usage {
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(4),
            ..bare(9, 1)
        }
    );
}

#[test]
fn usage_reads_back_as_what_was_written() {
    for written in [
        bare(9, 1),
        usage(9, 1, 0, 4),
        Usage {
            reasoning_output_tokens: Some(1),
            ..bare(9, 1)
        },
    ] {
        let json = serde_json::to_string(&written).unwrap();

        assert_eq!(serde_json::from_str::<Usage>(&json).unwrap(), written);
    }
}

#[test]
fn from_inclusive_takes_every_count_as_it_is() {
    assert_eq!(
        Usage::from_inclusive(TokenCounts {
            input: 1000,
            output: 200,
            reasoning: Some(50),
            cache_read: Some(700),
            cache_write: Some(100)
        }),
        Usage {
            reasoning_output_tokens: Some(50),
            ..usage(1000, 200, 700, 100)
        }
    );
    assert_eq!(
        Usage::from_inclusive(TokenCounts {
            input: 1000,
            output: 200,
            reasoning: None,
            cache_read: Some(0),
            cache_write: None
        }),
        Usage {
            cache_read_tokens: Some(0),
            ..bare(1000, 200)
        }
    );
}

#[test]
fn from_uncached_adds_both_cache_counts_into_the_input() {
    let counts = TokenCounts {
        input: 200,
        output: 50,
        reasoning: Some(0),
        cache_read: Some(700),
        cache_write: Some(100),
    };

    assert_eq!(Usage::from_uncached(counts), usage(1000, 50, 700, 100));
    assert_eq!(Usage::from_uncached(counts).uncached_input_tokens(), 200);
    assert_eq!(
        Usage::from_uncached(TokenCounts {
            input: u64::MAX,
            output: 0,
            reasoning: Some(0),
            cache_read: Some(1),
            cache_write: Some(1)
        })
        .input_tokens,
        u64::MAX
    );
}

#[test]
fn from_uncached_adds_nothing_for_a_cache_count_the_provider_did_not_report() {
    let read_only = Usage::from_uncached(TokenCounts {
        input: 200,
        output: 50,
        reasoning: None,
        cache_read: Some(700),
        cache_write: None,
    });
    let written_only = Usage::from_uncached(TokenCounts {
        input: 200,
        output: 50,
        reasoning: None,
        cache_read: None,
        cache_write: Some(100),
    });
    let neither = Usage::from_uncached(TokenCounts {
        input: 200,
        output: 50,
        ..TokenCounts::default()
    });

    assert_eq!(
        read_only,
        Usage {
            cache_read_tokens: Some(700),
            ..bare(900, 50)
        }
    );
    assert_eq!(
        written_only,
        Usage {
            cache_write_tokens: Some(100),
            ..bare(300, 50)
        }
    );
    assert_eq!(neither, bare(200, 50));
}

#[test]
fn a_misspelt_usage_field_is_an_error_not_a_count_nobody_reported() {
    for misspelt in [
        json!({ "input_token": 12, "output_tokens": 3 }),
        json!({ "input_tokens": 12, "output_tokens": 3, "cache_read_token": 8 }),
    ] {
        let error = serde_json::from_value::<Usage>(misspelt).unwrap_err();

        assert!(error.to_string().contains("unknown field"), "{error}");
    }
}

#[test]
fn total_is_input_plus_output_because_input_already_holds_the_cached_tokens() {
    assert_eq!(usage(1000, 200, 700, 100).total(), 1200);
    assert_eq!(bare(1000, 200).total(), 1200);
}

#[test]
fn uncached_input_is_input_less_both_cache_fields() {
    assert_eq!(usage(1000, 200, 700, 100).uncached_input_tokens(), 200);
    assert_eq!(usage(1000, 200, 0, 0).uncached_input_tokens(), 1000);
}

#[test]
fn a_cache_count_nobody_reported_takes_nothing_from_the_uncached_input() {
    let read_only = Usage {
        cache_read_tokens: Some(700),
        ..bare(1000, 200)
    };
    let written_only = Usage {
        cache_write_tokens: Some(100),
        ..bare(1000, 200)
    };

    assert_eq!(bare(1000, 200).uncached_input_tokens(), 1000);
    assert_eq!(read_only.uncached_input_tokens(), 300);
    assert_eq!(written_only.uncached_input_tokens(), 900);
}

#[test]
fn uncached_input_stops_at_zero_when_a_provider_reports_more_cache_than_input() {
    assert_eq!(usage(10, 0, 8, 8).uncached_input_tokens(), 0);
}

#[test]
fn usage_adds_field_by_field() {
    let one = Usage {
        reasoning_output_tokens: Some(7),
        ..usage(1, 20, 300, 4000)
    };
    let other = Usage {
        reasoning_output_tokens: Some(11),
        ..usage(5, 60, 700, 8000)
    };

    assert_eq!(
        one + other,
        Usage {
            reasoning_output_tokens: Some(18),
            ..usage(6, 80, 1000, 12000)
        }
    );
}

#[test]
fn a_sum_lacks_a_count_only_when_no_side_reported_it() {
    let reasoning = Usage {
        reasoning_output_tokens: Some(7),
        ..bare(1, 20)
    };
    let read = Usage {
        cache_read_tokens: Some(300),
        ..bare(5, 60)
    };
    let written = Usage {
        cache_write_tokens: Some(4000),
        ..bare(9, 100)
    };

    assert_eq!(bare(1, 20) + bare(5, 60), bare(6, 80));
    assert_eq!(
        reasoning + read,
        Usage {
            reasoning_output_tokens: Some(7),
            cache_read_tokens: Some(300),
            ..bare(6, 80)
        }
    );
    assert_eq!(
        read + reasoning + written,
        Usage {
            reasoning_output_tokens: Some(7),
            cache_read_tokens: Some(300),
            cache_write_tokens: Some(4000),
            ..bare(15, 180)
        }
    );
}

#[test]
fn a_reported_zero_is_still_reported_in_a_sum() {
    assert_eq!(usage(1, 2, 0, 0) + bare(10, 20), usage(11, 22, 0, 0));
    assert_eq!(bare(10, 20) + usage(1, 2, 0, 0), usage(11, 22, 0, 0));
}

#[test]
fn add_assign_accumulates_like_add() {
    let mut running = usage(1, 2, 3, 4);
    running += usage(10, 20, 30, 40);
    running += bare(100, 200);

    assert_eq!(running, usage(111, 222, 33, 44));
}

#[test]
fn sums_saturate_rather_than_overflow() {
    let full = Usage {
        reasoning_output_tokens: Some(u64::MAX),
        ..usage(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
    };
    let one = Usage {
        reasoning_output_tokens: Some(1),
        ..usage(1, 1, 1, 1)
    };

    assert_eq!(full + one, full);
    assert_eq!(full.total(), u64::MAX);
}

/// A count across the whole range, biased to the small values and the
/// extremes where saturation bites.
fn any_count() -> impl proptest::strategy::Strategy<Value = u64> {
    proptest::prop_oneof![0u64..1_000_000, u64::MAX - 8..=u64::MAX]
}

/// A count a provider may not have reported.
fn any_part() -> impl proptest::strategy::Strategy<Value = Option<u64>> {
    proptest::option::of(any_count())
}

proptest::prop_compose! {
    fn any_counts()(
        input in any_count(),
        output in any_count(),
        reasoning in any_part(),
        cache_read in any_part(),
        cache_write in any_part(),
    ) -> TokenCounts {
        TokenCounts { input, output, reasoning, cache_read, cache_write }
    }
}

proptest::prop_compose! {
    fn any_usage()(counts in any_counts()) -> Usage {
        Usage::from_inclusive(counts)
    }
}

/// The three counts a provider may leave out, in field order.
const fn parts(usage: Usage) -> [Option<u64>; 3] {
    [
        usage.reasoning_output_tokens,
        usage.cache_read_tokens,
        usage.cache_write_tokens,
    ]
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

    /// Summing usage is a commutative monoid: a run's totals are the same
    /// whatever order the turns are added in, and an empty run adds nothing,
    /// not even a count of zero where nothing was reported.
    #[test]
    fn summing_usage_is_a_commutative_monoid(
        a in any_usage(), b in any_usage(), c in any_usage()
    ) {
        proptest::prop_assert_eq!(a + Usage::default(), a);
        proptest::prop_assert_eq!(Usage::default() + a, a);
        proptest::prop_assert_eq!(a + b, b + a);
        proptest::prop_assert_eq!((a + b) + c, a + (b + c));
    }

    /// A sum reports a count exactly when a side did, and what it reports is
    /// what the sides reported, added up.
    #[test]
    fn a_sum_reports_what_its_sides_reported(a in any_usage(), b in any_usage()) {
        for ((sum, one), other) in parts(a + b).into_iter().zip(parts(a)).zip(parts(b)) {
            proptest::prop_assert_eq!(sum.is_some(), one.is_some() || other.is_some());
            proptest::prop_assert_eq!(
                sum.unwrap_or(0),
                one.unwrap_or(0).saturating_add(other.unwrap_or(0))
            );
        }
    }

    /// Both totals hold their subsets, so neither is ever added to, whether
    /// or not the subsets were reported.
    #[test]
    fn the_totals_never_double_count_the_parts_they_hold(usage in any_usage()) {
        proptest::prop_assert_eq!(
            usage.total(),
            usage.input_tokens.saturating_add(usage.output_tokens)
        );
        proptest::prop_assert!(usage.uncached_input_tokens() <= usage.input_tokens);
    }

    /// A count nobody reported is read as a count of zero would be.
    #[test]
    fn a_missing_count_is_read_as_nothing(usage in any_usage()) {
        let zeroed = Usage {
            reasoning_output_tokens: Some(usage.reasoning_output_tokens.unwrap_or(0)),
            cache_read_tokens: Some(usage.cache_read_tokens.unwrap_or(0)),
            cache_write_tokens: Some(usage.cache_write_tokens.unwrap_or(0)),
            ..usage
        };

        proptest::prop_assert_eq!(usage.total(), zeroed.total());
        proptest::prop_assert_eq!(usage.uncached_input_tokens(), zeroed.uncached_input_tokens());
    }

    /// The two constructors agree on everything but the input count, and
    /// taking the cache counts back out of an input they were added into
    /// gives the provider's own number.
    #[test]
    fn from_uncached_is_from_inclusive_with_the_cache_counts_added_in(
        counts in any_counts()
    ) {
        let usage = Usage::from_uncached(counts);
        let cached = counts.cache_read.unwrap_or(0).saturating_add(counts.cache_write.unwrap_or(0));

        proptest::prop_assert_eq!(parts(usage), parts(Usage::from_inclusive(counts)));
        proptest::prop_assert_eq!(usage.output_tokens, counts.output);
        proptest::prop_assert_eq!(usage.input_tokens, counts.input.saturating_add(cached));
        if counts.input.checked_add(cached).is_some() {
            proptest::prop_assert_eq!(usage.uncached_input_tokens(), counts.input);
        }
    }
}
