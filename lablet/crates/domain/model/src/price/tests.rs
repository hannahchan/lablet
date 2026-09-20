use serde_json::json;

use super::*;

/// The rates reach the wide event, so a run's document carries them and a
/// document that names a rate no price list could have is refused on the way
/// in, as `Cost` is.
#[test]
fn rates_have_one_json_form_and_a_rate_that_is_not_a_price_is_refused() {
    let rates = Rates::new(3.0, 15.0, 0.3, 3.75).unwrap();
    let json = json!({ "input": 3.0, "output": 15.0, "cache_read": 0.3, "cache_write": 3.75 });

    assert_eq!(serde_json::to_value(rates).unwrap(), json);
    assert_eq!(serde_json::from_value::<Rates>(json).unwrap(), rates);
    assert!(
        serde_json::from_value::<Rates>(
            json!({ "input": -1.0, "output": 15.0, "cache_read": 0.3, "cache_write": 3.75 })
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<Rates>(json!({ "input": 3.0, "output": 15.0 })).is_err(),
        "a rate left out is not zero"
    );
}

#[test]
fn the_first_rate_that_is_not_a_price_is_the_one_reported() {
    for (name, rates) in [
        ("input", Rates::new(f64::NAN, 15.0, 0.3, 3.75)),
        ("output", Rates::new(3.0, -0.01, 0.3, 3.75)),
        ("cache_read", Rates::new(3.0, 15.0, f64::INFINITY, 3.75)),
        ("cache_write", Rates::new(3.0, 15.0, 0.3, -1.0)),
    ] {
        assert_eq!(rates.unwrap_err().name, name);
    }
    assert_eq!(
        Rates::new(-3.0, 15.0, 0.3, 3.75).unwrap_err().to_string(),
        "input rate -3 isn't a finite number of at least 0"
    );
}

/// A locally served model costs nothing, so zero is a price like any other.
#[test]
fn a_rate_of_zero_is_a_price() {
    assert!(Rates::new(0.0, 0.0, 0.0, 0.0).is_ok());
}

fn usd(usd: f64) -> Cost {
    Cost::new(usd).expect("the amounts in these tests are all real costs")
}

#[test]
fn a_cost_is_a_bare_number_of_dollars() {
    let cost = usd(0.0125);

    assert!((cost.usd() - 0.0125).abs() < f64::EPSILON);
    assert_eq!(serde_json::to_value(cost).unwrap(), json!(0.0125));
    assert_eq!(serde_json::from_value::<Cost>(json!(0.0125)).unwrap(), cost);
    assert!(usd(1.0) < usd(2.0));
    assert_eq!(usd(0.0), Cost::new(0.0).unwrap());
}

/// An amount JSON can't write, or one that means nothing, is refused on both
/// paths: serde would otherwise put a `null` where a cost belongs, and `null`
/// is how a run with no pricing configured writes the same field.
#[test]
fn an_amount_that_is_not_a_finite_number_of_dollars_is_no_cost() {
    for amount in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01] {
        assert!(Cost::new(amount).is_err(), "{amount}");
    }
    // JSON has no infinity or NaN, so a negative is the only one a document
    // can hold.
    assert!(serde_json::from_value::<Cost>(json!(-0.01)).is_err());
    assert_eq!(
        Cost::new(-1.5).unwrap_err().to_string(),
        "-1.5 isn't a finite number of US dollars of at least 0"
    );
}
