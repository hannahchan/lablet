use super::*;

/// Rates an f64 holds exactly, so the sums below are exact: 4 for input, 16
/// for output, 0.5 for a cache read, 5 for a cache write.
fn pricing() -> Pricing {
    Pricing::new(4.0, 16.0, 0.5, 5.0).unwrap()
}

fn cost(usage: Usage) -> Cost {
    pricing().cost(&usage)
}

const fn usd(usd: f64) -> Cost {
    Cost::new(usd)
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
        cache_read_tokens: 1_000_000,
        ..Usage::default()
    };
    let cache_write = Usage {
        input_tokens: 1_000_000,
        cache_write_tokens: 1_000_000,
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
        cache_read_tokens: 500_000,
        cache_write_tokens: 250_000,
    };

    // 250k uncached at 4, 250k output at 16, 500k read at 0.5, 250k written at 5.
    assert_eq!(cost(usage), usd(1.0 + 4.0 + 0.25 + 1.25));
}

#[test]
fn cached_tokens_inside_the_input_count_are_billed_once() {
    let usage = Usage {
        input_tokens: 1_000_000,
        output_tokens: 0,
        cache_read_tokens: 500_000,
        cache_write_tokens: 250_000,
    };
    let double_counted = 1_000_000.0 * 4.0 / 1e6 + 500_000.0 * 0.5 / 1e6 + 250_000.0 * 5.0 / 1e6;

    assert_eq!(usd(double_counted), usd(5.5));
    assert_eq!(cost(usage), usd(2.5));
}

#[test]
fn an_input_that_is_all_cache_reads_pays_only_the_cache_read_rate() {
    let usage = Usage {
        input_tokens: 2_000_000,
        cache_read_tokens: 2_000_000,
        ..Usage::default()
    };

    assert_eq!(cost(usage), usd(1.0));
}

#[test]
fn cache_counts_above_the_input_count_never_make_the_input_part_negative() {
    let usage = Usage {
        input_tokens: 100,
        cache_read_tokens: 1_000_000,
        cache_write_tokens: 1_000_000,
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
        cache_read_tokens: u64::MAX,
        cache_write_tokens: u64::MAX,
    };
    let cost = cost(usage).usd();

    assert!(cost.is_finite());
    assert!(cost > 0.0);
}

#[test]
fn free_pricing_costs_nothing() {
    let free = Pricing::new(0.0, 0.0, 0.0, 0.0).unwrap();
    let usage = Usage {
        input_tokens: 1_000_000,
        output_tokens: 1_000_000,
        cache_read_tokens: 10,
        cache_write_tokens: 10,
    };

    assert_eq!(free.cost(&usage), usd(0.0));
}

#[test]
fn a_negative_rate_is_refused_and_the_error_names_it() {
    let refused = |name, result: Result<Pricing, PricingError>| {
        assert_eq!(result, Err(PricingError::Rate { name, value: -0.01 }));
    };

    refused("input", Pricing::new(-0.01, 16.0, 0.5, 5.0));
    refused("output", Pricing::new(4.0, -0.01, 0.5, 5.0));
    refused("cache_read", Pricing::new(4.0, 16.0, -0.01, 5.0));
    refused("cache_write", Pricing::new(4.0, 16.0, 0.5, -0.01));
}

#[test]
fn a_rate_that_is_not_a_finite_number_is_refused() {
    assert_eq!(
        Pricing::new(4.0, f64::INFINITY, 0.5, 5.0),
        Err(PricingError::Rate {
            name: "output",
            value: f64::INFINITY,
        })
    );
    assert!(matches!(
        Pricing::new(4.0, 16.0, f64::NAN, 5.0),
        Err(PricingError::Rate { name: "cache_read", value }) if value.is_nan()
    ));
}

#[test]
fn the_first_refused_rate_in_argument_order_is_the_one_reported() {
    assert_eq!(
        Pricing::new(4.0, -1.0, -2.0, -3.0),
        Err(PricingError::Rate {
            name: "output",
            value: -1.0,
        })
    );
}

#[test]
fn the_error_says_which_rate_and_what_it_was() {
    assert_eq!(
        Pricing::new(-3.0, 16.0, 0.5, 5.0).unwrap_err().to_string(),
        "input rate -3 isn't a finite number of at least 0"
    );
}
