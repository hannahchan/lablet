use opentelemetry::logs::{AnyValue, Logger as _, LoggerProvider as _};
use opentelemetry::{Array, Key, Value};
use opentelemetry_sdk::logs::SdkLoggerProvider;

use super::*;

const TEXT: &str = "lablet.test.text";
const COUNT: &str = "lablet.test.count";

fn text_of(attributes: Attributes) -> String {
    match attributes.into_key_values().remove(0).value {
        Value::String(text) => text.as_str().to_owned(),
        other => panic!("{other:?} isn't text"),
    }
}

#[test]
fn a_text_as_long_as_the_limit_is_kept_whole_and_a_longer_one_is_cut_there() {
    let at_the_limit = "a".repeat(ATTRIBUTE_MAX_BYTES);
    let past_it = "a".repeat(ATTRIBUTE_MAX_BYTES + 1);

    assert_eq!(
        text_of(Attributes::default().with(TEXT, at_the_limit.clone())),
        at_the_limit
    );
    assert_eq!(
        text_of(Attributes::default().with(TEXT, past_it)),
        at_the_limit
    );
}

#[test]
fn a_text_is_cut_at_a_character_boundary() {
    // Three bytes to a character, so the limit falls inside one.
    let text = "€".repeat(ATTRIBUTE_MAX_BYTES / 3 + 1);

    let kept = text_of(Attributes::default().with(TEXT, text));

    assert_eq!(kept.len(), ATTRIBUTE_MAX_BYTES - ATTRIBUTE_MAX_BYTES % 3);
    assert!(kept.chars().all(|character| character == '€'));
}

#[test]
fn each_text_of_a_list_is_cut_on_its_own() {
    let texts = vec!["b".repeat(ATTRIBUTE_MAX_BYTES + 7), "short".to_owned()];

    let mut held = Attributes::default().with(TEXT, texts).into_key_values();

    let Value::Array(Array::String(kept)) = held.remove(0).value else {
        panic!("a list of texts is an array of strings");
    };
    let lengths: Vec<_> = kept.iter().map(|text| text.as_str().len()).collect();
    assert_eq!(lengths, [ATTRIBUTE_MAX_BYTES, 5]);
}

#[test]
fn a_count_the_wire_cannot_say_is_the_largest_it_can() {
    let held = Attributes::default()
        .with(COUNT, u64::MAX)
        .with(COUNT, 7_u64)
        .into_key_values();

    let values: Vec<_> = held.into_iter().map(|held| held.value).collect();
    assert_eq!(values, [Value::I64(i64::MAX), Value::I64(7)]);
}

#[test]
fn an_attribute_without_a_value_is_left_out() {
    let held = Attributes::default()
        .with_any(TEXT, None::<String>)
        .with_any(COUNT, Some(3_u32))
        .into_key_values();

    let keys: Vec<_> = held.iter().map(|held| held.key.as_str()).collect();
    assert_eq!(keys, [COUNT]);
}

#[test]
fn a_span_and_a_log_record_are_given_the_same_values() {
    let attributes = || {
        Attributes::default()
            .with(TEXT, "chat")
            .with(COUNT, 2_u16)
            .and(
                Attributes::default()
                    .with("lablet.test.ratio", 0.5)
                    .with("lablet.test.flag", true)
                    .with("lablet.test.seed", -4_i64)
                    .with("lablet.test.names", vec!["bash".to_owned()]),
            )
    };
    let provider = SdkLoggerProvider::builder().build();
    let mut record = provider.logger("test").create_log_record();

    attributes().onto(&mut record);

    let on_the_span: Vec<_> = attributes()
        .into_key_values()
        .into_iter()
        .map(|held| (held.key, held.value))
        .collect();
    assert_eq!(
        on_the_span,
        [
            (Key::new(TEXT), Value::from("chat")),
            (Key::new(COUNT), Value::I64(2)),
            (Key::new("lablet.test.ratio"), Value::F64(0.5)),
            (Key::new("lablet.test.flag"), Value::Bool(true)),
            (Key::new("lablet.test.seed"), Value::I64(-4)),
            (
                Key::new("lablet.test.names"),
                Value::Array(Array::String(vec!["bash".into()]))
            ),
        ]
    );
    let on_the_record: Vec<_> = record.attributes_iter().cloned().collect();
    assert_eq!(
        on_the_record,
        [
            (Key::new(TEXT), AnyValue::from("chat")),
            (Key::new(COUNT), AnyValue::Int(2)),
            (Key::new("lablet.test.ratio"), AnyValue::Double(0.5)),
            (Key::new("lablet.test.flag"), AnyValue::Boolean(true)),
            (Key::new("lablet.test.seed"), AnyValue::Int(-4)),
            (
                Key::new("lablet.test.names"),
                AnyValue::ListAny(Box::new(vec![AnyValue::from("bash")]))
            ),
        ]
    );
}
