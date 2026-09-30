use lablet_model::{Rates, TokenCounts};

use super::*;

/// Rates an f64 holds exactly, so the sums below are exact: 4 for input, 16
/// for output, 0.5 for a cache read, 5 for a cache write.
fn pricing() -> Pricing {
    Pricing::new(Rates::new(4.0, 16.0, 0.5, 5.0).unwrap())
}

fn cost(usage: Usage) -> Option<Cost> {
    pricing().cost(&usage)
}

fn usd(usd: f64) -> Option<Cost> {
    Cost::new(usd).ok()
}

#[test]
fn no_usage_costs_nothing() {
    assert_eq!(cost(Usage::default()), usd(0.0));
}

#[test]
fn a_million_tokens_of_each_kind_cost_that_kind_s_rate() {
    let input = Usage {
        input_tokens: 1_000_000,
        ..Usage::default()
    };
    let output = Usage {
        output_tokens: 1_000_000,
        ..Usage::default()
    };
    let cache_read = Usage {
        input_tokens: 1_000_000,
        cache_read_tokens: Some(1_000_000),
        ..Usage::default()
    };
    let cache_write = Usage {
        input_tokens: 1_000_000,
        cache_write_tokens: Some(1_000_000),
        ..Usage::default()
    };

    assert_eq!(cost(input), usd(4.0));
    assert_eq!(cost(output), usd(16.0));
    assert_eq!(cost(cache_read), usd(0.5));
    assert_eq!(cost(cache_write), usd(5.0));
}

#[test]
fn a_cost_is_the_sum_of_its_four_parts_in_proportion_to_the_tokens() {
    let usage = Usage {
        input_tokens: 1_000_000,
        output_tokens: 250_000,
        reasoning_output_tokens: Some(0),
        cache_read_tokens: Some(500_000),
        cache_write_tokens: Some(250_000),
    };

    // 250k uncached at 4, 250k output at 16, 500k read at 0.5, 250k written at 5.
    assert_eq!(cost(usage), usd(1.0 + 4.0 + 0.25 + 1.25));
}

#[test]
fn cached_tokens_inside_the_input_count_are_billed_once() {
    let usage = Usage {
        input_tokens: 1_000_000,
        output_tokens: 0,
        reasoning_output_tokens: Some(0),
        cache_read_tokens: Some(500_000),
        cache_write_tokens: Some(250_000),
    };
    let double_counted = 1_000_000.0 * 4.0 / 1e6 + 500_000.0 * 0.5 / 1e6 + 250_000.0 * 5.0 / 1e6;

    assert_eq!(usd(double_counted), usd(5.5));
    assert_eq!(cost(usage), usd(2.5));
}

#[test]
fn a_cache_count_the_provider_did_not_report_is_nothing_to_price() {
    let nothing_reported = Usage {
        input_tokens: 1_000_000,
        output_tokens: 250_000,
        ..Usage::default()
    };
    let reads_only = Usage {
        cache_read_tokens: Some(500_000),
        ..nothing_reported
    };
    let writes_only = Usage {
        cache_write_tokens: Some(250_000),
        ..nothing_reported
    };

    assert_eq!(nothing_reported.cache_read_tokens, None);
    assert_eq!(nothing_reported.cache_write_tokens, None);
    // The input whole at 4 and the output at 16.
    assert_eq!(cost(nothing_reported), usd(4.0 + 4.0));
    // 500k uncached at 4, 500k read at 0.5.
    assert_eq!(cost(reads_only), usd(2.0 + 4.0 + 0.25));
    // 750k uncached at 4, 250k written at 5.
    assert_eq!(cost(writes_only), usd(3.0 + 4.0 + 1.25));
}

#[test]
fn a_reasoning_count_changes_no_cost_whether_or_not_it_was_reported() {
    let unreported = Usage {
        input_tokens: 1_000_000,
        output_tokens: 250_000,
        ..Usage::default()
    };
    let reported = Usage {
        reasoning_output_tokens: Some(200_000),
        ..unreported
    };

    assert_eq!(cost(reported), cost(unreported));
    assert_eq!(cost(reported), usd(4.0 + 4.0));
}

#[test]
fn an_input_that_is_all_cache_reads_pays_only_the_cache_read_rate() {
    let usage = Usage {
        input_tokens: 2_000_000,
        cache_read_tokens: Some(2_000_000),
        ..Usage::default()
    };

    assert_eq!(cost(usage), usd(1.0));
}

#[test]
fn cache_counts_above_the_input_count_never_make_the_input_part_negative() {
    let usage = Usage {
        input_tokens: 100,
        cache_read_tokens: Some(1_000_000),
        cache_write_tokens: Some(1_000_000),
        ..Usage::default()
    };

    assert_eq!(cost(usage), usd(0.5 + 5.0));
}

#[test]
fn a_single_token_costs_a_millionth_of_the_rate() {
    let usage = Usage {
        output_tokens: 1,
        ..Usage::default()
    };

    assert_eq!(cost(usage), usd(16.0 / 1e6));
}

