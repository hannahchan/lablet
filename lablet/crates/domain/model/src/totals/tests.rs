use serde_json::json;

use super::*;
use crate::{
    FinishReason, ToolCallEnd, ToolCallId, ToolCallStatus, ToolInput, ToolName, ToolResultContent,
    ToolSource, Usage,
};

/// The latency of calls that took `each`, added up in order.
fn latency(each: &[u64]) -> Latency {
    sum(each.iter().copied().map(Latency::of))
}

/// The record of a turn whose provider call took `attempts`, the last of
/// which answered after `latency_ms`.
fn record(attempts: u32, latency_ms: u64) -> TurnRecord {
    TurnRecord {
        usage: Usage::default(),
        finish: FinishReason::EndTurn,
        response_id: None,
        response_model: None,
        started_ms: 5,
        latency_ms,
        attempts,
    }
}

/// A call whose arguments are seven bytes of JSON.
fn call() -> ToolUse {
    ToolUse {
        id: ToolCallId::new("call_1").unwrap(),
        name: ToolName::new("bash").unwrap(),
        input: ToolInput::Json(json!({ "n": 1 })),
    }
}

/// The outcome of a call that took 30 ms and sent the model four bytes.
fn outcome(status: ToolCallStatus, truncated_from_bytes: Option<u64>) -> ToolCallOutcome {
    ToolCallOutcome {
        call_id: ToolCallId::new("call_1").unwrap(),
        status,
        started_ms: 5,
        latency_ms: 30,
        truncated_from_bytes,
        content: vec![ToolResultContent::Text("done".to_owned())],
    }
}

fn ran(source: ToolSource, ended: ToolCallEnd) -> ToolCallStatus {
    ToolCallStatus::ran(source, ended)
}

#[test]
fn every_total_starts_at_nothing() {
    assert_eq!(Latency::default(), Latency::of(0));
    assert_eq!((Latency::of(0).total_ms(), Latency::of(0).max_ms()), (0, 0));
    assert_eq!(
        ProviderTotals::default(),
        ProviderTotals {
            retries: 0,
            latency: Latency::of(0),
        }
    );
    assert_eq!(
        ToolCallTotals::default(),
        ToolCallTotals {
            errors: 0,
            unknown: 0,
            truncated: 0,
            latency_ms: 0,
            input_bytes: 0,
            output_bytes: 0,
        }
    );
    assert_eq!(
        ToolStats::default(),
        ToolStats {
            calls: 0,
            errors: 0,
            latency_ms: 0,
        }
    );
}

#[test]
fn the_sum_of_no_parts_is_nothing_and_of_some_is_each_of_them_added() {
    assert_eq!(latency(&[]), Latency::default());
    assert_eq!(latency(&[250]), Latency::of(250));
    assert_eq!(
        latency(&[250, 400, 90]),
        Latency::of(250) + Latency::of(400) + Latency::of(90)
    );
}

#[test]
fn the_latency_of_one_call_is_the_total_and_the_slowest() {
    let one = Latency::of(250);

    assert_eq!((one.total_ms(), one.max_ms()), (250, 250));
}

#[test]
fn latencies_add_to_their_sum_and_keep_the_slowest_of_them() {
    let slowest_first = latency(&[400, 250, 90]);
    let slowest_last = latency(&[90, 250, 400]);

    assert_eq!(
        (slowest_first.total_ms(), slowest_first.max_ms()),
        (740, 400)
    );
    assert_eq!(slowest_last, slowest_first);
}

#[test]
fn a_turn_adds_the_attempts_beyond_its_first_and_the_latency_of_the_one_that_answered() {
    assert_eq!(
        ProviderTotals::of(&record(1, 250)),
        ProviderTotals {
            retries: 0,
            latency: Latency::of(250),
        }
    );
    assert_eq!(
        ProviderTotals::of(&record(3, 120)),
        ProviderTotals {
            retries: 2,
            latency: Latency::of(120),
        }
    );
}

