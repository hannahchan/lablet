//! Reads a config's tree into the config's types, and says where a value
//! was refused.
//!
//! serde's own reader of a tree says what was wrong and not where, and its
//! words hold the value it read. This one knows the path of every value it
//! hands on, so a refusal names its key, and a refusal holds only what was
//! expected: the value is shown from the tree, as it was written.

use std::fmt::{self, Display};

use serde::de::value::StrDeserializer;
use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, Expected, IntoDeserializer, MapAccess,
    SeqAccess, Unexpected, VariantAccess, Visitor,
};
use serde_json::Value;

use super::key::KeyPath;

/// Why a value of the tree was refused, and at which key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fault {
    /// Where the refusal was made. `None` until the reader of the value it
    /// was made in hands it on, since the type that refused doesn't know.
    pub(crate) key: Option<KeyPath>,
    pub(crate) kind: FaultKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FaultKind {
    /// A key that no setting has, beside the keys the section does have.
    UnknownKey {
        name: String,
        known: &'static [&'static str],
    },
    /// A setting that has no default isn't stated.
    Missing { name: &'static str },
    /// A value of the wrong kind, or one the setting doesn't take.
    Refused { reason: String },
}

impl Fault {
    fn new(kind: FaultKind) -> Self {
        Self { key: None, kind }
    }

    fn refused(reason: String) -> Self {
        Self::new(FaultKind::Refused { reason })
    }

    /// The fault, made at `key` when nothing nearer to it said where.
    fn at(mut self, key: &KeyPath) -> Self {
        if self.key.is_none() {
            self.key = Some(match &self.kind {
                FaultKind::UnknownKey { name, .. } => key.key(name),
                FaultKind::Missing { name } => key.key(name),
                FaultKind::Refused { .. } => key.clone(),
            });
        }
        self
    }
}

impl Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            FaultKind::UnknownKey { name, .. } => write!(f, "unknown key {name}"),
            FaultKind::Missing { name } => write!(f, "{name} isn't set"),
            FaultKind::Refused { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for Fault {}

/// The accepted values, as a refusal lists them.
pub(crate) fn listed(values: &[&str]) -> String {
    match values {
        [] => "none".to_owned(),
        [one] => format!("`{one}`"),
        values => values
            .iter()
            .map(|value| format!("`{value}`"))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

impl de::Error for Fault {
    fn custom<T: Display>(message: T) -> Self {
        Self::refused(message.to_string())
    }

    // What was found is the value, which the refusal shows as it was
    // written, so only what was expected is kept.
    fn invalid_type(_: Unexpected<'_>, expected: &dyn Expected) -> Self {
        Self::refused(format!("expected {expected}"))
    }

    fn invalid_value(_: Unexpected<'_>, expected: &dyn Expected) -> Self {
        Self::refused(format!("expected {expected}"))
    }

    fn invalid_length(_: usize, expected: &dyn Expected) -> Self {
        Self::refused(format!("expected {expected}"))
    }

    fn unknown_variant(_: &str, expected: &'static [&'static str]) -> Self {
        Self::refused(format!("the accepted values are {}", listed(expected)))
    }

    fn unknown_field(field: &str, expected: &'static [&'static str]) -> Self {
        Self::new(FaultKind::UnknownKey {
            name: field.to_owned(),
            known: expected,
        })
    }

    fn missing_field(field: &'static str) -> Self {
        Self::new(FaultKind::Missing { name: field })
    }

    fn duplicate_field(_: &'static str) -> Self {
        Self::refused("the key is written twice".to_owned())
    }
}

/// The value at `path` of a tree, as a serde deserialiser.
pub(crate) struct At<'a> {
    pub(crate) value: &'a Value,
    pub(crate) path: KeyPath,
}

