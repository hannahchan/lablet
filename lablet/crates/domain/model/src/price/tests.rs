use super::*;

#[test]
fn rates_hold_each_price_under_its_own_name() {
    assert_eq!(
        Rates::new(3.0, 15.0, 0.3, 3.75),
        Ok(Rates {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
        })
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
    assert!(usd(1.0) < usd(2.0));
    assert_eq!(usd(0.0), Cost::new(0.0).unwrap());
}

/// An amount JSON can't write, or one that means nothing, is refused: a
/// record would otherwise hold a `null` where a cost belongs, and `null` is
/// how a run with no pricing configured writes the same field.
#[test]
fn an_amount_that_is_not_a_finite_number_of_dollars_is_no_cost() {
    for amount in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01] {
        assert!(Cost::new(amount).is_err(), "{amount}");
    }
    assert_eq!(
        Cost::new(-1.5).unwrap_err().to_string(),
        "-1.5 isn't a finite number of US dollars of at least 0"
    );
}