#[test]
fn provider_totals_add_field_by_field() {
    let one = ProviderTotals {
        retries: 2,
        latency: latency(&[100, 300]),
    };
    let other = ProviderTotals {
        retries: 5,
        latency: latency(&[250]),
    };

    assert_eq!(
        one + other,
        ProviderTotals {
            retries: 7,
            latency: latency(&[100, 300, 250]),
        }
    );
}

#[test]
fn a_call_that_ran_well_adds_its_latency_and_its_sizes_and_no_count() {
    assert_eq!(
        ToolCallTotals::of(
            &call(),
            &outcome(ran(ToolSource::Builtin, ToolCallEnd::Ok), None)
        ),
        ToolCallTotals {
            errors: 0,
            unknown: 0,
            truncated: 0,
            latency_ms: 30,
            input_bytes: 7,
            output_bytes: 4,
        }
    );
}

#[test]
fn a_call_adds_to_the_count_of_what_became_of_it() {
    let base = ToolCallTotals::of(
        &call(),
        &outcome(ran(ToolSource::Builtin, ToolCallEnd::Ok), None),
    );

    assert_eq!(
        ToolCallTotals::of(
            &call(),
            &outcome(ran(ToolSource::Builtin, ToolCallEnd::ToolError), None)
        ),
        ToolCallTotals { errors: 1, ..base }
    );
    assert_eq!(
        ToolCallTotals::of(&call(), &outcome(ToolCallStatus::Unknown, None)),
        ToolCallTotals {
            errors: 1,
            unknown: 1,
            ..base
        }
    );
    assert_eq!(
        ToolCallTotals::of(&call(), &outcome(ToolCallStatus::MalformedInput, None)),
        ToolCallTotals { errors: 1, ..base },
        "a call whose arguments didn't parse named a tool the run offered"
    );
    assert_eq!(
        ToolCallTotals::of(
            &call(),
            &outcome(ran(ToolSource::Builtin, ToolCallEnd::Ok), Some(5_000))
        ),
        ToolCallTotals {
            truncated: 1,
            ..base
        }
    );
}

#[test]
fn tool_call_totals_add_field_by_field() {
    let one = ToolCallTotals {
        errors: 1,
        unknown: 20,
        truncated: 300,
        latency_ms: 4_000,
        input_bytes: 50_000,
        output_bytes: 600_000,
    };
    let other = ToolCallTotals {
        errors: 2,
        unknown: 30,
        truncated: 400,
        latency_ms: 5_000,
        input_bytes: 60_000,
        output_bytes: 700_000,
    };

    assert_eq!(
        one + other,
        ToolCallTotals {
            errors: 3,
            unknown: 50,
            truncated: 700,
            latency_ms: 9_000,
            input_bytes: 110_000,
            output_bytes: 1_300_000,
        }
    );
}

#[test]
fn a_call_adds_one_call_to_its_tool_s_share_with_its_latency_and_whether_it_failed() {
    assert_eq!(
        ToolStats::of(&outcome(ran(ToolSource::Builtin, ToolCallEnd::Ok), None)),
        ToolStats {
            calls: 1,
            errors: 0,
            latency_ms: 30,
        }
    );
    assert_eq!(
        ToolStats::of(&outcome(
            ran(ToolSource::Builtin, ToolCallEnd::Timeout),
            None
        )),
        ToolStats {
            calls: 1,
            errors: 1,
            latency_ms: 30,
        }
    );
}

#[test]
fn tool_stats_add_field_by_field() {
    let one = ToolStats {
        calls: 1,
        errors: 20,
        latency_ms: 300,
    };
    let other = ToolStats {
        calls: 4,
        errors: 50,
        latency_ms: 600,
    };

    assert_eq!(
        one + other,
        ToolStats {
            calls: 5,
            errors: 70,
            latency_ms: 900,
        }
    );
}

