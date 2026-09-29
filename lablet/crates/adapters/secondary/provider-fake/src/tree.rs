//! A script's text as a tree of JSON values, before anything reads its
//! shapes.
//!
//! Both formats are read into the one tree, so an entry is read, and
//! refused, the same way whichever format its script is in.

use std::fmt;

use serde::de::{Deserialize, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// A tree in which no mapping holds a key twice.
///
/// `serde_json::Value` reads a repeated key as its last value and says
/// nothing, so a response that wrote `finish` twice would be played with
/// the second and look as if it had been written once.
pub(crate) struct Tree(pub(crate) Value);

impl<'de> Deserialize<'de> for Tree {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(Strict).map(Self)
    }
}

struct Strict;

impl<'de> Visitor<'de> for Strict {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a value JSON can hold")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E: Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom(format!("{value} isn't a number JSON can hold")))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(Tree(item)) = sequence.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut mapping: A) -> Result<Value, A::Error> {
        let mut entries = Map::new();
        while let Some(key) = mapping.next_key::<String>()? {
            if entries.contains_key(&key) {
                return Err(A::Error::custom(format!(
                    "the key {key:?} is written twice in one mapping"
                )));
            }
            let Tree(value) = mapping.next_value()?;
            entries.insert(key, value);
        }
        Ok(Value::Object(entries))
    }
}

#[cfg(test)]
mod tests;