impl<'de> Deserializer<'de> for At<'_> {
    type Error = Fault;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        match self.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(value) => visitor.visit_bool(*value),
            Value::Number(number) => match (number.as_u64(), number.as_i64()) {
                (Some(value), _) => visitor.visit_u64(value),
                (None, Some(value)) => visitor.visit_i64(value),
                // A number that's neither is a float, and every float
                // the tree holds is finite.
                (None, None) => visitor.visit_f64(number.as_f64().unwrap_or_default()),
            },
            Value::String(value) => visitor.visit_str(value),
            Value::Array(items) => visitor.visit_seq(Items {
                rest: items.iter().enumerate(),
                path: &self.path,
            }),
            Value::Object(entries) => visitor.visit_map(Entries {
                rest: entries.iter(),
                path: &self.path,
                value: None,
            }),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Fault> {
        match self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Fault> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Fault> {
        let variant = match self.value {
            Value::String(name) => Some((name.as_str(), None)),
            Value::Object(entries) if entries.len() == 1 => entries
                .iter()
                .next()
                .map(|(name, value)| (name.as_str(), Some(value))),
            _ => None,
        };
        let Some((name, value)) = variant else {
            return Err(Fault::refused(
                "expected a value's name, or a mapping of one name to what it holds".to_owned(),
            ));
        };
        visitor.visit_enum(Variant {
            name,
            value,
            path: &self.path,
        })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple tuple_struct map struct
        identifier ignored_any
    }
}

struct Items<'a, I> {
    rest: I,
    path: &'a KeyPath,
}

impl<'de, 'a, I: Iterator<Item = (usize, &'a Value)>> SeqAccess<'de> for Items<'_, I> {
    type Error = Fault;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Fault> {
        let Some((index, value)) = self.rest.next() else {
            return Ok(None);
        };
        let path = self.path.index(index);
        seed.deserialize(At {
            value,
            path: path.clone(),
        })
        .map(Some)
        .map_err(|fault| fault.at(&path))
    }
}

struct Entries<'a, I> {
    rest: I,
    path: &'a KeyPath,
    /// The value of the key last read, with its path.
    value: Option<(&'a Value, KeyPath)>,
}

impl<'de, 'a, I: Iterator<Item = (&'a String, &'a Value)>> MapAccess<'de> for Entries<'a, I> {
    type Error = Fault;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Fault> {
        let Some((key, value)) = self.rest.next() else {
            return Ok(None);
        };
        self.value = Some((value, self.path.key(key)));
        let key: StrDeserializer<'_, Fault> = key.as_str().into_deserializer();
        seed.deserialize(key)
            .map(Some)
            .map_err(|fault| fault.at(self.path))
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Fault> {
        let (value, path) = self
            .value
            .take()
            .ok_or_else(|| Fault::refused("a value was read before its key".to_owned()))?;
        seed.deserialize(At {
            value,
            path: path.clone(),
        })
        .map_err(|fault| fault.at(&path))
    }
}

/// A variant of an enum, by its name, with what it holds when it holds
/// anything.
struct Variant<'a> {
    name: &'a str,
    value: Option<&'a Value>,
    path: &'a KeyPath,
}

impl<'de, 'a> EnumAccess<'de> for Variant<'a> {
    type Error = Fault;
    type Variant = Held<'a>;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Held<'a>), Fault> {
        let name: StrDeserializer<'_, Fault> = self.name.into_deserializer();
        let variant = seed.deserialize(name)?;
        Ok((
            variant,
            Held {
                value: self.value,
                path: self.path.key(self.name),
            },
        ))
    }
}

/// What a variant holds.
struct Held<'a> {
    value: Option<&'a Value>,
    path: KeyPath,
}

impl Held<'_> {
    fn at(&self) -> Result<At<'_>, Fault> {
        match self.value {
            Some(value) => Ok(At {
                value,
                path: self.path.clone(),
            }),
            None => Err(Fault::refused(
                "expected a mapping of the value's name to what it holds".to_owned(),
            )),
        }
    }
}

impl<'de> VariantAccess<'de> for Held<'_> {
    type Error = Fault;

    fn unit_variant(self) -> Result<(), Fault> {
        match self.value {
            None | Some(Value::Null) => Ok(()),
            Some(_) => Err(Fault::refused("expected the value's name alone".to_owned())),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Fault> {
        seed.deserialize(self.at()?)
            .map_err(|fault| fault.at(&self.path))
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Fault> {
        self.at()?
            .deserialize_any(visitor)
            .map_err(|fault| fault.at(&self.path))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Fault> {
        self.at()?
            .deserialize_any(visitor)
            .map_err(|fault| fault.at(&self.path))
    }
}

#[cfg(test)]
mod tests;