#[test]
fn the_largest_usage_has_a_finite_cost() {
    let usage = Usage {
        input_tokens: u64::MAX,
        output_tokens: u64::MAX,
        reasoning_output_tokens: Some(0),
        cache_read_tokens: Some(u64::MAX),
        cache_write_tokens: Some(u64::MAX),
    };
    let cost = cost(usage)
        .expect("rates this ordinary can't overflow")
        .usd();

    assert!(cost.is_finite());
    assert!(cost > 0.0);
}

#[test]
fn a_cost_that_overflows_an_f64_is_reported_as_no_cost() {
    let absurd = Pricing::new(Rates::new(f64::MAX, 0.0, 0.0, 0.0).unwrap());
    let usage = Usage {
        input_tokens: u64::MAX,
        ..Usage::default()
    };

    assert_eq!(absurd.cost(&usage), None);
}

#[test]
fn free_pricing_costs_nothing() {
    let free = Pricing::new(Rates::new(0.0, 0.0, 0.0, 0.0).unwrap());
    let usage = Usage {
        input_tokens: 1_000_000,
        output_tokens: 1_000_000,
        reasoning_output_tokens: Some(0),
        cache_read_tokens: Some(10),
        cache_write_tokens: Some(10),
    };

    assert_eq!(free.cost(&usage), usd(0.0));
}

#[test]
fn the_pricing_reports_the_rates_it_was_built_with() {
    assert_eq!(pricing().rates(), Rates::new(4.0, 16.0, 0.5, 5.0).unwrap());
}

proptest::prop_compose! {
    /// A usage whose cache counts are a part of its input, as every provider
    /// that can count reports it, and which leaves out any count a provider
    /// may.
    fn consistent_usage()(
        uncached in 0u64..10_000_000,
        output in 0u64..10_000_000,
        reasoning in proptest::option::of(0u64..1_000_000),
        cache_read in proptest::option::of(0u64..10_000_000),
        cache_write in proptest::option::of(0u64..10_000_000),
    ) -> Usage {
        Usage::from_uncached(TokenCounts {
            input: uncached,
            output,
            reasoning,
            cache_read,
            cache_write,
        })
    }
}

/// Tokens of one kind a run is billed for, as a provider reports them: a
/// cached token is part of the input too, and a reasoning token part of the
/// output.
fn tokens_of_one_kind() -> impl proptest::strategy::Strategy<Value = Usage> {
    use proptest::strategy::{Just, Strategy};

    (0u64..1_000_000).prop_flat_map(|extra| {
        let none = TokenCounts::default();
        proptest::prop_oneof![
            Just(TokenCounts {
                input: extra,
                ..none
            }),
            Just(TokenCounts {
                output: extra,
                ..none
            }),
            Just(TokenCounts {
                output: extra,
                reasoning: Some(extra),
                ..none
            }),
            Just(TokenCounts {
                cache_read: Some(extra),
                ..none
            }),
            Just(TokenCounts {
                cache_write: Some(extra),
                ..none
            }),
        ]
        .prop_map(Usage::from_uncached)
    })
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

    /// The law a composer depends on: summing the cost of each run gives the
    /// same answer as pricing the summed usage. It holds for counts a
    /// provider could actually report, and the wide event carries the rates
    /// so a consumer can tell when one didn't; `uncached_input_tokens`
    /// saturating to zero is what breaks it otherwise.
    #[test]
    fn cost_is_additive_over_usages_a_provider_could_report(
        a in consistent_usage(), b in consistent_usage()
    ) {
        let parts = pricing().cost(&a).unwrap().usd() + pricing().cost(&b).unwrap().usd();
        let whole = pricing().cost(&(a + b)).unwrap().usd();

        proptest::prop_assert!(
            (parts - whole).abs() <= whole.abs() * 1e-9 + 1e-9,
            "{parts} != {whole}"
        );
    }

    /// A count the provider didn't report is priced as a count of zero is, so
    /// leaving one out never changes what a run cost.
    #[test]
    fn a_missing_count_costs_what_a_count_of_zero_costs(usage in consistent_usage()) {
        let zeroed = Usage {
            reasoning_output_tokens: Some(usage.reasoning_output_tokens.unwrap_or(0)),
            cache_read_tokens: Some(usage.cache_read_tokens.unwrap_or(0)),
            cache_write_tokens: Some(usage.cache_write_tokens.unwrap_or(0)),
            ..usage
        };

        proptest::prop_assert_eq!(pricing().cost(&usage), pricing().cost(&zeroed));
    }

    /// More tokens never cost less, whichever kind they are.
    #[test]
    fn cost_never_falls_as_tokens_rise(usage in consistent_usage(), extra in tokens_of_one_kind()) {
        let more = usage + extra;

        proptest::prop_assert!(pricing().cost(&more).unwrap() >= pricing().cost(&usage).unwrap());
    }
}