#[test]
fn sums_saturate_rather_than_overflow() {
    let full = ToolCallTotals {
        errors: u64::MAX,
        unknown: u64::MAX,
        truncated: u64::MAX,
        latency_ms: u64::MAX,
        input_bytes: u64::MAX,
        output_bytes: u64::MAX,
    };
    let one = ToolCallTotals {
        errors: 1,
        unknown: 1,
        truncated: 1,
        latency_ms: 1,
        input_bytes: 1,
        output_bytes: 1,
    };
    let every_call = ToolStats {
        calls: u64::MAX,
        errors: u64::MAX,
        latency_ms: u64::MAX,
    };
    let one_call = ToolStats {
        calls: 1,
        errors: 1,
        latency_ms: 1,
    };
    let every_attempt = ProviderTotals {
        retries: u64::MAX,
        latency: Latency::of(u64::MAX),
    };
    let one_attempt = ProviderTotals {
        retries: 1,
        latency: Latency::of(1),
    };

    assert_eq!(full + one, full);
    assert_eq!(every_call + one_call, every_call);
    assert_eq!(every_attempt + one_attempt, every_attempt);
}

/// A count across the whole range, biased to the small values and the
/// extremes where saturation bites.
fn any_count() -> impl proptest::strategy::Strategy<Value = u64> {
    proptest::prop_oneof![0u64..1_000_000, u64::MAX - 8..=u64::MAX]
}

proptest::prop_compose! {
    fn any_latency()(each in proptest::collection::vec(any_count(), 0..4)) -> Latency {
        latency(&each)
    }
}

proptest::prop_compose! {
    fn any_provider_totals()(retries in any_count(), latency in any_latency()) -> ProviderTotals {
        ProviderTotals { retries, latency }
    }
}

proptest::prop_compose! {
    fn any_tool_call_totals()(
        errors in any_count(),
        unknown in any_count(),
        truncated in any_count(),
        latency_ms in any_count(),
        input_bytes in any_count(),
        output_bytes in any_count(),
    ) -> ToolCallTotals {
        ToolCallTotals { errors, unknown, truncated, latency_ms, input_bytes, output_bytes }
    }
}

proptest::prop_compose! {
    fn any_tool_stats()(
        calls in any_count(),
        errors in any_count(),
        latency_ms in any_count(),
    ) -> ToolStats {
        ToolStats { calls, errors, latency_ms }
    }
}

/// Holds an addition to the laws of a commutative monoid: a run's totals
/// are the same whatever order its calls are added in, and a run without
/// calls adds nothing.
macro_rules! assert_commutative_monoid {
    ($nothing:expr, $a:expr, $b:expr, $c:expr) => {
        proptest::prop_assert_eq!($a + $nothing, $a);
        proptest::prop_assert_eq!($nothing + $a, $a);
        proptest::prop_assert_eq!($a + $b, $b + $a);
        proptest::prop_assert_eq!(($a + $b) + $c, $a + ($b + $c));
    };
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

    #[test]
    fn summing_latencies_is_a_commutative_monoid(
        a in any_latency(), b in any_latency(), c in any_latency()
    ) {
        assert_commutative_monoid!(Latency::default(), a, b, c);
    }

    #[test]
    fn summing_provider_totals_is_a_commutative_monoid(
        a in any_provider_totals(), b in any_provider_totals(), c in any_provider_totals()
    ) {
        assert_commutative_monoid!(ProviderTotals::default(), a, b, c);
    }

    #[test]
    fn summing_tool_call_totals_is_a_commutative_monoid(
        a in any_tool_call_totals(), b in any_tool_call_totals(), c in any_tool_call_totals()
    ) {
        assert_commutative_monoid!(ToolCallTotals::default(), a, b, c);
    }

    #[test]
    fn summing_tool_stats_is_a_commutative_monoid(
        a in any_tool_stats(), b in any_tool_stats(), c in any_tool_stats()
    ) {
        assert_commutative_monoid!(ToolStats::default(), a, b, c);
    }

    /// The slowest call is one of the calls, so it's never longer than all
    /// of them together, and both are what the calls were.
    #[test]
    fn the_slowest_of_some_calls_is_among_them(
        each in proptest::collection::vec(any_count(), 0..6)
    ) {
        let summed = latency(&each);

        proptest::prop_assert!(summed.max_ms() <= summed.total_ms());
        proptest::prop_assert_eq!(summed.max_ms(), each.iter().copied().max().unwrap_or(0));
        proptest::prop_assert_eq!(
            summed.total_ms(),
            each.iter().fold(0u64, |total, latency| total.saturating_add(*latency))
        );
    }
}
