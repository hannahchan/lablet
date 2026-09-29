use lablet_model::{self as model, TokenCounts};
use serde_json::json;

use super::*;

/// What a provider that reports every count gives.
fn reported() -> model::Usage {
    model::Usage::from_inclusive(TokenCounts {
        input: 1_000,
        output: 200,
        reasoning: Some(50),
        cache_read: Some(700),
        cache_write: Some(0),
    })
}

/// What a provider that reports only its input and output gives.
fn bare() -> model::Usage {
    model::Usage::from_inclusive(TokenCounts {
        input: 9,
        output: 1,
        reasoning: None,
        cache_read: None,
        cache_write: None,
    })
}

#[test]
fn every_count_is_written_under_its_own_name_in_this_order() {
    assert_eq!(
        serde_json::to_string(&Usage::from(reported())).unwrap(),
        concat!(
            r#"{"input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":50,"#,
            r#""cache_read_tokens":700,"cache_write_tokens":0}"#
        )
    );
}

/// O14, the outcome's half: a count the provider didn't report is in the
/// document as `null`, and a count of zero as `0`.
#[test]
fn a_count_nobody_reported_is_written_as_null_and_never_left_out_or_zero() {
    assert_eq!(
        serde_json::to_string(&Usage::from(bare())).unwrap(),
        concat!(
            r#"{"input_tokens":9,"output_tokens":1,"reasoning_output_tokens":null,"#,
            r#""cache_read_tokens":null,"cache_write_tokens":null}"#
        )
    );
}

#[test]
fn usage_reads_back_as_the_usage_it_was_written_from() {
    for usage in [reported(), bare(), model::Usage::default()] {
        let written = serde_json::to_string(&Usage::from(usage)).unwrap();

        let read = serde_json::from_str::<Usage>(&written).unwrap();

        assert_eq!(model::Usage::from(read), usage);
    }
}

#[test]
fn a_count_left_out_reads_as_zero_for_the_totals_and_as_not_reported_for_their_parts() {
    let nothing = serde_json::from_value::<Usage>(json!({})).unwrap();
    let totals =
        serde_json::from_value::<Usage>(json!({ "input_tokens": 9, "output_tokens": 1 })).unwrap();
    let a_part = serde_json::from_value::<Usage>(json!({
        "input_tokens": 9,
        "output_tokens": 1,
        "cache_read_tokens": 0,
    }))
    .unwrap();

    assert_eq!(model::Usage::from(nothing), model::Usage::default());
    assert_eq!(model::Usage::from(totals), bare());
    assert_eq!(
        model::Usage::from(a_part),
        model::Usage {
            cache_read_tokens: Some(0),
            ..bare()
        }
    );
}

#[test]
fn a_misspelt_count_is_an_error_not_a_count_nobody_reported() {
    for (misspelt, unknown) in [
        (
            json!({ "input_token": 12, "output_tokens": 3 }),
            "input_token",
        ),
        (
            json!({ "input_tokens": 12, "output_tokens": 3, "cache_read_token": 8 }),
            "cache_read_token",
        ),
    ] {
        let refused = serde_json::from_value::<Usage>(misspelt).unwrap_err();

        assert!(
            refused
                .to_string()
                .contains(&format!("unknown field `{unknown}`")),
            "{refused}"
        );
    }
}
